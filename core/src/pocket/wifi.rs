//! Copying a recording over the Pocket's WiFi (docs/pocket.md, WiFi), about
//! twenty times faster than Bluetooth. The Pocket raises an access point of
//! its own and sends the MP3 as raw bytes on a TCP socket; Bluetooth still
//! drives it. So the Mac's Wi-Fi joins the Pocket's network for the copy and
//! goes back to its own network afterwards (turned off and on, macOS rejoins
//! the network it knows), whatever happened.
//!
//! The steps, from pocket-libre's captures of the vendor app (firmware 1.8):
//! `WIFIC` (a fresh module: its socket takes one or two connections a
//! boot), `WIFIO`, `WIFI` for the network's name and password, `WIFIS`
//! until it is up, the Mac joins, `WIFIS` until it says a client joined,
//! the socket opened, `U&<date>&<name>` for the size, `U&WIFI` to send it
//! to the socket, the bytes, `WIFIC`.

use std::fmt;
use std::process::Command;
use std::time::{Duration, Instant};

use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

use super::link::{Link, LinkError};
use super::protocol::{Message, Recording, command};

/// The Pocket's own address on its network, and the socket's port.
const POCKET: (&str, u16) = ("192.168.200.1", 8475);
/// The Mac's address there starts with this.
const SUBNET: &str = "192.168.200.";
/// A socket that stops sending for this long has stalled.
const STALL: Duration = Duration::from_secs(15);

#[derive(Debug)]
pub enum WifiError {
    /// This Pocket or this Mac can't: Bluetooth it is, for the whole run.
    Unsupported(String),
    /// This time it didn't work: Bluetooth for this connection.
    Failed(String),
    /// The Bluetooth link itself broke.
    Link(LinkError),
}

impl fmt::Display for WifiError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            WifiError::Unsupported(why) | WifiError::Failed(why) => write!(f, "{why}"),
            WifiError::Link(e) => write!(f, "{e}"),
        }
    }
}

/// A Bluetooth error while setting WiFi up: a broken link ends the sync,
/// anything else is this attempt failing.
fn on_link(what: &str) -> impl Fn(LinkError) -> WifiError + '_ {
    move |e| match e {
        LinkError::Closed => WifiError::Link(LinkError::Closed),
        e => WifiError::Failed(format!("{what}: {e}")),
    }
}

/// Download `recording` over WiFi. `key` is the session key, whose first
/// eight characters are the network's password when the Pocket doesn't
/// say it. `progress` hears the bytes so far and the total.
pub async fn download(link: &mut Link, recording: &Recording, key: &str, mut progress: impl FnMut(u64, u64)) -> Result<Vec<u8>, WifiError> {
    let mut mac = Mac::take().await?;
    let result = copy(link, &mut mac, recording, key, &mut progress).await;
    // Whatever happened: the Pocket's WiFi off (it costs its battery), the
    // Mac back on its own network.
    let _ = link.request(command::WIFI_OFF, |m| *m == Message::WifiOff, Duration::from_secs(5)).await;
    mac.give_back().await;
    result
}

