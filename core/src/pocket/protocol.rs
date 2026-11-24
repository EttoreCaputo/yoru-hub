//! The Pocket's words over Bluetooth (docs/pocket.md): ASCII commands
//! `APP&…` written to one characteristic, answers and events `MCU&…`
//! notified on the same one. Pure parsing, so it is tested without a device.

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use serde::Serialize;

/// What the device said, one message at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// `MCU&SK&OK` or `MCU&SK&ERR`: the session key taken or refused.
    Key(bool),
    Battery(u8),
    Firmware(String),
    WifiFirmware(String),
    /// Free and total storage, in MB.
    Space(u32, u32),
    /// 0 idle, 1 recording, 2 in a call.
    State(u8),
    Usb(bool),
    Mac(String),
    /// `MCU&T&OK`: the clock was set.
    Clock,
    /// A date folder (`MCU&DIRS&2026-11-19`).
    Dir(String),
    /// The end of the date folders, with their count.
    DirsEnd(u32),
    /// A recording in a listing.
    File(Recording),
    /// The end of a listing, with its count.
    ListEnd(u32),
    /// A download starts: the file's exact size in bytes.
    Upload(u64),
    /// The end of a transfer.
    Off,
    /// A recording started, with the name it is listed under.
    Started(String),
    /// The recording stopped.
    Stopped,
    /// `MCU&D`: a deletion was done.
    Deleted,
    /// `MCU&WIFIO`: its WiFi access point is coming up.
    WifiOn,
    /// `MCU&WIFIC`: its WiFi is off.
    WifiOff,
    /// `MCU&WIFIS&<n>`: 0 off, 3 coming up, 2 up and waiting, 1 a client joined.
    WifiState(u8),
    /// `MCU&WIFI&<ssid>&<password>`: how to join its access point.
    WifiAccess { ssid: String, password: String },
    /// `MCU&U&WIFI`: the download goes to the WiFi socket.
    UploadWifi,
    /// Anything else, kept as it came.
    Other(String),
}

/// A recording on the device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Recording {
    /// Its date folder (`2026-11-19`).
    pub date: String,
    /// Its name, without `.mp3`: the time it started (`20261119143012`),
    /// or `PH` and a shorter time for some.
    pub name: String,
    /// Its length in seconds (the listing's last field).
    pub seconds: u32,
}

/// Bytes per second of the device's MP3 (32 kbps): a recording's size from
/// its length.
pub const BYTES_PER_SECOND: u64 = 4000;

impl Recording {
    pub fn estimated_bytes(&self) -> u64 {
        u64::from(self.seconds) * BYTES_PER_SECOND
    }

    /// When it started, from its name, which the device's clock gives:
    /// UTC, since Yoru (like the vendor app) sets it in UTC.
    pub fn started_at(&self) -> Option<DateTime<Utc>> {
        let (digits, format) = match self.name.strip_prefix("PH") {
            Some(rest) => (rest, "%y%m%d%H%M%S"),
            None => (self.name.as_str(), "%Y%m%d%H%M%S"),
        };
        NaiveDateTime::parse_from_str(digits, format).ok().map(|t| Utc.from_utc_datetime(&t))
    }
}

/// The messages in one notification: several can come back to back
/// (`MCU&WIFIOMCU&OFF`), padded with NULs.
pub fn split(bytes: &[u8]) -> Vec<String> {
    let text: String = String::from_utf8_lossy(bytes).chars().filter(|&c| c != '\0').collect();
    let mut out = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        let next = rest[1..].find("MCU&").map_or(rest.len(), |i| i + 1);
        let part = rest[..next].trim();
        if !part.is_empty() {
            out.push(part.to_string());
        }
        rest = rest[next..].trim_start();
    }
    out
}

