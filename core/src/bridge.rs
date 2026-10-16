//! Sample sensors at a fixed rate and send each snapshot to the board,
//! reconnecting when the USB cable or the board goes away.
//!
//! Frontends observe the loop through [`BridgeEvent`]s: the CLI prints them,
//! a Tauri app can forward them to the webview with `app.emit(..)`.
//!
//! The work is split by what it belongs to:
//!
//! - [`sampler`]: the computer's side, once for every board: the sensors,
//!   Claude Code's usage, the timers, what's playing, the pet's diary. It
//!   builds a snapshot each interval while a session is open and publishes it.
//! - [`session`]: one board's connection: the protocol v2 handshake (see
//!   [`crate::link`]), its reader and voice pipeline, the pointer for its
//!   eyes, and sending it each snapshot.
//! - [`behaviour`]: what differs by kind of board ([`crate::BoardProfile`]):
//!   a DualEye's faces, rotation and music cover ([`dualeye`]), a Yoru's
//!   single screen ([`yoru`]).
//! - [`alerts`]: what is said out loud of the host's own accord: Claude Code
//!   alerts, timers that are up, the pet's comments on your day.
//! - [`agents`]: Claude Code's sessions step by step, and its permission
//!   requests, to the frontend and to the boards that can answer them.
//!
//! With a [`Hub`] other processes (the MCP server, `companion call`) use the
//! board through this connection, and bring Claude Code's hook events
//! (`host/claude_hook`) to the alerts.
//!
//! - [`manager`]: finds the boards on USB and WiFi and runs a session for
//!   each, up to a cap. [`Bridge`] is the manager with one board at most, as
//!   the bridge always was.

mod agents;
mod alerts;
mod behaviour;
mod dualeye;
mod floor;
mod manager;
mod sampler;
mod session;
mod yoru;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::claude::agents::{AgentSession, Agents, Approval};
use crate::claude::alerts::{Alert, AlertSettings};
use crate::diary;
use crate::firmware::BoardFirmware;
use crate::hub::Hub;
use crate::music::Music;
use crate::quips::{Quip, QuipLevel};
use crate::snapshot::{Faces, Rotations, Snapshot};
use crate::stt::Transcript;
use crate::timers::{Fired, Timers};
use crate::voice::{Spoken, Utterance, VoiceConfig};

pub use manager::{Manager, ManagerEvent, ManagerSink, run as run_manager};

/// Which way the bridge reaches the board, when no port is given.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    /// USB when the board is plugged in, otherwise WiFi (firmware 1.4.3), and
    /// back to USB as soon as it's plugged in again.
    #[default]
    Auto,
    Usb,
    Wifi,
}

#[derive(Debug, Clone)]
pub struct BridgeConfig {
    /// Serial port, or `wifi:<id>` for a paired board on WiFi; `None` finds
    /// the board on every (re)connect, the way `transport` says.
    pub port: Option<String>,
    pub transport: Transport,
    /// Time between snapshots. The firmware marks data stale after 3 s.
    pub interval: Duration,
    /// Watch face of each screen. Shared, so a frontend can change it while
    /// the bridge runs; the board gets it by the next snapshot.
    pub faces: Arc<Mutex<Faces>>,
    /// How each screen is turned, shared the same way as `faces`.
    pub rotation: Arc<Mutex<Rotations>>,
    /// Faces and rotation of each DualEye by its id, when it has its own;
    /// one without an entry gets `faces` and `rotation`. Shared like `faces`.
    pub board_settings: Arc<Mutex<BTreeMap<String, ScreenSettings>>>,
    /// Lets other processes use the board through this bridge.
    pub hub: Option<Arc<Hub>>,
    /// On each handshake, take `faces` and `rotation` from the board instead
    /// of pushing ours. Off when the user asked for specific ones.
    pub adopt_board_settings: bool,
    /// What happens to what the board hears after its wake word. Shared, so
    /// a frontend can turn transcription on or change the model while the
    /// bridge runs; each utterance uses the config of the moment it ends.
    pub voice: Arc<Mutex<VoiceConfig>>,
    /// Which Claude Code alerts to give, and how. Shared like `voice`.
    pub claude_alerts: Arc<Mutex<AlertSettings>>,
    /// Claude Code's sessions and permission requests, fed by the hooks.
    /// Shared, so the frontend reads and answers them.
    pub agents: Arc<Agents>,
    /// Timers, pomodoro and reminders: the board shows and rings them, the
    /// voice and hub clients set them.
    pub timers: Arc<Timers>,
    /// What's playing, for the music face; the voice plays, pauses and skips.
    pub music: Arc<Music>,
    /// Send the mouse pointer to the eyes face. Shared, so a frontend can
    /// turn it off while the bridge runs.
    pub follow_pointer: Arc<AtomicBool>,
    /// How often the pet comments on your day, out loud (with text-to-speech
    /// on). Shared like `voice`; in the language of `claude_alerts`.
    pub quips: Arc<Mutex<QuipLevel>>,
    /// Where the pet's diary of your days is kept; `None` keeps it in memory.
    pub diary: Option<PathBuf>,
    /// The id of the board that says alerts and quips when several are
    /// connected (a timer is said by the board it was set from). Shared like
    /// `voice`.
    pub primary: Arc<Mutex<Option<String>>>,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            port: None,
            transport: Transport::Auto,
            interval: Duration::from_secs(1),
            faces: Arc::default(),
            rotation: Arc::default(),
            board_settings: Arc::default(),
            hub: None,
            adopt_board_settings: true,
            voice: Arc::default(),
            claude_alerts: Arc::default(),
            agents: Arc::default(),
            timers: Arc::default(),
            music: Arc::default(),
            follow_pointer: Arc::new(AtomicBool::new(true)),
            quips: Arc::default(),
            diary: diary::default_file(),
            primary: Arc::default(),
        }
    }
}