async fn copy(link: &mut Link, mac: &mut Mac, recording: &Recording, key: &str, progress: &mut impl FnMut(u64, u64)) -> Result<Vec<u8>, WifiError> {
    let _ = link.request(command::WIFI_OFF, |m| *m == Message::WifiOff, Duration::from_secs(5)).await;
    match link.request(command::WIFI_ON, |m| *m == Message::WifiOn, Duration::from_secs(10)).await {
        Ok(_) => {}
        Err(LinkError::Timeout) => return Err(WifiError::Unsupported("the Pocket didn't turn its WiFi on".into())),
        Err(e) => return Err(on_link("turning WiFi on")(e)),
    }
    let (ssid, password) = match link.request(command::WIFI_ACCESS, |m| matches!(m, Message::WifiAccess { .. }), Duration::from_secs(5)).await {
        Ok(Message::WifiAccess { ssid, password }) => (ssid, password),
        Ok(_) => unreachable!(),
        Err(e) => return Err(on_link("the WiFi's name")(e)),
    };
    let password = if password.len() >= 8 { password } else { key.chars().take(8).collect() };
    wait_state(link, |n| n == 1 || n == 2, Duration::from_secs(25)).await?;
    mac.join(&ssid, &password).await?;
    wait_state(link, |n| n == 1, Duration::from_secs(20)).await?;
    let mut socket = connect(Duration::from_secs(15)).await?;

    let size = match link.request(&command::upload(&recording.date, &recording.name), |m| matches!(m, Message::Upload(_)), Duration::from_secs(10)).await {
        Ok(Message::Upload(size)) => size,
        Ok(_) => unreachable!(),
        Err(e) => return Err(on_link("the file's size")(e)),
    };
    // As the vendor app: a moment after the size, the switch to WiFi. Never
    // without the socket open, or the Pocket hangs and shuts down.
    tokio::time::sleep(Duration::from_millis(300)).await;
    link.request(command::WIFI_UPLOAD, |m| *m == Message::UploadWifi, Duration::from_secs(5)).await.map_err(on_link("the switch to WiFi"))?;

    let mut bytes = Vec::with_capacity(size as usize);
    let mut buffer = vec![0u8; 64 * 1024];
    let mut shown = 0usize;
    while (bytes.len() as u64) < size {
        let n = match tokio::time::timeout(STALL, socket.read(&mut buffer)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(WifiError::Failed(format!("the WiFi socket: {e}"))),
            Err(_) => return Err(WifiError::Failed("the WiFi copy stalled".into())),
        };
        bytes.extend_from_slice(&buffer[..n]);
        if bytes.len() - shown >= 64 * 1024 {
            shown = bytes.len();
            progress((bytes.len() as u64).min(size), size);
        }
    }
    // Ten fixed bytes follow the file; they aren't the recording's.
    if (bytes.len() as u64) < size {
        return Err(WifiError::Failed(format!("only {} of {size} bytes came over WiFi", bytes.len())));
    }
    bytes.truncate(size as usize);
    progress(size, size);
    Ok(bytes)
}