/// One message.
pub fn parse(text: &str) -> Message {
    let Some(body) = text.strip_prefix("MCU&") else { return Message::Other(text.into()) };
    let fields: Vec<&str> = body.split('&').collect();
    let num = |i: usize| fields.get(i).and_then(|f| f.trim().parse::<u64>().ok());
    let other = || Message::Other(text.into());
    match fields.as_slice() {
        ["SK", "OK", ..] => Message::Key(true),
        ["SK", "ERR", ..] => Message::Key(false),
        ["BAT", ..] => num(1).map_or_else(other, |n| Message::Battery(n.min(100) as u8)),
        ["FW", v, ..] => Message::Firmware(v.to_string()),
        ["WF", v, ..] => Message::WifiFirmware(v.to_string()),
        ["SPA", ..] => match (num(1), num(2)) {
            (Some(free), Some(total)) => Message::Space(free as u32, total as u32),
            _ => other(),
        },
        ["STE", ..] => num(1).map_or_else(other, |n| Message::State(n as u8)),
        ["USB", ..] => num(1).map_or_else(other, |n| Message::Usb(n == 1)),
        ["MAC", v, ..] => Message::Mac(v.to_string()),
        ["T", "OK", ..] => Message::Clock,
        ["DIRS_SUM", ..] => num(1).map_or_else(other, |n| Message::DirsEnd(n as u32)),
        ["DIRS", d, ..] => Message::Dir(d.to_string()),
        ["F", date, name, ..] => match num(3) {
            Some(seconds) => Message::File(Recording { date: date.to_string(), name: name.to_string(), seconds: seconds as u32 }),
            None => other(),
        },
        ["LIST", ..] => num(1).map_or_else(other, |n| Message::ListEnd(n as u32)),
        ["U", "WIFI", ..] => Message::UploadWifi,
        ["WIFIO", ..] => Message::WifiOn,
        ["WIFIC", ..] => Message::WifiOff,
        ["WIFIS", ..] => num(1).map_or_else(other, |n| Message::WifiState(n as u8)),
        ["WIFI", ssid, password, ..] if !ssid.is_empty() => Message::WifiAccess { ssid: ssid.to_string(), password: password.to_string() },
        ["U", ..] => num(1).map_or_else(other, Message::Upload),
        ["OFF", ..] => Message::Off,
        ["STA", name, ..] => Message::Started(name.to_string()),
        ["STO", ..] => Message::Stopped,
        ["D", ..] => Message::Deleted,
        _ => other(),
    }
}

/// A name the device gave, safe as a file name and as a command field.
pub fn safe_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// The commands, so their spelling lives in one place.
pub mod command {
    pub fn key(key: &str) -> String {
        format!("APP&SK&{key}")
    }
    pub const BATTERY: &str = "APP&BAT";
    pub const FIRMWARE: &str = "APP&FW";
    pub const WIFI_FIRMWARE: &str = "APP&WF";
    pub const SPACE: &str = "APP&SPACE";
    pub const STATE: &str = "APP&STE";
    pub const USB: &str = "APP&GET&USB";
    pub const MAC: &str = "APP&MAC";
    pub const DIRS: &str = "APP&LIST_DIRS";
    pub const START: &str = "APP&STA";
    pub const STOP: &str = "APP&STO";
    /// Raise the WiFi access point, ask how to join it, how it stands,
    /// send the download there, and close it.
    pub const WIFI_ON: &str = "APP&WIFIO";
    pub const WIFI_ACCESS: &str = "APP&WIFI";
    pub const WIFI_STATE: &str = "APP&WIFIS";
    pub const WIFI_UPLOAD: &str = "APP&U&WIFI";
    pub const WIFI_OFF: &str = "APP&WIFIC";
    pub fn clock(now: chrono::DateTime<chrono::Utc>) -> String {
        format!("APP&T&{}", now.format("%Y%m%d%H%M%S"))
    }
    pub fn list(date: &str) -> String {
        format!("APP&LIST&{date}")
    }
    pub fn upload(date: &str, name: &str) -> String {
        format!("APP&U&{date}&{name}")
    }
    pub fn delete(date: &str, name: &str) -> String {
        format!("APP&D&{date}&{name}")
    }
}

