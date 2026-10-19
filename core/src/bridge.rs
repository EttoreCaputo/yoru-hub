//! Sample sensors at a fixed rate and send each snapshot to the board,
//! reconnecting when the USB cable or the board goes away.
//!
//! Frontends observe the loop through [`BridgeEvent`]s: the CLI prints them,
//! a Tauri app can forward them to the webview with `app.emit(..)`.
//!
//! The work is split by what it belongs to:
//!
//! - [`sampler`]: the computer's side, once for every board: Claude Code's
//!   usage, the timers, the time and your presence. It builds a snapshot
//!   each interval while a session is open and publishes it.
//! - [`session`]: one board's connection: the protocol v2 handshake (see
//!   [`crate::link`]), its reader and voice pipeline, the pointer for its
//!   eyes, and sending it each snapshot.
//! - [`alerts`]: what is said out loud of the host's own accord: Claude Code
//!   alerts and timers that are up.
//! - [`agents`]: Claude Code's sessions step by step, and its permission
//!   requests, to the frontend and to the boards that can answer them.
//!
//! With a [`Hub`] other processes (the MCP server, `yoru call`) use the
//! board through this connection, and bring Claude Code's hook events
//! (`host/claude_hook`) to the alerts.
//!
//! - [`manager`]: finds the boards on USB and WiFi and runs a session for
//!   each, up to a cap. [`Bridge`] is the manager with one board at most, as
//!   the bridge always was.

mod agents;
mod alerts;
mod floor;
mod manager;
mod sampler;
mod session;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::assistant::Assistant;
use crate::claude::agents::{AgentSession, Agents, Approval};
use crate::claude::alerts::{Alert, AlertSettings};
use crate::firmware::BoardFirmware;
use crate::hub::Hub;
use crate::snapshot::Snapshot;
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
    /// Lets other processes use the board through this bridge.
    pub hub: Option<Arc<Hub>>,
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
    /// Send the mouse pointer to Yoru's eyes. Shared, so a frontend can
    /// turn it off while the bridge runs.
    pub follow_pointer: Arc<AtomicBool>,
    /// Yoru the assistant: it answers what the board hears, and its inbox
    /// goes to the board. Without it the voice uses [`VoiceConfig::agent`].
    pub assistant: Option<Arc<Assistant>>,
    /// The id of the board that says alerts when several are
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
            hub: None,
            voice: Arc::default(),
            claude_alerts: Arc::default(),
            agents: Arc::default(),
            timers: Arc::default(),
            follow_pointer: Arc::new(AtomicBool::new(true)),
            primary: Arc::default(),
            assistant: None,
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
    /// A snapshot was taken; `sent` is false when the board isn't ready for it.
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
    /// An alert about Claude Code, and how it went: `error` says why it
    /// didn't reach the board (or wasn't spoken).
    ClaudeAlert { alert: Alert, text: String, error: Option<String> },
    /// Claude Code's sessions or permission requests changed: all of them,
    /// as [`Agents::view`] has them.
    Agents { sessions: Vec<AgentSession>, approvals: Vec<Approval> },
    /// A timer or reminder is up (or a pomodoro moved on); `text` is what was
    /// said about it, `error` why it wasn't.
    TimerFired { fired: Fired, text: String, error: Option<String> },
    /// A notification reached the board (`error`: why it didn't).
    Notified { id: String, error: Option<String> },
    /// The user opened a notification on the board: the app opens its link.
    NotificationOpened { id: String, url: Option<String> },
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
            .name("yoru-bridge".into())
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
        let _ = thread::Builder::new().name("yoru-llm-start".into()).spawn(move || match agent.llm().warm_up() {
            Ok(()) => sink(BridgeEvent::BoardLog { line: format!("host: language model ready ({})", agent.llm().config().label()) }),
            Err(e) => sink(BridgeEvent::VoiceError { message: e.to_string() }),
        });
    }
    if let Some(stt) = stt {
        let sink = on_event.clone();
        let _ = thread::Builder::new().name("yoru-stt-start".into()).spawn(move || match stt.warm_up() {
            Ok(()) => sink(BridgeEvent::BoardLog { line: format!("host: speech-to-text ready ({})", stt.config().engine.label()) }),
            Err(e) => sink(BridgeEvent::VoiceError { message: e.to_string() }),
        });
    }
    if let Some(tts) = tts {
        let sink = on_event.clone();
        let _ = thread::Builder::new().name("yoru-tts-start".into()).spawn(move || match tts.warm_up() {
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
