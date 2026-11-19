//! The Pocket, a clip-on voice recorder (Open Vision Engineering's PKT01),
//! as a source of meetings (docs/pocket.md). Yoru Hub talks to it over
//! Bluetooth with its own session key, in place of the vendor's phone app:
//! it reads the battery and the storage, sets the clock, lists the
//! recordings, downloads the new ones and imports each as a meeting, which
//! the meetings' own pipeline then transcribes and summarizes.
//!
//! [`Pocket::start`] runs it on a thread of its own: it looks for the
//! device while it is on, stays connected while it is near (the device
//! takes one connection at a time and stops advertising during it), and
//! fetches a recording as soon as the device says it stopped. Bluetooth is
//! touched only once a key exists or pairing was asked, so the system's
//! permission prompt comes when the user sets the Pocket up.

mod link;
pub mod protocol;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::connectors::{Keychain, SecretStore};
use crate::meetings::{self, Meeting};
use link::{Bluetooth, Link, LinkError};
use protocol::{Message, Recording, command};

/// The keychain account of the session key, in Yoru's one vault item.
pub const KEY_ACCOUNT: &str = "pocket.session-key";

/// Where a pairing made before Yoru's (by hand, with `security`) left its key.
const LEGACY_SERVICE: &str = "pocket-ble-session-key";

/// How long a scan looks before resting.
const SCAN_FOR: Duration = Duration::from_secs(20);
/// The rest between scans while nothing is found.
const SCAN_REST: Duration = Duration::from_secs(20);
/// Battery and storage are read again this often while connected.
const REFRESH_EVERY: Duration = Duration::from_secs(5 * 60);
/// Long enough to mean "until something happens".
const FOREVER: Duration = Duration::from_secs(365 * 24 * 3600);

/// What the user decides (`pocket/settings.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Look for the Pocket and stay connected to it.
    pub enabled: bool,
    /// Download new recordings on their own; otherwise only when asked.
    pub auto_sync: bool,
    /// Delete a recording from the Pocket once it is safely a meeting.
    pub delete_after_sync: bool,
    /// Set the Pocket's clock at each connection, so its recordings are
    /// named and dated right.
    pub set_clock: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { enabled: true, auto_sync: true, delete_after_sync: false, set_clock: true }
    }
}

/// Where the link stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Turned off in the settings.
    Off,
    /// No session key yet: the Pocket must be paired.
    Unpaired,
    /// Waiting for a Pocket fresh from a reset, to give it a key.
    Pairing,
    Searching,
    Connecting,
    Connected,
    /// The Pocket answered with another key: it was paired by someone else.
    Refused,
    /// Bluetooth is off, or Yoru may not use it.
    NoBluetooth,
}

/// A recording on the Pocket, and what Yoru made of it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeviceRecording {
    #[serde(flatten)]
    pub recording: Recording,
    /// Unix milliseconds, from its name.
    pub started_at: Option<i64>,
    /// The meeting it became, once downloaded.
    pub meeting: Option<String>,
}

/// A download under way.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Transfer {
    pub name: String,
    pub bytes: u64,
    pub total: u64,
    /// Recordings still waiting after this one.
    pub queued: usize,
}

/// Everything the app shows about the Pocket.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Status {
    pub phase: Phase,
    pub settings: Settings,
    /// Its Bluetooth name (`PKT01_GREY_26425893`), once seen.
    pub name: Option<String>,
    pub battery: Option<u8>,
    pub firmware: Option<String>,
    pub wifi_firmware: Option<String>,
    /// Free and total storage, in MB.
    pub free_mb: Option<u32>,
    pub total_mb: Option<u32>,
    /// It is recording now.
    pub recording: bool,
    /// The recordings on it, newest first, from the last listing.
    pub recordings: Vec<DeviceRecording>,
    /// It is listing its recordings now.
    pub checking: bool,
    pub transfer: Option<Transfer>,
    /// When the last listing ended (Unix milliseconds).
    pub synced_at: Option<i64>,
    /// The last thing that went wrong, in plain words.
    pub error: Option<String>,
}

