//! The Bluetooth connection to the Pocket (btleplug): commands written to
//! one characteristic, answers notified on the same one, recordings
//! notified on another.

use std::collections::VecDeque;
use std::fmt;
use std::pin::Pin;
use std::time::Duration;

use btleplug::api::{Central, CentralState, Characteristic, Manager as _, Peripheral as _, ScanFilter, ValueNotification, WriteType};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::{FutureExt as _, Stream, StreamExt};
use uuid::Uuid;

use super::protocol::{self, Message};

/// The service the Pocket advertises (16-bit 0x5536).
const ADVERTISED: Uuid = Uuid::from_u128(0x0000_5536_0000_1000_8000_0080_5f9b_34fb);
/// Commands in, answers and events out.
const COMMANDS: Uuid = Uuid::from_u128(0x0011_20a3_2233_4455_6677_8899_1234_5678);
/// A recording's MP3, as it downloads or while it is being made.
const AUDIO: Uuid = Uuid::from_u128(0x0011_20a1_2233_4455_6677_8899_1234_5678);

/// A download that stops sending for this long has stalled.
const STALL: Duration = Duration::from_secs(15);

#[derive(Debug)]
pub enum LinkError {
    /// Bluetooth is off, missing, or not allowed.
    NoBluetooth(String),
    /// No answer in time.
    Timeout,
    /// The connection ended.
    Closed,
    /// A download ended short: what arrived, and what was announced.
    Incomplete(u64, u64),
    Ble(btleplug::Error),
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            LinkError::NoBluetooth(why) => write!(f, "{why}"),
            LinkError::Timeout => write!(f, "the Pocket didn't answer in time"),
            LinkError::Closed => write!(f, "the connection ended"),
            LinkError::Incomplete(got, total) => write!(f, "only {got} of {total} bytes arrived"),
            LinkError::Ble(e) => write!(f, "Bluetooth: {e}"),
        }
    }
}

impl From<btleplug::Error> for LinkError {
    fn from(e: btleplug::Error) -> Self {
        match e {
            btleplug::Error::PermissionDenied => LinkError::NoBluetooth("Yoru isn't allowed to use Bluetooth (System Settings > Privacy & Security > Bluetooth)".into()),
            btleplug::Error::NotConnected => LinkError::Closed,
            e => LinkError::Ble(e),
        }
    }
}

/// The computer's Bluetooth adapter, opened once.
pub struct Bluetooth {
    adapter: Adapter,
}

impl Bluetooth {
    pub async fn open() -> Result<Bluetooth, LinkError> {
        let manager = Manager::new().await?;
        let adapter = manager.adapters().await?.into_iter().next().ok_or_else(|| LinkError::NoBluetooth("this computer has no Bluetooth".into()))?;
        Ok(Bluetooth { adapter })
    }

    /// Look for a Pocket for up to `within`; the one named `name` when
    /// known (the name only shows once connected, so any Pocket found is
    /// checked then).
    pub async fn find(&self, within: Duration) -> Result<Option<Peripheral>, LinkError> {
        // CoreBluetooth reports Unknown for a moment after it starts.
        for _ in 0..10 {
            match self.adapter.adapter_state().await? {
                CentralState::PoweredOn => break,
                CentralState::PoweredOff => return Err(LinkError::NoBluetooth("Bluetooth is off".into())),
                CentralState::Unknown => tokio::time::sleep(Duration::from_millis(300)).await,
            }
        }
        let mut events = self.adapter.events().await?;
        self.adapter.start_scan(ScanFilter { services: vec![ADVERTISED] }).await?;
        let found = tokio::time::timeout(within, async {
            // One seen in an earlier scan may already be known.
            if let Some(p) = self.pockets().await.into_iter().next() {
                return Some(p);
            }
            while let Some(event) = events.next().await {
                if let btleplug::api::CentralEvent::DeviceDiscovered(id) | btleplug::api::CentralEvent::ServicesAdvertisement { id, .. } = event
                    && let Ok(p) = self.adapter.peripheral(&id).await
                    && advertises(&p).await
                {
                    return Some(p);
                }
            }
            None
        })
        .await
        .ok()
        .flatten();
        let _ = self.adapter.stop_scan().await;
        Ok(found)
    }

    async fn pockets(&self) -> Vec<Peripheral> {
        let mut out = Vec::new();
        for p in self.adapter.peripherals().await.unwrap_or_default() {
            if advertises(&p).await {
                out.push(p);
            }
        }
        out
    }
}

async fn advertises(p: &Peripheral) -> bool {
    match p.properties().await {
        Ok(Some(props)) => props.services.contains(&ADVERTISED) || props.local_name.is_some_and(|n| n.starts_with("PKT01")),
        _ => false,
    }
}

type Notes = Pin<Box<dyn Stream<Item = ValueNotification> + Send>>;

/// One connection.
pub struct Link {
    peripheral: Peripheral,
    commands: Characteristic,
    notes: Notes,
    /// Messages that came in one notification after the one wanted.
    queued: VecDeque<Message>,
    /// Events that came while waiting for an answer.
    events: VecDeque<Message>,
    name: String,
}