/// Ask the Pocket's WiFi state until `ready` likes it.
async fn wait_state(link: &mut Link, ready: impl Fn(u8) -> bool, within: Duration) -> Result<(), WifiError> {
    let deadline = Instant::now() + within;
    loop {
        match link.request(command::WIFI_STATE, |m| matches!(m, Message::WifiState(_)), Duration::from_secs(3)).await {
            Ok(Message::WifiState(n)) if ready(n) => return Ok(()),
            Ok(_) | Err(LinkError::Timeout) => {}
            Err(e) => return Err(on_link("the WiFi's state")(e)),
        }
        if Instant::now() >= deadline {
            return Err(WifiError::Failed("the Pocket's WiFi didn't get ready".into()));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// The Pocket's socket, tried again while it refuses (it does for a few
/// seconds after a connection closes).
async fn connect(within: Duration) -> Result<TcpStream, WifiError> {
    let deadline = Instant::now() + within;
    loop {
        match tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(POCKET)).await {
            Ok(Ok(socket)) => return Ok(socket),
            Ok(Err(_)) | Err(_) if Instant::now() < deadline => tokio::time::sleep(Duration::from_secs(1)).await,
            Ok(Err(e)) => return Err(WifiError::Failed(format!("the Pocket's WiFi socket: {e}"))),
            Err(_) => return Err(WifiError::Failed("the Pocket's WiFi socket didn't answer".into())),
        }
    }
}

/// The Mac's Wi-Fi, borrowed for one copy.
struct Mac {
    /// Its interface (`en0`).
    interface: String,
    /// It was on before: turned off and on afterwards, it rejoins its network.
    was_on: bool,
    /// The Pocket's network, once joined (or tried).
    joined: Option<String>,
}

impl Mac {
    async fn take() -> Result<Mac, WifiError> {
        if !cfg!(target_os = "macos") {
            return Err(WifiError::Unsupported("WiFi copies need macOS".into()));
        }
        let (interface, was_on) = blocking(|| {
            let ports = run("networksetup", &["-listallhardwareports"])?;
            let interface = wifi_interface(&ports).ok_or("this Mac has no Wi-Fi")?;
            let power = run("networksetup", &["-getairportpower", &interface])?;
            Ok((interface, power.trim_end().ends_with("On")))
        })
        .await
        .map_err(WifiError::Unsupported)?;
        Ok(Mac { interface, was_on, joined: None })
    }

    /// Join the Pocket's network and wait for an address on it.
    async fn join(&mut self, ssid: &str, password: &str) -> Result<(), WifiError> {
        self.joined = Some(ssid.to_string());
        let (interface, was_on, ssid, password) = (self.interface.clone(), self.was_on, ssid.to_string(), password.to_string());
        blocking(move || {
            if !was_on {
                run("networksetup", &["-setairportpower", &interface, "on"])?;
                std::thread::sleep(Duration::from_secs(3));
            }
            // Its network is hidden: macOS may not see it at the first try.
            let mut last = String::new();
            for _ in 0..4 {
                let said = run("networksetup", &["-setairportnetwork", &interface, &ssid, &password])?;
                if !said.contains("Could not") && !said.contains("Error") && !said.contains("Failed") {
                    last.clear();
                    break;
                }
                last = said.trim().to_string();
                std::thread::sleep(Duration::from_secs(2));
            }
            if !last.is_empty() {
                return Err(format!("the Mac couldn't join the Pocket's WiFi: {last}"));
            }
            let deadline = Instant::now() + Duration::from_secs(20);
            while Instant::now() < deadline {
                if run("ipconfig", &["getifaddr", &interface]).is_ok_and(|ip| ip.trim().starts_with(SUBNET)) {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            Err("the Mac got no address on the Pocket's WiFi".into())
        })
        .await
        .map_err(WifiError::Failed)
    }

    /// Back as it was: the Pocket's network forgotten, and Wi-Fi turned off
    /// and on (macOS then joins the network it prefers), or left off.
    async fn give_back(&mut self) {
        let Some(ssid) = self.joined.take() else { return };
        let (interface, was_on) = (self.interface.clone(), self.was_on);
        let _ = blocking(move || {
            // Without an administrator this may be refused: harmless, the
            // network exists only while Yoru copies.
            let _ = run("networksetup", &["-removepreferredwirelessnetwork", &interface, &ssid]);
            run("networksetup", &["-setairportpower", &interface, "off"])?;
            if was_on {
                std::thread::sleep(Duration::from_secs(1));
                run("networksetup", &["-setairportpower", &interface, "on"])?;
            }
            Ok(())
        })
        .await
        .inspect_err(|e| crate::hostlog::line("pocket", &format!("giving Wi-Fi back: {e}")));
    }
}

/// The Wi-Fi port's device in `networksetup -listallhardwareports`.
fn wifi_interface(ports: &str) -> Option<String> {
    let mut lines = ports.lines();
    while let Some(line) = lines.next() {
        if matches!(line.trim(), "Hardware Port: Wi-Fi" | "Hardware Port: AirPort") {
            return lines.next()?.trim().strip_prefix("Device: ").map(str::to_string);
        }
    }
    None
}

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program).args(args).output().map_err(|e| format!("{program}: {e}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if out.status.success() { Ok(text) } else { Err(format!("{program}: {}", text.trim())) }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_wifi_interface() {
        let ports = "\nHardware Port: Ethernet\nDevice: en1\nEthernet Address: x\n\nHardware Port: Wi-Fi\nDevice: en0\nEthernet Address: y\n";
        assert_eq!(wifi_interface(ports).as_deref(), Some("en0"));
        assert_eq!(wifi_interface("Hardware Port: Thunderbolt Bridge\nDevice: bridge0\n"), None);
    }
}