impl Status {
    fn new(settings: Settings) -> Self {
        Status {
            phase: Phase::Off,
            settings,
            name: known().name,
            battery: None,
            firmware: None,
            wifi_firmware: None,
            free_mb: None,
            total_mb: None,
            recording: false,
            recordings: Vec::new(),
            checking: false,
            transfer: None,
            synced_at: None,
            error: None,
        }
    }
}

/// What the app hears.
#[derive(Debug, Clone)]
pub enum Event {
    Status(Box<Status>),
    /// A recording became this meeting.
    Imported(Box<Meeting>),
}

/// What the app asks.
#[derive(Debug, Clone)]
pub enum Command {
    /// List and download what is new now.
    Sync,
    /// Download these recordings (by name), even when auto sync is off.
    Download(Vec<String>),
    /// Delete a recording (by name) from the Pocket.
    Delete(String),
    /// Give a Pocket fresh from a reset a new key.
    Pair,
    /// Stop waiting for a Pocket to pair.
    CancelPair,
    /// Forget the key (the Pocket stays paired to it until reset).
    Forget,
    Settings(Settings),
}

/// The Pocket's thread, and what it last knew.
pub struct Pocket {
    commands: mpsc::UnboundedSender<Command>,
    status: Arc<Mutex<Status>>,
}

impl Pocket {
    pub fn start(on_event: impl Fn(Event) + Send + Sync + 'static) -> io::Result<Pocket> {
        let (tx, rx) = mpsc::unbounded_channel();
        let status = Arc::new(Mutex::new(Status::new(settings())));
        let worker = Worker {
            status: status.clone(),
            on_event: Box::new(on_event),
            commands: rx,
            keychain: Keychain::default(),
            bluetooth: None,
            pairing: false,
            failed: BTreeSet::new(),
        };
        std::thread::Builder::new().name("pocket".into()).spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a tokio runtime");
            runtime.block_on(worker.run());
        })?;
        Ok(Pocket { commands: tx, status })
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

/// The settings as saved, or the defaults.
pub fn settings() -> Settings {
    read_json("settings.json")
}