impl Link {
    pub async fn open(peripheral: Peripheral) -> Result<Link, LinkError> {
        tokio::time::timeout(Duration::from_secs(15), peripheral.connect()).await.map_err(|_| LinkError::Timeout)??;
        let opened = async {
            peripheral.discover_services().await?;
            let chars = peripheral.characteristics();
            let find = |uuid: Uuid| chars.iter().find(|c| c.uuid == uuid).cloned();
            let (Some(commands), Some(audio)) = (find(COMMANDS), find(AUDIO)) else {
                return Err(LinkError::NoBluetooth("this device doesn't look like a Pocket".into()));
            };
            let notes = peripheral.notifications().await?;
            peripheral.subscribe(&commands).await?;
            peripheral.subscribe(&audio).await?;
            let name = peripheral.properties().await?.and_then(|p| p.local_name).unwrap_or_else(|| "Pocket".into());
            Ok((commands, notes, name))
        };
        match tokio::time::timeout(Duration::from_secs(15), opened).await {
            Ok(Ok((commands, notes, name))) => {
                Ok(Link { peripheral, commands, notes, queued: VecDeque::new(), events: VecDeque::new(), name })
            }
            Ok(Err(e)) => {
                let _ = peripheral.disconnect().await;
                Err(e)
            }
            Err(_) => {
                let _ = peripheral.disconnect().await;
                Err(LinkError::Timeout)
            }
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub async fn is_connected(&self) -> bool {
        self.peripheral.is_connected().await.unwrap_or(false)
    }

    pub async fn close(&mut self) {
        let _ = self.peripheral.disconnect().await;
    }

    async fn send(&self, command: &str) -> Result<(), LinkError> {
        self.peripheral.write(&self.commands, command.as_bytes(), WriteType::WithoutResponse).await?;
        Ok(())
    }

    /// The next message on the command characteristic; audio that isn't
    /// being downloaded (a recording streams live while it is made) is
    /// dropped. The device going quiet for good is caught by a check on the
    /// connection now and then.
    async fn message(&mut self) -> Result<Message, LinkError> {
        loop {
            if let Some(m) = self.queued.pop_front() {
                return Ok(m);
            }
            match tokio::time::timeout(Duration::from_secs(30), self.notes.next()).await {
                Ok(Some(note)) if note.uuid == COMMANDS => self.queued.extend(protocol::split(&note.value).iter().map(|t| protocol::parse(t))),
                Ok(Some(_)) => {}
                Ok(None) => return Err(LinkError::Closed),
                Err(_) => {
                    if !self.peripheral.is_connected().await.unwrap_or(false) {
                        return Err(LinkError::Closed);
                    }
                }
            }
        }
    }

    /// The next event (or anything said unasked), waiting as long as it takes.
    pub async fn next(&mut self) -> Result<Message, LinkError> {
        if let Some(m) = self.events.pop_front() {
            return Ok(m);
        }
        self.message().await
    }

    /// Keep what isn't an answer for [`Link::next`] when it matters.
    fn set_aside(&mut self, message: Message) {
        if matches!(message, Message::Started(_) | Message::Stopped) {
            self.events.push_back(message);
        }
    }

    /// Send `command` and wait for the message `want` picks.
    pub async fn request(&mut self, command: &str, want: impl Fn(&Message) -> bool, within: Duration) -> Result<Message, LinkError> {
        self.send(command).await?;
        let wait = async {
            loop {
                let m = self.message().await?;
                if want(&m) {
                    return Ok(m);
                }
                self.set_aside(m);
            }
        };
        tokio::time::timeout(within, wait).await.map_err(|_| LinkError::Timeout)?
    }

    /// Send `command` and gather what comes until `end`, which is included;
    /// `within` is the wait for each message.
    pub async fn collect(&mut self, command: &str, end: impl Fn(&Message) -> bool, within: Duration) -> Result<Vec<Message>, LinkError> {
        self.send(command).await?;
        let mut out = Vec::new();
        loop {
            let m = tokio::time::timeout(within, self.message()).await.map_err(|_| LinkError::Timeout)??;
            if matches!(m, Message::Started(_) | Message::Stopped) {
                self.set_aside(m);
                continue;
            }
            let last = end(&m);
            out.push(m);
            if last {
                return Ok(out);
            }
        }
    }

    /// Download a recording: `MCU&U&<size>`, the bytes on the audio
    /// characteristic, then `MCU&OFF`. `progress` hears the bytes so far
    /// and the total.
    pub async fn download(&mut self, command: &str, mut progress: impl FnMut(u64, u64)) -> Result<Vec<u8>, LinkError> {
        // Audio left from a live recording isn't this file's.
        while let Some(Some(note)) = self.notes.next().now_or_never() {
            if note.uuid == COMMANDS {
                self.queued.extend(protocol::split(&note.value).iter().map(|t| protocol::parse(t)));
            }
        }
        self.send(command).await?;
        let mut total: Option<u64> = None;
        let mut bytes = Vec::new();
        let mut shown = 0usize;
        loop {
            if let Some(m) = self.queued.pop_front() {
                match m {
                    Message::Upload(size) if total.is_none() => {
                        total = Some(size);
                        bytes.reserve(size as usize);
                    }
                    Message::Off if total.is_some() => break,
                    other => self.set_aside(other),
                }
                continue;
            }
            let note = tokio::time::timeout(STALL, self.notes.next()).await.map_err(|_| LinkError::Timeout)?.ok_or(LinkError::Closed)?;
            if note.uuid == COMMANDS {
                self.queued.extend(protocol::split(&note.value).iter().map(|t| protocol::parse(t)));
            } else if note.uuid == AUDIO && total.is_some() {
                bytes.extend_from_slice(&note.value);
                // A progress report every 16 KB is plenty.
                if bytes.len() - shown >= 16 * 1024 {
                    shown = bytes.len();
                    progress(bytes.len() as u64, total.unwrap_or(0));
                }
            }
        }
        let total = total.unwrap_or(0);
        if (bytes.len() as u64) < total {
            return Err(LinkError::Incomplete(bytes.len() as u64, total));
        }
        bytes.truncate(total as usize);
        progress(total, total);
        Ok(bytes)
    }
}