/// A new session key: 16 hex digits, as the vendor app's.
pub fn new_key() -> String {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("the system's random numbers");
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_messages_glued_together() {
        assert_eq!(split(b"MCU&WIFIOMCU&OFF\0\0"), vec!["MCU&WIFIO", "MCU&OFF"]);
        assert_eq!(split(b"MCU&BAT&98"), vec!["MCU&BAT&98"]);
        assert!(split(b"\0\0").is_empty());
    }

    #[test]
    fn parses_what_the_device_said() {
        assert_eq!(parse("MCU&SK&OK"), Message::Key(true));
        assert_eq!(parse("MCU&SK&ERR"), Message::Key(false));
        assert_eq!(parse("MCU&BAT&98"), Message::Battery(98));
        assert_eq!(parse("MCU&FW&1.8.9"), Message::Firmware("1.8.9".into()));
        assert_eq!(parse("MCU&WF&V12"), Message::WifiFirmware("V12".into()));
        assert_eq!(parse("MCU&SPA&059635&059636"), Message::Space(59635, 59636));
        assert_eq!(parse("MCU&STE&0"), Message::State(0));
        assert_eq!(parse("MCU&USB&1"), Message::Usb(true));
        assert_eq!(parse("MCU&T&OK"), Message::Clock);
        assert_eq!(parse("MCU&DIRS&2026-03-26"), Message::Dir("2026-03-26".into()));
        assert_eq!(parse("MCU&DIRS_SUM&003"), Message::DirsEnd(3));
        assert_eq!(parse("MCU&LIST&020"), Message::ListEnd(20));
        assert_eq!(
            parse("MCU&F&2026-03-28&20260328001919&6222"),
            Message::File(Recording { date: "2026-03-28".into(), name: "20260328001919".into(), seconds: 6222 })
        );
        assert_eq!(parse("MCU&U&167798"), Message::Upload(167798));
        assert_eq!(parse("MCU&U&167798&0"), Message::Upload(167798));
        assert_eq!(parse("MCU&U&WIFI"), Message::UploadWifi);
        assert_eq!(parse("MCU&WIFIO"), Message::WifiOn);
        assert_eq!(parse("MCU&WIFIC"), Message::WifiOff);
        assert_eq!(parse("MCU&WIFIS&2"), Message::WifiState(2));
        assert_eq!(
            parse("MCU&WIFI&PKT01_GREY_26425893&AB12CD34"),
            Message::WifiAccess { ssid: "PKT01_GREY_26425893".into(), password: "AB12CD34".into() }
        );
        assert!(matches!(parse("MCU&WIFI&&"), Message::Other(_)));
        assert_eq!(parse("MCU&OFF"), Message::Off);
        assert_eq!(parse("MCU&STA&20261009032636"), Message::Started("20261009032636".into()));
        assert_eq!(parse("MCU&STO"), Message::Stopped);
        assert!(matches!(parse("MCU&WIFIE&2&0&NOT_FOUND"), Message::Other(_)));
        assert!(matches!(parse("hello"), Message::Other(_)));
    }

    #[test]
    fn a_recording_knows_when_it_started() {
        let r = |name: &str| Recording { date: "2026-10-02".into(), name: name.into(), seconds: 25 };
        assert_eq!(r("20261002211958").started_at().unwrap().to_rfc3339(), "2026-10-02T21:19:58+00:00");
        assert_eq!(r("PH261002211958").started_at().unwrap().to_rfc3339(), "2026-10-02T21:19:58+00:00");
        assert_eq!(r("garbage").started_at(), None);
        assert_eq!(r("x").estimated_bytes(), 100_000);
    }

    #[test]
    fn names_are_checked_before_use() {
        assert!(safe_name("20261002211958"));
        assert!(safe_name("2026-10-02"));
        assert!(!safe_name("../etc"));
        assert!(!safe_name("a&b"));
        assert!(!safe_name(""));
    }

    #[test]
    fn keys_are_sixteen_hex_digits() {
        let key = new_key();
        assert_eq!(key.len(), 16);
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(command::key("AB"), "APP&SK&AB");
        let when = Utc.with_ymd_and_hms(2026, 11, 19, 21, 5, 9).unwrap();
        assert_eq!(command::clock(when), "APP&T&20261119210509");
    }
}