/// `pocket/` in Yoru's data folder.
fn dir() -> io::Result<PathBuf> {
    let dir = crate::paths::data_dir().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data folder"))?.join("pocket");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(name: &str) -> T {
    dir().ok().and_then(|d| fs::read(d.join(name)).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn write_json(name: &str, value: &impl Serialize) -> io::Result<()> {
    let dir = dir()?;
    let part = dir.join(format!("{name}.part"));
    fs::write(&part, serde_json::to_vec_pretty(value)?)?;
    fs::rename(part, dir.join(name))
}

/// The recordings already made meetings: name → meeting id
/// (`pocket/synced.json`). Kept after a meeting is deleted, so a recording
/// still on the Pocket doesn't come back.
fn synced() -> BTreeMap<String, String> {
    read_json("synced.json")
}

fn mark_synced(name: &str, meeting: &str) -> io::Result<()> {
    let mut all = synced();
    all.insert(name.to_string(), meeting.to_string());
    write_json("synced.json", &all)
}

/// The Pocket last connected (`pocket/device.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Known {
    name: Option<String>,
}

fn known() -> Known {
    read_json("device.json")
}

/// Why a connection ended.
enum End {
    /// The device went away, or the link broke: look again.
    Lost(String),
    /// Something the app asked needs the loop (pair, forget, settings).
    Restart,
    /// The device has another key.
    Refused,
    /// A fresh device took the key and dropped the link, as it does.
    Paired,
}

/// Which recordings a sync downloads.
enum Pick {
    None,
    /// Those not downloaded yet.
    New,
    These(Vec<String>),
}

struct Worker {
    status: Arc<Mutex<Status>>,
    on_event: Box<dyn Fn(Event) + Send + Sync>,
    commands: mpsc::UnboundedReceiver<Command>,
    keychain: Keychain,
    bluetooth: Option<Bluetooth>,
    /// The user asked to pair: the next Pocket found gets a new key.
    pairing: bool,
    /// Recordings that failed to download in this run: not tried again on
    /// their own until the user asks.
    failed: BTreeSet<String>,
}

impl Worker {
    fn update(&self, f: impl FnOnce(&mut Status)) {
        let snapshot = {
            let mut status = self.status.lock().unwrap();
            f(&mut status);
            status.clone()
        };
        (self.on_event)(Event::Status(Box::new(snapshot)));
    }

    fn settings(&self) -> Settings {
        self.status.lock().unwrap().settings.clone()
    }

    fn key(&self) -> Option<String> {
        match self.keychain.get(KEY_ACCOUNT) {
            Ok(Some(key)) if valid_key(&key) => Some(key),
            Ok(_) => self.import_key(),
            Err(e) => {
                crate::hostlog::line("pocket", &format!("keychain: {e}"));
                None
            }
        }
    }

    /// A key a pairing made before Yoru did it (the `pocket-ble-session-key`
    /// item `security` keeps), moved into Yoru's vault once.
    fn import_key(&self) -> Option<String> {
        if !cfg!(target_os = "macos") {
            return None;
        }
        let out = std::process::Command::new("security").args(["find-generic-password", "-s", LEGACY_SERVICE, "-w"]).output().ok()?;
        let key = String::from_utf8(out.stdout).ok()?.trim().to_uppercase();
        if !out.status.success() || !valid_key(&key) {
            return None;
        }
        match self.keychain.set(KEY_ACCOUNT, &key) {
            Ok(()) => crate::hostlog::line("pocket", "key imported from an earlier pairing"),
            Err(e) => crate::hostlog::line("pocket", &format!("keychain: {e}")),
        }
        Some(key)
    }

    async fn run(mut self) {
        loop {
            if !self.settings().enabled {
                self.update(|s| s.phase = Phase::Off);
                self.wait(FOREVER).await;
                continue;
            }
            let key = self.key();
            if key.is_none() && !self.pairing {
                self.update(|s| s.phase = Phase::Unpaired);
                self.wait(FOREVER).await;
                continue;
            }
            let fresh = key.is_none();
            let key = key.unwrap_or_else(protocol::new_key);
            self.update(|s| s.phase = if fresh { Phase::Pairing } else { Phase::Searching });
            let found = match self.find().await {
                Ok(found) => found,
                Err(LinkError::NoBluetooth(why)) => {
                    self.update(|s| {
                        s.phase = Phase::NoBluetooth;
                        s.error = Some(why);
                    });
                    self.wait(SCAN_REST * 3).await;
                    continue;
                }
                Err(e) => {
                    crate::hostlog::line("pocket", &format!("scan: {e}"));
                    self.wait(SCAN_REST).await;
                    continue;
                }
            };
            let Some(peripheral) = found else {
                self.wait(SCAN_REST).await;
                continue;
            };
            self.update(|s| s.phase = Phase::Connecting);
            let end = match Link::open(peripheral).await {
                Ok(mut link) => {
                    let end = self.session(&mut link, &key, fresh).await;
                    link.close().await;
                    end
                }
                Err(e) => End::Lost(e.to_string()),
            };
            self.update(|s| {
                s.transfer = None;
                s.checking = false;
                s.recording = false;
            });
            match end {
                End::Paired => {
                    self.pairing = false;
                    match self.keychain.set(KEY_ACCOUNT, &key) {
                        Ok(()) => crate::hostlog::line("pocket", "paired"),
                        Err(e) => self.update(|s| s.error = Some(format!("The key couldn't be kept in the keychain: {e}"))),
                    }
                    // The device drops the link a moment after taking a key.
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                End::Refused => {
                    self.pairing = false;
                    self.update(|s| {
                        s.phase = Phase::Refused;
                        s.error = Some("The Pocket has another key: reset it and pair it again.".into());
                    });
                    self.wait(FOREVER).await;
                }
                End::Restart => {}
                End::Lost(why) => {
                    crate::hostlog::line("pocket", &format!("link ended: {why}"));
                    self.wait(Duration::from_secs(5)).await;
                }
            }
        }
    }

    async fn find(&mut self) -> Result<Option<btleplug::platform::Peripheral>, LinkError> {
        if self.bluetooth.is_none() {
            self.bluetooth = Some(Bluetooth::open().await?);
        }
        self.bluetooth.as_ref().unwrap().find(SCAN_FOR).await
    }

    /// Handle the app's commands for a while, not connected. Returns early
    /// when one needs the loop to start over.
    async fn wait(&mut self, time: Duration) {
        let sleep = tokio::time::sleep(time);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => return,
                command = self.commands.recv() => {
                    let Some(command) = command else { return std::future::pending().await };
                    if self.idle_command(command) {
                        return;
                    }
                }
            }
        }
    }

    /// A command while not connected. True when the loop must start over.
    fn idle_command(&mut self, command: Command) -> bool {
        match command {
            Command::Pair => {
                self.pairing = true;
                self.update(|s| s.error = None);
                true
            }
            Command::CancelPair => {
                self.pairing = false;
                true
            }
            Command::Forget => {
                self.pairing = false;
                let _ = self.keychain.delete(KEY_ACCOUNT);
                if let Ok(d) = dir() {
                    let _ = fs::remove_file(d.join("device.json"));
                }
                self.update(|s| {
                    *s = Status::new(s.settings.clone());
                    s.name = None;
                });
                true
            }
            Command::Settings(settings) => self.apply_settings(settings),
            // Only possible connected: look for the device now.
            Command::Sync | Command::Download(_) | Command::Delete(_) => true,
        }
    }

    /// True when it turns the link on or off.
    fn apply_settings(&self, settings: Settings) -> bool {
        if let Err(e) = write_json("settings.json", &settings) {
            crate::hostlog::line("pocket", &format!("settings: {e}"));
        }
        let switched = self.settings().enabled != settings.enabled;
        self.update(|s| s.settings = settings);
        switched
    }

    /// One connection, from the key to the link's end.
    async fn session(&mut self, link: &mut Link, key: &str, fresh: bool) -> End {
        let name = link.name().to_string();
        match link.request(&command::key(key), |m| matches!(m, Message::Key(_)), Duration::from_secs(5)).await {
            Ok(Message::Key(true)) if fresh => {
                let _ = write_json("device.json", &Known { name: Some(name.clone()) });
                self.update(|s| s.name = Some(name));
                return End::Paired;
            }
            Ok(Message::Key(true)) => {}
            Ok(_) => return End::Refused,
            // Paired elsewhere, the device drops a wrong key's link at once.
            Err(LinkError::Closed) => return End::Refused,
            Err(e) => return End::Lost(format!("no answer to the key: {e}")),
        }
        let _ = write_json("device.json", &Known { name: Some(name.clone()) });
        self.update(|s| {
            s.phase = Phase::Connected;
            s.name = Some(name);
            s.error = None;
        });
        crate::hostlog::line("pocket", "connected");
        if let Err(e) = self.read_status(link).await {
            return End::Lost(e.to_string());
        }
        if self.settings().set_clock {
            let _ = link.request(&command::clock(chrono::Utc::now()), |m| *m == Message::Clock, Duration::from_secs(3)).await;
        }
        let auto = self.settings().auto_sync;
        if let Err(e) = self.sync(link, if auto { Pick::New } else { Pick::None }).await {
            return End::Lost(e.to_string());
        }
        self.stay(link).await
    }

    async fn read_status(&self, link: &mut Link) -> Result<(), LinkError> {
        type Want = fn(&Message) -> bool;
        let asks: [(&str, Want); 5] = [
            (command::BATTERY, |m| matches!(m, Message::Battery(_))),
            (command::FIRMWARE, |m| matches!(m, Message::Firmware(_))),
            (command::WIFI_FIRMWARE, |m| matches!(m, Message::WifiFirmware(_))),
            (command::SPACE, |m| matches!(m, Message::Space(..))),
            (command::STATE, |m| matches!(m, Message::State(_))),
        ];
        for (cmd, want) in asks {
            match link.request(cmd, want, Duration::from_secs(3)).await {
                Ok(message) => self.note(&message),
                Err(LinkError::Timeout) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// What a message says about the device, into the status.
    fn note(&self, message: &Message) {
        self.update(|s| match message {
            Message::Battery(b) => s.battery = Some(*b),
            Message::Firmware(v) => s.firmware = Some(v.clone()),
            Message::WifiFirmware(v) => s.wifi_firmware = Some(v.clone()),
            Message::Space(free, total) => {
                s.free_mb = Some(*free);
                s.total_mb = Some(*total);
            }
            Message::State(n) => s.recording = *n == 1,
            Message::Started(_) => s.recording = true,
            Message::Stopped => s.recording = false,
            _ => {}
        });
    }

    /// List the recordings, then download those `pick` picks, oldest first.
    async fn sync(&mut self, link: &mut Link, pick: Pick) -> Result<(), LinkError> {
        self.update(|s| s.checking = true);
        let listed = self.list(link).await;
        self.update(|s| s.checking = false);
        let recordings = listed?;
        let done = synced();
        self.update(|s| s.recordings = view(&recordings, &done));
        let wanted: Vec<Recording> = recordings
            .into_iter()
            .rev()
            .filter(|r| !done.contains_key(&r.name))
            .filter(|r| match &pick {
                Pick::None => false,
                Pick::New => !self.failed.contains(&r.name),
                Pick::These(names) => names.contains(&r.name),
            })
            .collect();
        let count = wanted.len();
        crate::hostlog::line("pocket", &format!("{} recordings on the Pocket, {} to copy", self.status.lock().unwrap().recordings.len(), count));
        for (i, recording) in wanted.iter().enumerate() {
            self.fetch(link, recording, count - i - 1).await?;
        }
        let done = synced();
        self.update(|s| {
            s.recordings.iter_mut().for_each(|r| r.meeting = done.get(&r.recording.name).cloned());
            s.synced_at = Some(chrono::Utc::now().timestamp_millis());
        });
        if count > 0 {
            // Deletions change the free space.
            if let Ok(m) = link.request(command::SPACE, |m| matches!(m, Message::Space(..)), Duration::from_secs(3)).await {
                self.note(&m);
            }
        }
        Ok(())
    }

    /// Every recording on the device, newest first. A listing that doesn't
    /// end with its count is incomplete, not empty: it fails.
    async fn list(&mut self, link: &mut Link) -> Result<Vec<Recording>, LinkError> {
        let wait = Duration::from_secs(8);
        let messages = link.collect(command::DIRS, |m| matches!(m, Message::DirsEnd(_)), wait).await?;
        let dates: Vec<String> = messages.into_iter().filter_map(|m| if let Message::Dir(d) = m { Some(d) } else { None }).filter(|d| protocol::safe_name(d)).collect();
        let mut all = Vec::new();
        for date in dates {
            let messages = link.collect(&command::list(&date), |m| matches!(m, Message::ListEnd(_)), wait).await?;
            all.extend(messages.into_iter().filter_map(|m| if let Message::File(r) = m { Some(r) } else { None }).filter(|r| protocol::safe_name(&r.name)));
        }
        all.sort_by(|a, b| b.name.cmp(&a.name));
        Ok(all)
    }

    /// Download one recording and make it a meeting. A file that doesn't
    /// come whole is skipped (and remembered as failed); a broken link
    /// ends the sync.
    async fn fetch(&mut self, link: &mut Link, recording: &Recording, queued: usize) -> Result<(), LinkError> {
        let name = recording.name.clone();
        self.update(|s| s.transfer = Some(Transfer { name: name.clone(), bytes: 0, total: recording.estimated_bytes(), queued }));
        crate::hostlog::line("pocket", &format!("{name}: copying ({} s)", recording.seconds));
        let status = self.status.clone();
        let on_event = &self.on_event;
        let bytes = link
            .download(&command::upload(&recording.date, &name), |got, total| {
                let snapshot = {
                    let mut s = status.lock().unwrap();
                    s.transfer = Some(Transfer { name: name.clone(), bytes: got, total, queued });
                    s.clone()
                };
                on_event(Event::Status(Box::new(snapshot)));
            })
            .await;
        self.update(|s| s.transfer = None);
        let bytes = match bytes {
            Ok(bytes) => bytes,
            Err(e @ (LinkError::Incomplete(..) | LinkError::Timeout)) => {
                self.failed.insert(name.clone());
                self.update(|s| s.error = Some(format!("{name}: {e}")));
                return if link.is_connected().await { Ok(()) } else { Err(LinkError::Closed) };
            }
            Err(e) => return Err(e),
        };
        match import(recording, &bytes) {
            Ok(meeting) => {
                crate::hostlog::line("pocket", &format!("{name}: {} bytes, meeting {}", bytes.len(), meeting.id));
                self.failed.remove(&name);
                if let Err(e) = mark_synced(&name, &meeting.id) {
                    crate::hostlog::line("pocket", &format!("synced.json: {e}"));
                }
                (self.on_event)(Event::Imported(Box::new(meeting)));
                if self.settings().delete_after_sync {
                    self.delete(link, recording).await?;
                }
            }
            Err(e) => {
                self.failed.insert(name.clone());
                self.update(|s| s.error = Some(format!("{name}: {e}")));
            }
        }
        Ok(())
    }

    async fn delete(&mut self, link: &mut Link, recording: &Recording) -> Result<(), LinkError> {
        match link.request(&command::delete(&recording.date, &recording.name), |m| *m == Message::Deleted, Duration::from_secs(5)).await {
            Ok(_) => {
                crate::hostlog::line("pocket", &format!("{}: deleted from the Pocket", recording.name));
                self.update(|s| s.recordings.retain(|r| r.recording.name != recording.name));
                Ok(())
            }
            Err(LinkError::Timeout) => {
                self.update(|s| s.error = Some(format!("{}: the Pocket didn't confirm the deletion", recording.name)));
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Connected and idle: the device's events, the app's commands, and a
    /// look at the battery now and then.
    async fn stay(&mut self, link: &mut Link) -> End {
        let mut refresh = tokio::time::interval(REFRESH_EVERY);
        refresh.tick().await;
        loop {
            let result = tokio::select! {
                message = link.next() => match message {
                    Ok(Message::Stopped) => {
                        self.note(&Message::Stopped);
                        // The file is complete a moment after the event.
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        let auto = self.settings().auto_sync;
                        self.sync(link, if auto { Pick::New } else { Pick::None }).await
                    }
                    Ok(message) => {
                        self.note(&message);
                        Ok(())
                    }
                    Err(e) => Err(e),
                },
                command = self.commands.recv() => match command {
                    None => return End::Lost("the app went away".into()),
                    Some(Command::Sync) => {
                        self.failed.clear();
                        self.sync(link, Pick::New).await
                    }
                    Some(Command::Download(names)) => {
                        for n in &names {
                            self.failed.remove(n);
                        }
                        self.sync(link, Pick::These(names)).await
                    }
                    Some(Command::Delete(name)) => {
                        let found = self.status.lock().unwrap().recordings.iter().find(|r| r.recording.name == name).map(|r| r.recording.clone());
                        match found {
                            Some(recording) => self.delete(link, &recording).await,
                            None => Ok(()),
                        }
                    }
                    Some(Command::Settings(settings)) => {
                        let auto_on = settings.auto_sync && !self.settings().auto_sync;
                        if self.apply_settings(settings) {
                            return End::Restart;
                        }
                        if auto_on { self.sync(link, Pick::New).await } else { Ok(()) }
                    }
                    Some(Command::CancelPair) => Ok(()),
                    Some(other) => {
                        self.idle_command(other);
                        return End::Restart;
                    }
                },
                _ = refresh.tick() => self.read_status(link).await,
            };
            if let Err(e) = result {
                return End::Lost(e.to_string());
            }
        }
    }
}

/// Keep the recording, then make it a meeting (which copies it).
fn import(recording: &Recording, bytes: &[u8]) -> io::Result<Meeting> {
    let folder = dir()?.join("recordings");
    fs::create_dir_all(&folder)?;
    let path = folder.join(format!("{}.mp3", recording.name));
    fs::write(&path, bytes)?;
    let started = recording.started_at().map(|t| t.with_timezone(&chrono::Local)).unwrap_or_else(chrono::Local::now);
    let meeting = meetings::import_recording(&path, started, "pocket", "Pocket")?;
    let _ = fs::remove_file(&path);
    Ok(meeting)
}

fn valid_key(key: &str) -> bool {
    key.len() == 16 && key.chars().all(|c| c.is_ascii_hexdigit())
}

/// The listing as the app shows it.
fn view(recordings: &[Recording], done: &BTreeMap<String, String>) -> Vec<DeviceRecording> {
    recordings
        .iter()
        .map(|r| DeviceRecording { recording: r.clone(), started_at: r.started_at().map(|t| t.timestamp_millis()), meeting: done.get(&r.name).cloned() })
        .collect()
}