/// One DualEye's watch faces and how its screens are turned.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenSettings {
    pub faces: Faces,
    pub rotation: Rotations,
}

impl BridgeConfig {
    /// What DualEye `board` shows: its own settings, else the shared ones.
    pub fn screens(&self, board: Option<&str>) -> ScreenSettings {
        board
            .and_then(|id| self.board_settings.lock().unwrap().get(id).copied())
            .unwrap_or_else(|| ScreenSettings { faces: *self.faces.lock().unwrap(), rotation: *self.rotation.lock().unwrap() })
    }

    /// Keep `settings` as DualEye `board`'s (the shared ones without an id).
    pub fn set_screens(&self, board: Option<&str>, settings: ScreenSettings) {
        match board {
            Some(id) => {
                self.board_settings.lock().unwrap().insert(id.to_string(), settings);
            }
            None => {
                *self.faces.lock().unwrap() = settings.faces;
                *self.rotation.lock().unwrap() = settings.rotation;
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
// A snapshot a second: its size doesn't matter.
#[allow(clippy::large_enum_variant)]
pub enum BridgeEvent {
    /// No board to talk to yet; the bridge keeps retrying.
    Waiting { reason: String },
    Connected { port: String },
    /// A snapshot was taken; `sent` is false when it had no temperature to
    /// show or the board isn't ready for it.
    Snapshot { snapshot: Snapshot, sent: bool },
    /// A line from the board's log, or console text from outside the
    /// protocol (boot messages, a crash).
    BoardLog { line: String },
    /// What the board runs; raised once per connection, and again if it reboots into another version.
    Firmware { firmware: BoardFirmware },
    /// The board heard its wake word. `volume_db` is the input level (dBFS).
    Wake { word: String, volume_db: Option<f64> },
    /// What the board's "eyes" overlay shows: `idle`, `listening`, `thinking` or `speaking`.
    VoiceState { state: String },
    /// The board started streaming what it hears: after the wake word
    /// (`trigger` `wake`) or `voice/listen` (`host`).
    Listening { id: u8, trigger: String },
    /// An utterance ended and arrived. `wav` is where a copy was saved, when
    /// [`VoiceConfig::dump_dir`] is set.
    Utterance { utterance: Utterance, duration_ms: u64, peak_db: Option<f64>, wav: Option<String> },
    /// What was said in utterance `id`; `None` when Whisper heard no words.
    Transcript { id: u8, transcript: Option<Transcript> },
    /// What the host made of transcript `id`: the board tools it called
    /// (`actions`, with what each said) and the answer, in `language`.
    /// `understood` is false when no command matched. `by` is `llm` (the
    /// local language model) or `rules` (without one, or when it failed).
    Reply { id: u8, text: String, language: String, actions: Vec<String>, understood: bool, by: String, elapsed_ms: u64 },
    /// The answer to `id` was spoken through the board's speaker.
    Spoken { id: u8, spoken: Spoken },
    /// Speech-to-text or text-to-speech failed, or its sidecar couldn't start.
    VoiceError { message: String },
    /// Faces or rotation came from the board: at the handshake, or because a
    /// hub client changed them. `faces` and `rotation` in the config hold the
    /// new values already.
    Settings { faces: Faces, rotation: Rotations },
    /// An alert about Claude Code, and how it went: `error` says why it
    /// didn't reach the board (or wasn't spoken).
    ClaudeAlert { alert: Alert, text: String, error: Option<String> },
    /// Claude Code's sessions or permission requests changed: all of them,
    /// as [`Agents::view`] has them.
    Agents { sessions: Vec<AgentSession>, approvals: Vec<Approval> },
    /// A timer or reminder is up (or a pomodoro moved on); `text` is what was
    /// said about it, `error` why it wasn't.
    TimerFired { fired: Fired, text: String, error: Option<String> },
    /// The pet commented on your day ([`crate::quips`]); `error` says why it
    /// wasn't said.
    Quip { quip: Quip, text: String, error: Option<String> },
    Disconnected { port: String, reason: String, permission_denied: bool },
}

pub type EventSink = Arc<dyn Fn(BridgeEvent) + Send + Sync>;


/// The bridge loop on its own thread. Dropping it stops the loop.
pub struct Bridge {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Bridge {
    /// One board at a time, as the bridge always drove: the [`Manager`] with
    /// `max_boards` 1, its events without their session.
    pub fn spawn(config: BridgeConfig, on_event: impl Fn(BridgeEvent) + Send + Sync + 'static) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let sink: EventSink = Arc::new(on_event);
        let thread = thread::Builder::new()
            .name("companion-bridge".into())
            .spawn(move || run(&config, &flag, sink))
            .expect("spawn bridge thread");
        Self { stop, thread: Some(thread) }
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// How long the board gets for a tool call the bridge makes itself.
const TOOL_TIMEOUT: Duration = Duration::from_secs(1);

/// Run the bridge on the current thread until `stop` is set: the
/// [`Manager`] with one board at most, its events without their session.
pub fn run(config: &BridgeConfig, stop: &AtomicBool, on_event: EventSink) {
    manager::run(config, Some(1), stop, Arc::new(move |e: ManagerEvent| on_event(e.event)));
}

/// Whisper, Piper and llama.cpp load their models meanwhile, so the first utterance doesn't wait.
fn warm_up_voice(config: &BridgeConfig, on_event: &EventSink) {
    let (stt, tts, agent) = {
        let voice = config.voice.lock().unwrap();
        (voice.stt.clone(), voice.tts.clone(), voice.agent.clone())
    };
    if let Some(agent) = agent {
        let sink = on_event.clone();
        let _ = thread::Builder::new().name("companion-llm-start".into()).spawn(move || match agent.llm().warm_up() {
            Ok(()) => sink(BridgeEvent::BoardLog { line: format!("host: language model ready ({})", agent.llm().config().label()) }),
            Err(e) => sink(BridgeEvent::VoiceError { message: e.to_string() }),
        });
    }
    if let Some(stt) = stt {
        let sink = on_event.clone();
        let _ = thread::Builder::new().name("companion-stt-start".into()).spawn(move || match stt.warm_up() {
            Ok(()) => sink(BridgeEvent::BoardLog { line: format!("host: speech-to-text ready ({})", stt.config().engine.label()) }),
            Err(e) => sink(BridgeEvent::VoiceError { message: e.to_string() }),
        });
    }
    if let Some(tts) = tts {
        let sink = on_event.clone();
        let _ = thread::Builder::new().name("companion-tts-start".into()).spawn(move || match tts.warm_up() {
            Ok(()) => {
                let voices: Vec<&str> = tts.config().voices.values().map(String::as_str).collect();
                sink(BridgeEvent::BoardLog { line: format!("host: text-to-speech ready ({})", voices.join(", ")) })
            }
            Err(e) => sink(BridgeEvent::VoiceError { message: e.to_string() }),
        });
    }
}

fn sleep_unless_stopped(duration: Duration, stop: &AtomicBool) {
    let end = Instant::now() + duration;
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if now >= end {
            return;
        }
        thread::sleep((end - now).min(Duration::from_millis(50)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{Face, Rotation};

    #[test]
    fn a_dualeye_with_an_id_keeps_its_own_screens() {
        let config = BridgeConfig::default();
        let mut own = config.screens(None);
        own.faces.cpu = Face::Eyes;
        own.rotation.gpu = Rotation::from_degrees(180).unwrap();
        config.set_screens(Some("aa"), own);
        assert_eq!(config.screens(Some("aa")), own);
        // Another board, and one without an id, get the shared ones.
        assert_eq!(config.screens(Some("bb")), config.screens(None));
        assert_ne!(config.screens(None), own);
    }
}
