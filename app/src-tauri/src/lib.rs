//! Tauri shell around `companion-core`: the bridge (a [`BoardManager`], one session
//! per board found) runs on its own thread for the whole life of the process,
//! and every event it raises is kept here, per board (so a freshly loaded
//! webview can catch up), and forwarded to the UI: all of them as `manager`,
//! and as `bridge` the host's own and those of the board the UI shows (the
//! primary one, else the first connected), for a UI that knows one board.
//! Commands about a board take an optional `board` (an id, or a kind), the
//! board the UI shows without one.
//!
//! Closing the window only hides it; the tray icon brings it back or quits, and
//! launching the app again shows the running instance, unless a newer build
//! was installed meanwhile: then that one takes over (see `instances`).
//!
//! Identifying and flashing the board go through esptool, which the app sets
//! up by itself on first use (Python + virtualenv in its data folder); the
//! session on that port is paused meanwhile so esptool can own it, while the
//! other boards carry on.
//!
//! For the Claude faces the app can also point Claude Code's status line at
//! itself (`main.rs` handles that invocation); see `companion_core::claude`.
//! Likewise Claude Code's hooks, for the alerts: Claude needs you, it's done,
//! the limits are running out (`companion_core::claude::alerts`).
//!
//! Voice (opt-in, off by default): with it on, what the board hears after its
//! wake word is transcribed by a whisper.cpp `whisper-server` sidecar with a
//! model the app downloads (`companion_core::models`), understood by a small
//! language model in a llama.cpp `llama-server` sidecar that calls the
//! board's tools (`companion_core::agent`; without one, a few fixed phrases,
//! `companion_core::intents`) and answered out loud by a Piper sidecar
//! through the board's speaker. The app installs Piper itself, into a
//! virtualenv made with esptool's Python. Turning voice on, or picking
//! another model, language or voice, swaps the bridge's shared voice config:
//! no reconnect.
//!
//! The same binary with `--mcp` is an MCP server for Claude Code and Claude
//! Desktop (`companion_core::mcp`). It reaches the board through this app's
//! [`Hub`], which lives as long as the app, across bridge restarts.
//!
//! Timers, reminders and a pomodoro (`companion_core::timers`) live here too,
//! in `timers.json` next to the hub file: the bridge ticks them and the board
//! counts the first one down and rings; the Timers tab, the voice and MCP
//! clients set them.
//!
//! On macOS the app can install a root helper that reads the exact CPU power,
//! which macOS 27 hides from ordinary apps (`power_helper`).

mod instances;
mod power_helper;

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use companion_core::agent;
use companion_core::claude::alerts::AlertSettings;
use companion_core::quips::QuipLevel;
use companion_core::claude::hooks::{self, HooksStatus};
use companion_core::claude::mcpconfig;
use companion_core::claude::statusline::{self, LinkStatus};
use companion_core::flasher::setup;
use companion_core::hardware::{self, Hardware, Recommendation};
use companion_core::cloud::elevenlabs::{self, AccountVoice};
use companion_core::cloud::{self, CloudError, CloudRef, KeySource, Provider};
use companion_core::llm::{self, Llm, LlmConfig, LocalLlm};
use companion_core::models::{self, Kind, Model};
use companion_core::music::{self, Music};
use companion_core::pointer;
use companion_core::stt::{self, Stt, SttConfig, SttEngine, SttLanguage, Transcript};
use companion_core::tts::{self, Engine, Tts, TtsConfig};
use companion_core::timers::{self, Pomodoro, ShowOn, TimerInfo, Timers};
use companion_core::voice::{self, Spoken, VoiceConfig};
use companion_core::{
    Agent, Board, BoardFirmware, BoardKind, BridgeConfig, BridgeEvent, Manager as BoardManager, ManagerEvent, ScreenSettings, ChipInfo, Collector, Esptool, Faces, FlashEvent, Hub, HubStatus, ImageInfo, PortInfo, Reading, Rotations, Snapshot, firmware,
    Transport, keep_warm, mcp, media, serial, wifi,
};
use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

const LOG_LINES: usize = 300;
const TRANSCRIPTS: usize = 50;
const TRAY_ID: &str = "main";
/// Each board's merged firmware image, shipped inside the app: copied into
/// `firmware/<kind>/` from the board's own repository (`tools/fetch_firmware.sh`).
const FIRMWARE_DUALEYE: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../firmware/dualeye/merged-binary.bin"));
const FIRMWARE_YORU: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../firmware/yoru/merged-binary.bin"));

/// The firmware image the app flashes on a `kind` board.
fn firmware_for(kind: BoardKind) -> &'static [u8] {
    match kind {
        BoardKind::DualEye => FIRMWARE_DUALEYE,
        BoardKind::Yoru => FIRMWARE_YORU,
    }
}

#[derive(Default, Clone, Serialize, Deserialize)]
struct Settings {
    /// `None` auto-detects the board.
    port: Option<String>,
    /// USB, WiFi, or USB when plugged in and WiFi otherwise.
    #[serde(default)]
    transport: Transport,
    #[serde(default)]
    faces: Faces,
    #[serde(default)]
    rotation: Rotations,
    /// Each DualEye's own faces and rotation, by id; `faces` and `rotation`
    /// are for one without.
    #[serde(default)]
    boards: BTreeMap<String, ScreenSettings>,
    /// The id of the board that says alerts and quips, and that the UI and
    /// hub clients reach when several could.
    #[serde(default)]
    primary: Option<String>,
    #[serde(default)]
    voice: VoiceSettings,
    #[serde(default)]
    claude_alerts: AlertSettings,
    /// The eyes face looks about on its own instead of at the mouse pointer.
    #[serde(default)]
    ignore_pointer: bool,
    /// How often the pet comments on your day out loud.
    #[serde(default)]
    quips: QuipLevel,
    /// The app's language (`en`, `it`); `None` follows the system's.
    #[serde(default)]
    ui_language: Option<String>,
    /// The first-run setup is behind us: done, skipped, or not needed. A
    /// settings file from before it existed is someone upgrading, who skips it.
    #[serde(default = "yes")]
    onboarding_done: bool,
    /// The last version of the tour seen (or declined): a newer one offers its new steps.
    #[serde(default)]
    tour_version: u32,
}

fn yes() -> bool {
    true
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct VoiceSettings {
    /// Transcribe what the board hears. Off until the person turns it on.
    enabled: bool,
    /// A `models::MODELS` id, or a cloud model's (`groq:whisper-large-v3`).
    model: String,
    language: SttLanguage,
    /// Keep each utterance as a WAV file (`voice/` in DualEye's data folder).
    keep_recordings: bool,
    /// Answer out loud through the board's speaker.
    speak: bool,
    /// After a spoken answer, listen a few seconds more without the wake word.
    follow_up: bool,
    /// Pause the music playing on this computer while the board listens.
    pause_music: bool,
    /// Keep the downloaded models loaded, those standing in for cloud ones
    /// too, and start again any that stops: the first answer doesn't wait.
    keep_warm: bool,
    /// Voice by language (`it`, `en`): `models::MODELS` ids, or cloud ones.
    voices: BTreeMap<String, String>,
    /// Understand commands with a local language model; without it, a few
    /// fixed phrases.
    llm: bool,
    /// A `models::MODELS` id of kind `llm`, or a cloud model's.
    llm_model: String,
    /// How the language model talks: a preset of `agent::personality_prompt`, or `custom`.
    personality: String,
    /// The person's own description, for `custom`.
    personality_custom: String,
}

/// This computer and the models for it, with what llama-server says it can
/// run on once [`check_hardware`] has asked it; until then without.
fn hardware() -> (Hardware, Recommendation) {
    let hw = CHECKED.get().cloned().unwrap_or_else(Hardware::detect_quick);
    let rec = hardware::recommend(&hw);
    (hw, rec)
}

static CHECKED: OnceLock<Hardware> = OnceLock::new();

/// Ask llama-server for its devices in the background: the first time on a
/// Mac it takes seconds.
fn check_hardware() {
    let _ = thread::Builder::new().name("dualeye-hardware".into()).spawn(|| {
        let _ = CHECKED.set(Hardware::detect());
    });
}

impl Default for VoiceSettings {
    fn default() -> Self {
        let (_, rec) = hardware();
        let voices = ["it", "en"].into_iter().filter_map(|l| models::default_voice(l).map(|v| (l.to_string(), v.to_string()))).collect();
        Self {
            enabled: false,
            model: rec.whisper.into(),
            language: SttLanguage::Auto,
            keep_recordings: false,
            speak: true,
            follow_up: true,
            pause_music: true,
            keep_warm: true,
            voices,
            llm: rec.llm.is_some(),
            llm_model: rec.llm.unwrap_or(models::DEFAULT_LLM).into(),
            personality: "cute".into(),
            personality_custom: String::new(),
        }
    }
}

/// What the host did about a transcript.
#[derive(Clone, Serialize)]
struct ReplyEntry {
    text: String,
    actions: Vec<String>,
    understood: bool,
    /// `llm` or `rules`.
    by: String,
}

/// One line of the Voice tab's log.
#[derive(Clone, Serialize)]
struct TranscriptEntry {
    id: u8,
    /// Unix time in ms.
    at: u64,
    /// `None`: Whisper heard no words.
    transcript: Option<Transcript>,
    reply: Option<ReplyEntry>,
    spoken: Option<Spoken>,
}

#[derive(Default)]
struct Link {
    kind: &'static str,
    /// What the session opened (a serial port, or `wifi:<id>`); `None` for the host's.
    target: Option<String>,
    /// The board's id and kind, once its `hello` said.
    id: Option<String>,
    board_kind: Option<BoardKind>,
    port: Option<String>,
    message: Option<String>,
    last: Option<Snapshot>,
    sent: Option<Snapshot>,
    sent_at: Option<Instant>,
    connected_at: Option<Instant>,
    /// What the board said it runs, since it connected.
    firmware: Option<BoardFirmware>,
    logs: VecDeque<String>,
    /// The latest Claude alert: what it said, why it didn't reach the board, when (Unix ms).
    last_alert: Option<(String, Option<String>, u64)>,
    /// The pet's latest comment on your day, likewise.
    last_quip: Option<QuipEntry>,
    /// What the board's voice overlay shows (`idle` until it says otherwise).
    voice: Option<String>,
    transcripts: VecDeque<TranscriptEntry>,
}

impl Link {
    fn record(&mut self, event: &BridgeEvent) {
        match event {
            BridgeEvent::Waiting { reason } => {
                self.kind = "searching";
                self.message = Some(reason.clone());
            }
            BridgeEvent::Connected { port } => {
                self.kind = "connected";
                self.port = Some(port.clone());
                self.message = None;
                self.sent = None;
                self.firmware = None;
                self.voice = None;
                self.connected_at = Some(Instant::now());
            }
            BridgeEvent::Snapshot { snapshot, sent } => {
                self.last = Some(snapshot.clone());
                if *sent {
                    self.sent = Some(snapshot.clone());
                    self.sent_at = Some(Instant::now());
                }
            }
            BridgeEvent::BoardLog { line } => {
                if self.logs.len() == LOG_LINES {
                    self.logs.pop_front();
                }
                self.logs.push_back(line.clone());
            }
            BridgeEvent::Firmware { firmware } => self.firmware = Some(firmware.clone()),
            // `AppState` keeps faces and rotation, and the timers.
            BridgeEvent::Settings { .. } | BridgeEvent::Wake { .. } | BridgeEvent::TimerFired { .. } => {}
            BridgeEvent::Transcript { id, transcript } => {
                if self.transcripts.len() == TRANSCRIPTS {
                    self.transcripts.pop_front();
                }
                let at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
                self.transcripts.push_back(TranscriptEntry { id: *id, at, transcript: transcript.clone(), reply: None, spoken: None });
            }
            BridgeEvent::Reply { id, text, actions, understood, by, .. } => {
                if let Some(entry) = self.transcripts.iter_mut().rev().find(|e| e.id == *id) {
                    entry.reply = Some(ReplyEntry { text: text.clone(), actions: actions.clone(), understood: *understood, by: by.clone() });
                }
            }
            BridgeEvent::Spoken { id, spoken } => {
                if let Some(entry) = self.transcripts.iter_mut().rev().find(|e| e.id == *id) {
                    entry.spoken = Some(spoken.clone());
                }
            }
            // `AppState::stt_error` keeps the last error.
            BridgeEvent::Listening { .. } | BridgeEvent::Utterance { .. } | BridgeEvent::VoiceError { .. } => {}
            BridgeEvent::VoiceState { state } => self.voice = Some(state.clone()),
            BridgeEvent::ClaudeAlert { text, error, .. } => {
                let at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
                self.last_alert = Some((text.clone(), error.clone(), at));
            }
            BridgeEvent::Quip { quip, text, error } => {
                let at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
                self.last_quip = Some(QuipEntry { topic: quip.topic.name(), text: text.clone(), error: error.clone(), at });
            }
            BridgeEvent::Disconnected { reason, .. } => {
                self.kind = "offline";
                self.message = Some(reason.clone());
            }
        }
    }
}

/// Each board's [`Link`], and the host's.
#[derive(Default)]
struct Devices {
    /// By the session's target.
    boards: BTreeMap<String, Link>,
    /// The host's own events: no board found, alerts, quips.
    host: Link,
    /// The target of the board picked in the UI's switcher, while it's there.
    shown: Option<String>,
}

impl Devices {
    fn record(&mut self, event: &ManagerEvent) {
        let Some(target) = &event.target else {
            self.host.record(&event.event);
            return;
        };
        if let BridgeEvent::Disconnected { .. } = &event.event {
            self.boards.remove(target);
            // A UI that knows one board shows it gone once none is left.
            if self.boards.is_empty() {
                self.host.record(&event.event);
            }
            return;
        }
        let link = self.boards.entry(target.clone()).or_insert_with(|| Link { kind: "searching", ..Default::default() });
        link.target = Some(target.clone());
        link.id = event.board.clone().or(link.id.take());
        link.board_kind = event.board_kind.or(link.board_kind);
        link.record(&event.event);
    }

    /// The board named by its id or its session's target.
    fn find(&self, board: &str) -> Option<&Link> {
        self.boards.get(board).or_else(|| self.boards.values().find(|l| l.id.as_deref() == Some(board)))
    }

    /// The board the UI shows: the one picked, else the primary one, else
    /// the first connected. A board picked that is gone (unplugged, or being
    /// flashed) is waited for, not swapped for another.
    fn current(&self, primary: Option<&str>) -> Option<&Link> {
        if let Some(target) = &self.shown {
            return self.boards.get(target);
        }
        let connected = || self.boards.values().filter(|l| l.kind == "connected");
        primary
            .and_then(|p| connected().find(|l| l.id.as_deref() == Some(p)))
            .or_else(|| connected().min_by_key(|l| l.connected_at))
            .or_else(|| self.boards.values().next())
    }

    fn infos(&self, primary: Option<&str>) -> Vec<DeviceInfo> {
        let current = self.current(primary).and_then(|l| l.target.clone());
        self.boards
            .values()
            .map(|l| DeviceInfo {
                target: l.target.clone().unwrap_or_default(),
                id: l.id.clone(),
                kind: l.board_kind,
                link: l.kind,
                port: l.port.clone(),
                firmware: l.firmware.clone(),
                primary: primary.is_some() && l.id.as_deref() == primary,
                current: l.target == current,
            })
            .collect()
    }
}

/// One board, for the device switcher.
#[derive(Clone, Serialize)]
struct DeviceInfo {
    target: String,
    id: Option<String>,
    kind: Option<BoardKind>,
    /// `searching` until its `hello`, then `connected`.
    link: &'static str,
    port: Option<String>,
    firmware: Option<BoardFirmware>,
    primary: bool,
    /// The board the UI shows and commands without `board` reach.
    current: bool,
}

/// What the pet said about your day, and when (Unix ms).
#[derive(Clone, Serialize)]
struct QuipEntry {
    topic: &'static str,
    text: String,
    error: Option<String>,
    at: u64,
}

struct AppState {
    devices: Mutex<Devices>,
    manager: Mutex<Option<BoardManager>>,
    settings: Mutex<Settings>,
    settings_path: Option<PathBuf>,
    /// Handed to every bridge, so a face change reaches the running one.
    faces: Arc<Mutex<Faces>>,
    /// Likewise for the rotation of each screen.
    rotation: Arc<Mutex<Rotations>>,
    /// Each DualEye's own faces and rotation, handed to the bridge like `faces`.
    board_settings: Arc<Mutex<BTreeMap<String, ScreenSettings>>>,
    /// The primary board's id, handed to the bridge like `faces`.
    primary: Arc<Mutex<Option<String>>>,
    /// Shares the bridge's connection with MCP servers; `None` if it couldn't listen.
    hub: Option<Arc<Hub>>,
    hub_error: Option<String>,
    /// Separate from the bridge's own collector, for the sensor list.
    collector: Mutex<Option<Collector>>,
    /// esptool holds the port (identify or flash in progress).
    device_busy: AtomicBool,
    /// Where esptool's Python and virtualenv live.
    esptool_dir: PathBuf,
    /// Handed to every bridge; `apply_voice` swaps what's in it.
    voice: Arc<Mutex<VoiceConfig>>,
    /// Handed to every bridge, like `faces`.
    claude_alerts: Arc<Mutex<AlertSettings>>,
    /// Handed to every bridge; kept in their own file.
    timers: Arc<Timers>,
    /// Handed to every bridge: what's playing, and its cover for the mirror.
    music: Arc<Music>,
    /// Handed to every bridge, like `faces`.
    follow_pointer: Arc<AtomicBool>,
    /// Handed to every bridge, like `faces`.
    quips: Arc<Mutex<QuipLevel>>,
    /// Speech-to-text: `off`, `starting`, `ready` or `error`, and why.
    stt_state: Arc<Mutex<(&'static str, Option<String>)>>,
    /// Text-to-speech, likewise.
    tts_state: Arc<Mutex<(&'static str, Option<String>)>>,
    /// The language model, likewise.
    llm_state: Arc<Mutex<(&'static str, Option<String>)>>,
    /// The text-to-speech engines being installed: the latest line of their output.
    installing: Mutex<BTreeMap<Engine, String>>,
    /// The model being downloaded and how far along (percent).
    download: Mutex<Option<(String, f32)>>,
    cancel_download: AtomicBool,
}

impl AppState {
    /// The id of the board the UI shows, when it has one.
    fn current_id(&self) -> Option<String> {
        let primary = self.primary.lock().unwrap().clone();
        self.devices.lock().unwrap().current(primary.as_deref()).and_then(|l| l.id.clone())
    }

    /// The board a command reaches: `board` (an id, a kind, or a session's
    /// target), else the one the UI shows.
    fn board(&self, board: Option<String>) -> Board {
        let by_target = |b: &str| self.devices.lock().unwrap().boards.get(b).and_then(|l| l.id.clone());
        let board = board.map(|b| by_target(&b).unwrap_or(b));
        self::board().on(board.or_else(|| self.current_id()))
    }

    /// What DualEye `id` shows: its own faces and rotation, else the shared ones.
    fn screens(&self, id: Option<&str>) -> ScreenSettings {
        id.and_then(|id| self.board_settings.lock().unwrap().get(id).copied())
            .unwrap_or_else(|| ScreenSettings { faces: *self.faces.lock().unwrap(), rotation: *self.rotation.lock().unwrap() })
    }

    fn set_screens(&self, id: Option<&str>, screens: ScreenSettings) {
        match id {
            Some(id) => {
                self.board_settings.lock().unwrap().insert(id.to_string(), screens);
                self.settings.lock().unwrap().boards.insert(id.to_string(), screens);
            }
            None => {
                *self.faces.lock().unwrap() = screens.faces;
                *self.rotation.lock().unwrap() = screens.rotation;
                let mut settings = self.settings.lock().unwrap();
                settings.faces = screens.faces;
                settings.rotation = screens.rotation;
            }
        }
        self.save_settings();
    }

    fn save_settings(&self) {
        let Some(path) = &self.settings_path else { return };
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let settings = self.settings.lock().unwrap().clone();
        if let Ok(json) = serde_json::to_string_pretty(&settings) {
            let _ = fs::write(path, json);
        }
    }
}

/// Make the bridge's voice config match the settings: start (or keep)
/// the whisper-server, Piper and llama-server sidecars, or stop them.
fn apply_voice(state: &AppState) {
    let settings = state.settings.lock().unwrap().voice.clone();
    let (current_stt, current_tts, current_agent) = {
        let voice = state.voice.lock().unwrap();
        (voice.stt.clone(), voice.tts.clone(), voice.agent.clone())
    };
    let stt = sidecar(
        settings.enabled.then(|| stt_config(&settings)),
        current_stt.filter(|s| settings.enabled && s.is_kept_warm() == settings.keep_warm && stt_config(&settings).is_ok_and(|c| s.config() == &c)),
        &state.stt_state,
        // A cloud model's limits are answered by a downloaded one.
        |config| warm(Stt::new(config).with_fallback(stt::local_fallback()).kept_warm(settings.keep_warm), Stt::rewarm),
        |stt: &Arc<Stt>| stt.warm_up().map_err(|e| e.to_string()),
    );
    let wanted_tts = (settings.enabled && settings.speak).then(|| tts_config(&settings));
    let keep_tts = current_tts.filter(|t| t.is_kept_warm() == settings.keep_warm && matches!(&wanted_tts, Some(Ok(c)) if t.config() == c));
    let tts = sidecar(
        wanted_tts,
        keep_tts,
        &state.tts_state,
        |config| warm(Tts::new(config).kept_warm(settings.keep_warm), Tts::rewarm),
        |tts: &Arc<Tts>| tts.warm_up().map_err(|e| e.to_string()),
    );
    let wanted_llm = (settings.enabled && settings.llm).then(|| llm_config(&settings));
    let keep_agent = current_agent.filter(|a| a.llm().is_kept_warm() == settings.keep_warm && matches!(&wanted_llm, Some(Ok(c)) if a.llm().config() == c));
    let agent = sidecar(
        wanted_llm,
        keep_agent,
        &state.llm_state,
        |config| Arc::new(Agent::new(warm(Llm::new(config).with_fallback(llm::local_fallback()).kept_warm(settings.keep_warm), Llm::rewarm))),
        |agent: &Arc<Agent>| agent.llm().warm_up().map_err(|e| e.to_string()),
    );
    if let Some(agent) = &agent {
        agent.set_personality(&agent::personality_prompt(&settings.personality, &settings.personality_custom));
    }
    let dump_dir = settings.keep_recordings.then(voice::default_dump_dir).flatten();
    *state.voice.lock().unwrap() = VoiceConfig { dump_dir, stt, tts, agent, follow_up: settings.follow_up, pause_music: settings.pause_music };
}

/// `sidecar` shared, with its servers started again whenever they stop
/// while it's [kept warm](Stt::kept_warm).
fn warm<T: Send + Sync + 'static, E: 'static>(sidecar: T, rewarm: fn(&T) -> Result<(), E>) -> Arc<T> {
    let sidecar = Arc::new(sidecar);
    keep_warm(&sidecar, move |s| drop(rewarm(s)));
    sidecar
}

/// One sidecar for [`apply_voice`]: off (`wanted` is `None`), not possible
/// (`Some(Err)`), the running one when `keep` has it, or a new one warmed up
/// in the background. `status` follows.
fn sidecar<C, T: Send + Sync + 'static>(
    wanted: Option<Result<C, String>>,
    keep: Option<Arc<T>>,
    status: &Arc<Mutex<(&'static str, Option<String>)>>,
    make: impl FnOnce(C) -> Arc<T>,
    warm_up: impl FnOnce(&Arc<T>) -> Result<(), String> + Send + 'static,
) -> Option<Arc<T>> {
    match wanted {
        None => {
            *status.lock().unwrap() = ("off", None);
            None
        }
        Some(Err(why)) => {
            *status.lock().unwrap() = ("error", Some(why));
            None
        }
        Some(Ok(_)) if keep.is_some() => keep,
        Some(Ok(config)) => {
            let sidecar = make(config);
            *status.lock().unwrap() = ("starting", None);
            let (warm, status) = (sidecar.clone(), status.clone());
            thread::spawn(move || {
                *status.lock().unwrap() = match warm_up(&warm) {
                    Ok(()) => ("ready", None),
                    Err(e) => ("error", Some(e)),
                }
            });
            Some(sidecar)
        }
    }
}

fn tts_config(settings: &VoiceSettings) -> Result<TtsConfig, String> {
    let voices: BTreeMap<String, String> = settings
        .voices
        .iter()
        // A voice that's no longer offered (Kokoro's): the language's default.
        .map(|(l, id)| (l, if Model::by_id(id).is_some() || CloudRef::by_id(id).is_some() { id.as_str() } else { models::default_voice(l).unwrap_or(id) }))
        .filter(|(_, id)| Model::by_id(id).is_some_and(|m| m.kind == Kind::Voice && m.is_installed()) || CloudRef::by_id(id).is_some_and(|r| r.model.kind == Kind::Voice))
        .map(|(l, id)| (l.clone(), id.to_string()))
        .collect();
    if voices.is_empty() {
        return Err("no voice downloaded yet".into());
    }
    let config = TtsConfig::new(tts::installed_engines(), voices)?;
    if let Some(e) = config.engines().find(|e| !config.pythons.contains_key(e)) {
        return Err(format!("{} isn't installed yet", e.name()));
    }
    Ok(config)
}

fn llm_config(settings: &VoiceSettings) -> Result<LlmConfig, String> {
    if cloud::is_cloud_id(&settings.llm_model) {
        return cloud::Client::new(&settings.llm_model, Kind::Llm).map(LlmConfig::Cloud);
    }
    let server = llm::find_server().ok_or("llama-server not found: reinstall the app, or install llama.cpp (brew install llama.cpp on macOS)")?;
    // A model that's no longer offered (qwen3-4b-2507): the default.
    let model = Model::by_id(&settings.llm_model).or_else(|| Model::by_id(models::DEFAULT_LLM)).filter(|m| m.kind == Kind::Llm).ok_or_else(|| format!("unknown language model {}", settings.llm_model))?;
    if !model.is_installed() {
        return Err(format!("the {} model isn't downloaded yet", model.id));
    }
    Ok(LlmConfig::Local(LocalLlm { server, model: model.path().ok_or("no data folder for the models")?, gpu_layers: None }))
}

fn stt_config(settings: &VoiceSettings) -> Result<SttConfig, String> {
    if cloud::is_cloud_id(&settings.model) {
        return Ok(SttConfig { engine: SttEngine::Cloud(cloud::Client::new(&settings.model, Kind::Whisper)?), language: settings.language });
    }
    let server = stt::find_server().ok_or("whisper-server not found: reinstall the app, or install whisper.cpp (brew install whisper-cpp on macOS)")?;
    let model = Model::by_id(&settings.model).ok_or_else(|| format!("unknown model {}", settings.model))?;
    if !model.is_installed() {
        return Err(format!("the {} model isn't downloaded yet", model.id));
    }
    Ok(SttConfig { engine: SttEngine::Local { server, model: model.path().ok_or("no data folder for the models")? }, language: settings.language })
}

#[derive(Serialize)]
struct ModelInfo {
    id: String,
    kind: Kind,
    language: Option<&'static str>,
    /// What speaks a voice.
    engine: Option<Engine>,
    bytes: u64,
    note: &'static str,
    license: &'static str,
    /// Downloaded; for a cloud model, its provider has a key.
    installed: bool,
    /// The [`cloud::Provider`] that runs it, for a cloud model.
    provider: Option<&'static str>,
}

#[derive(Serialize)]
struct ProviderInfo {
    id: &'static str,
    name: &'static str,
    note: &'static str,
    keys_url: &'static str,
    key_env: &'static str,
    /// Where its key comes from, if it has one.
    key: Option<KeySource>,
}

#[derive(Serialize)]
struct VoiceInfo {
    settings: VoiceSettings,
    /// The whisper-server binary, if found.
    server: Option<String>,
    models: Vec<ModelInfo>,
    stt: &'static str,
    stt_error: Option<String>,
    /// Piper's virtualenv interpreter, if installed.
    piper: Option<String>,
    /// While Piper installs: the latest line of its output.
    piper_install: Option<String>,
    tts: &'static str,
    tts_error: Option<String>,
    /// Why the downloaded voice is answering instead of the cloud one, while it is.
    tts_fallback: Option<String>,
    /// The llama-server binary, if found.
    llm_server: Option<String>,
    llm: &'static str,
    llm_error: Option<String>,
    download: Option<(String, f32)>,
    /// The online services that can run the models instead.
    providers: Vec<ProviderInfo>,
    hardware: Hardware,
    recommendation: Recommendation,
}

fn voice_info_of(state: &AppState) -> VoiceInfo {
    let (hardware, recommendation) = hardware();
    let (stt, stt_error) = state.stt_state.lock().unwrap().clone();
    let (tts, tts_error) = state.tts_state.lock().unwrap().clone();
    let (llm, llm_error) = state.llm_state.lock().unwrap().clone();
    let installing = state.installing.lock().unwrap().clone();
    VoiceInfo {
        settings: state.settings.lock().unwrap().voice.clone(),
        server: stt::find_server().map(|p| p.display().to_string()),
        models: models::MODELS
            .iter()
            .map(|m| ModelInfo { id: m.id.into(), kind: m.kind, language: m.language, engine: m.engine(), bytes: m.bytes(), note: m.note, license: m.license, installed: m.is_installed(), provider: None })
            .chain(cloud_models())
            .collect(),
        stt,
        stt_error,
        piper: Engine::Piper.python().map(|p| p.display().to_string()),
        piper_install: installing.get(&Engine::Piper).cloned(),
        tts,
        tts_error,
        tts_fallback: state.voice.lock().unwrap().tts.as_ref().and_then(|t| t.cloud_trouble()),
        llm_server: llm::find_server().map(|p| p.display().to_string()),
        llm,
        llm_error,
        download: state.download.lock().unwrap().clone(),
        providers: cloud::PROVIDERS
            .iter()
            .map(|p| ProviderInfo { id: p.id, name: p.name, note: p.note, keys_url: p.keys_url, key_env: p.key_env, key: p.key_source().map(|(_, s)| s) })
            .collect(),
        hardware,
        recommendation,
    }
}

/// The cloud models, after the local ones of each kind.
fn cloud_models() -> Vec<ModelInfo> {
    [Kind::Whisper, Kind::Llm, Kind::Voice]
        .into_iter()
        .flat_map(CloudRef::of_kind)
        .map(|r| {
            let installed = r.provider.api_key().is_some();
            ModelInfo { id: r.id(), kind: r.model.kind, language: r.model.language, engine: None, bytes: 0, note: r.model.note, license: "", installed, provider: Some(r.provider.id) }
        })
        .collect()
}

/// Save a provider's API key (`None` or empty forgets it), check it, and
/// start what was waiting for it.
#[tauri::command]
async fn set_api_key(app: AppHandle, provider: String, key: Option<String>) -> Result<VoiceInfo, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let p = Provider::by_id(&provider).ok_or_else(|| format!("unknown provider {provider}"))?;
        cloud::set_api_key(p, key.as_deref()).map_err(|e| e.to_string())?;
        // Something free of it: it only proves the key works.
        if key.as_deref().is_some_and(|k| !k.trim().is_empty()) {
            p.check_key().map_err(|e| e.to_string())?;
        }
        apply_voice(&state);
        Ok(voice_info_of(&state))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Whether a provider's key works: `valid`, `invalid` (refused or missing) or
/// `unknown` (it couldn't be asked), with why.
#[derive(Serialize)]
struct KeyCheck {
    state: &'static str,
    error: Option<String>,
}

#[tauri::command]
async fn check_api_key(provider: String) -> Result<KeyCheck, String> {
    let p = Provider::by_id(&provider).ok_or_else(|| format!("unknown provider {provider}"))?;
    tauri::async_runtime::spawn_blocking(move || match p.check_key() {
        // A limit reached still means the key was taken.
        Ok(()) | Err(CloudError::RateLimited { .. }) => KeyCheck { state: "valid", error: None },
        Err(e @ (CloudError::BadKey { .. } | CloudError::NoKey { .. })) => KeyCheck { state: "invalid", error: Some(e.to_string()) },
        Err(e) => KeyCheck { state: "unknown", error: Some(e.to_string()) },
    })
    .await
    .map_err(|e| e.to_string())
}

/// Open the page where a provider gives out API keys, in the browser.
#[tauri::command]
fn open_key_page(provider: String) -> Result<(), String> {
    let p = Provider::by_id(&provider).ok_or_else(|| format!("unknown provider {provider}"))?;
    open_in_browser(p.keys_url)
}

/// Only the app's own fixed addresses come here, never one from outside.
fn open_in_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(windows)]
    let mut cmd = {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(url).spawn().map(drop).map_err(|e| e.to_string())
}

/// The voices the ElevenLabs account can speak with: its own, then ElevenLabs'.
#[tauri::command]
async fn elevenlabs_voices() -> Result<Vec<AccountVoice>, String> {
    tauri::async_runtime::spawn_blocking(|| elevenlabs::voices().map_err(|e| e.to_string())).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn voice_info(state: State<AppState>) -> VoiceInfo {
    voice_info_of(&state)
}

#[tauri::command]
fn set_voice(state: State<AppState>, settings: VoiceSettings) -> VoiceInfo {
    state.settings.lock().unwrap().voice = settings;
    state.save_settings();
    apply_voice(&state);
    voice_info_of(&state)
}

#[tauri::command]
async fn download_model(app: AppHandle, id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let model = Model::by_id(&id).ok_or_else(|| format!("unknown model {id}"))?;
        {
            let mut download = state.download.lock().unwrap();
            if download.is_some() {
                return Err("another model is downloading".to_string());
            }
            *download = Some((id.clone(), 0.0));
        }
        state.cancel_download.store(false, Ordering::SeqCst);
        let result = model.download(&state.cancel_download, |p| *state.download.lock().unwrap() = Some((id.clone(), p.unwrap_or(0.0))));
        *state.download.lock().unwrap() = None;
        result.map_err(|e| e.to_string())?;
        // The model voice was waiting for.
        apply_voice(&state);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn cancel_download(state: State<AppState>) {
    state.cancel_download.store(true, Ordering::SeqCst);
}

#[tauri::command]
fn delete_model(state: State<AppState>, id: String) -> Result<VoiceInfo, String> {
    let model = Model::by_id(&id).ok_or_else(|| format!("unknown model {id}"))?;
    {
        // Stop the server that has it open first.
        let mut voice = state.voice.lock().unwrap();
        if voice.stt.as_ref().is_some_and(|s| s.config().engine.model_path() == model.path().as_deref()) {
            voice.stt = None;
        }
        if voice.tts.as_ref().is_some_and(|t| t.config().voices.values().any(|v| v == model.id)) {
            voice.tts = None;
        }
        if let Some(agent) = voice.agent.take_if(|a| a.llm().config().model_path() == model.path().as_deref()) {
            agent.llm().shutdown();
        }
    }
    model.remove().map_err(|e| e.to_string())?;
    apply_voice(&state);
    Ok(voice_info_of(&state))
}

/// Set a text-to-speech engine up: a virtualenv made with esptool's Python
/// (downloaded first if the machine has none), then `pip install` it.
#[tauri::command]
async fn install_engine(app: AppHandle, engine: Engine) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        {
            let mut installing = state.installing.lock().unwrap();
            if installing.contains_key(&engine) {
                return Err(format!("{} is already being installed", engine.name()));
            }
            installing.insert(engine, "Getting Python".into());
        }
        let progress = |line: String| {
            state.installing.lock().unwrap().insert(engine, line);
        };
        let on_event = |e| {
            if let FlashEvent::Setup { message, percent } = e {
                progress(percent.map_or(message.clone(), |p| format!("{message} {p:.0}%")));
            }
        };
        let result = setup::python(&state.esptool_dir, on_event).and_then(|python| tts::install(engine, &python, progress));
        state.installing.lock().unwrap().remove(&engine);
        result.map_err(|e| e.to_string())?;
        apply_voice(&state);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn board() -> Board {
    Board::new(None, &format!("companion-hub/{}", env!("CARGO_PKG_VERSION")))
}

/// The board's own voice settings (it keeps them in NVS); `None` where it
/// doesn't say (`eyes`: a firmware before 1.0.1; `idle_eyes`: before 1.0.2;
/// `wake_sound`: before 1.3.3; `pet_sounds`: before 1.4.0).
#[derive(serde::Serialize)]
struct BoardVoice {
    volume: Option<u8>,
    eyes: Option<bool>,
    idle_eyes: Option<bool>,
    wake_sound: Option<bool>,
    pet_sounds: Option<bool>,
}

#[tauri::command]
async fn board_voice(state: State<'_, AppState>, board: Option<String>) -> Result<BoardVoice, String> {
    let board = state.board(board);
    tauri::async_runtime::spawn_blocking(move || {
        let state = board.call_tool("get_state", serde_json::json!({})).map_err(|e| e.to_string())?;
        let s = state.structured_content.unwrap_or_default();
        Ok(BoardVoice {
            volume: s.pointer("/audio/volume").and_then(|v| v.as_u64()).map(|v| v as u8),
            eyes: s.pointer("/voice/eyes").and_then(|v| v.as_bool()),
            idle_eyes: s.pointer("/voice/idle_eyes").and_then(|v| v.as_bool()),
            wake_sound: s.pointer("/voice/wake_sound").and_then(|v| v.as_bool()),
            pet_sounds: s.pointer("/voice/pet_sounds").and_then(|v| v.as_bool()),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

fn call_board(board: &Board, name: &'static str, args: serde_json::Value) -> Result<(), String> {
    let result = board.call_tool(name, args).map_err(|e| e.to_string())?;
    if result.is_error { Err(result.text()) } else { Ok(()) }
}

/// Call tool `name` on `board` (an id or a kind), or on the board the UI shows.
async fn call_board_on(state: State<'_, AppState>, board: Option<String>, name: &'static str, args: serde_json::Value) -> Result<(), String> {
    let board = state.board(board);
    tauri::async_runtime::spawn_blocking(move || call_board(&board, name, args)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn set_board_volume(state: State<'_, AppState>, board: Option<String>, percent: u8) -> Result<(), String> {
    call_board_on(state, board, "set_volume", serde_json::json!({"percent": percent})).await
}

/// Animated eyes (or the ring) while the board talks with you.
#[tauri::command]
async fn set_board_eyes(state: State<'_, AppState>, board: Option<String>, on: bool) -> Result<(), String> {
    call_board_on(state, board, "set_eyes", serde_json::json!({"on": on})).await
}

/// The eyes' short scenes, now and then while nobody is talking.
#[tauri::command]
async fn set_board_idle_eyes(state: State<'_, AppState>, board: Option<String>, on: bool) -> Result<(), String> {
    call_board_on(state, board, "set_eyes", serde_json::json!({"idle": on})).await
}

/// A chime when the board hears the wake word.
#[tauri::command]
async fn set_board_wake_sound(state: State<'_, AppState>, board: Option<String>, on: bool) -> Result<(), String> {
    call_board_on(state, board, "set_mic", serde_json::json!({"wake_sound": on})).await
}

/// The board as a pet: its little sounds, its reactions to the computer.
#[tauri::command]
async fn set_board_pet(state: State<'_, AppState>, board: Option<String>, sounds: Option<bool>, react: Option<bool>, hear: Option<bool>) -> Result<(), String> {
    let mut args = serde_json::Map::new();
    if let Some(on) = sounds {
        args.insert("sounds".into(), on.into());
    }
    if let Some(on) = react {
        args.insert("react".into(), on.into());
    }
    if let Some(on) = hear {
        args.insert("hear".into(), on.into());
    }
    call_board_on(state, board, "set_pet", args.into()).await
}

/// The pet's mood, what's moving it and its last reactions (`get_pet`,
/// firmware 1.4); `None` on an older firmware, which doesn't have the tool.
#[tauri::command]
async fn board_pet(state: State<'_, AppState>, board: Option<String>) -> Result<Option<serde_json::Value>, String> {
    let board = state.board(board);
    tauri::async_runtime::spawn_blocking(move || match board.call_tool("get_pet", serde_json::json!({})) {
        Ok(result) if !result.is_error => Ok(result.structured_content),
        Ok(_) | Err(companion_core::CallError::Rpc { .. }) => Ok(None),
        Err(e) => Err(e.to_string()),
    })
    .await
    .map_err(|e| e.to_string())?
}

/// How often the pet comments on your day, and what it said last.
#[derive(Serialize)]
struct QuipsInfo {
    level: QuipLevel,
    last: Option<QuipEntry>,
}

#[tauri::command]
fn quips_info(state: State<AppState>) -> QuipsInfo {
    QuipsInfo { level: *state.quips.lock().unwrap(), last: state.devices.lock().unwrap().host.last_quip.clone() }
}

#[tauri::command]
fn set_quips(state: State<AppState>, level: QuipLevel) -> QuipsInfo {
    *state.quips.lock().unwrap() = level;
    state.settings.lock().unwrap().quips = level;
    state.save_settings();
    quips_info(state)
}

/// One of the eyes' scenes, now.
#[tauri::command]
async fn play_board_scene(state: State<'_, AppState>, board: Option<String>, name: String) -> Result<(), String> {
    call_board_on(state, board, "play_eyes", serde_json::json!({"name": name})).await
}

/// One of the board's own sounds, now.
#[tauri::command]
async fn play_board_sound(state: State<'_, AppState>, board: Option<String>, name: String) -> Result<(), String> {
    call_board_on(state, board, "play_sound", serde_json::json!({"name": name})).await
}

/// A Yoru's settings, as its `get_settings` gives them: each as
/// `{"value","min","max"}` under its name, plus `wake_word`.
#[tauri::command]
async fn yoru_settings(state: State<'_, AppState>, board: Option<String>) -> Result<serde_json::Value, String> {
    let board = state.board(board);
    tauri::async_runtime::spawn_blocking(move || {
        let result = board.call_tool("get_settings", serde_json::json!({})).map_err(|e| e.to_string())?;
        if result.is_error {
            return Err(result.text());
        }
        result.structured_content.ok_or_else(|| "the board sent no settings".to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Change one of a Yoru's settings, within its range (switches are 0 or 1).
#[tauri::command]
async fn yoru_set_setting(state: State<'_, AppState>, board: Option<String>, name: String, value: i64) -> Result<(), String> {
    call_board_on(state, board, "set_setting", serde_json::json!({"name": name, "value": value})).await
}

/// One of a Yoru's reactions, now.
#[tauri::command]
async fn yoru_react(state: State<'_, AppState>, board: Option<String>, name: String) -> Result<(), String> {
    call_board_on(state, board, "react", serde_json::json!({"name": name})).await
}

/// Open a Yoru's settings menu, on `page` if given, or close it.
#[tauri::command]
async fn yoru_show_menu(state: State<'_, AppState>, board: Option<String>, open: bool, page: Option<String>) -> Result<(), String> {
    let mut args = serde_json::json!({"open": open});
    if let Some(page) = page {
        args["page"] = page.into();
    }
    call_board_on(state, board, "show_menu", args).await
}

fn side(side: &str) -> Result<&'static str, String> {
    match side {
        "left" => Ok("left"),
        "right" => Ok("right"),
        other => Err(format!("unknown screen {other}")),
    }
}

/// Put a picture or a GIF on `side`'s image face, emitting `image`
/// `{side, progress}` as it goes.
#[tauri::command]
async fn send_image(app: AppHandle, state: State<'_, AppState>, board: Option<String>, side: String, bytes: Vec<u8>) -> Result<media::Prepared, String> {
    let side = self::side(&side)?;
    let board = state.board(board);
    tauri::async_runtime::spawn_blocking(move || {
        let mut last = -1.0f32;
        media::send(&board, side, &bytes, |p| {
            if p - last >= 0.02 || p >= 1.0 {
                last = p;
                let _ = app.emit("image", serde_json::json!({"side": side, "progress": p}));
            }
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn clear_image(state: State<'_, AppState>, board: Option<String>, side: String) -> Result<(), String> {
    let side = self::side(&side)?;
    let board = state.board(board);
    tauri::async_runtime::spawn_blocking(move || media::clear(&board, side)).await.map_err(|e| e.to_string())?
}

/// What was last sent to `side`, as a data URL for the mirror.
#[tauri::command]
async fn image_preview(side: String) -> Result<Option<String>, String> {
    let side = self::side(&side)?;
    tauri::async_runtime::spawn_blocking(move || media::copy_data_url(side)).await.map_err(|e| e.to_string())
}

/// Say a test sentence through the board, with the voice of `language`.
#[tauri::command]
async fn test_voice(state: State<'_, AppState>, board: Option<String>, language: String) -> Result<(), String> {
    let text = if language == "it" { "Ciao! Questa è la mia voce." } else { "Hello! This is my voice." };
    let board = state.board(board);
    tauri::async_runtime::spawn_blocking(move || board.say(text, Some(&language)).map(drop).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
struct Status {
    /// The board this is about: its id and kind, once its `hello` said.
    board: Option<String>,
    board_kind: Option<BoardKind>,
    /// Every board, for the switcher, and the primary one's id.
    boards: Vec<DeviceInfo>,
    primary: Option<String>,
    link: &'static str,
    port: Option<String>,
    message: Option<String>,
    last: Option<Snapshot>,
    sent: Option<Snapshot>,
    sent_age_ms: Option<u64>,
    connected_age_ms: Option<u64>,
    firmware: Option<BoardFirmware>,
    logs: Vec<String>,
    voice: Option<String>,
    transcripts: Vec<TranscriptEntry>,
    port_setting: Option<String>,
    faces: Faces,
    rotation: Rotations,
    follow_pointer: bool,
}

/// What the UI shows about `board` (an id or a target), or about the board it shows.
#[tauri::command]
fn status(state: State<AppState>, board: Option<String>) -> Status {
    let primary = state.primary.lock().unwrap().clone();
    let devices = state.devices.lock().unwrap();
    let link = board.as_deref().and_then(|b| devices.find(b)).or_else(|| devices.current(primary.as_deref())).unwrap_or(&devices.host);
    let screens = state.screens(link.id.as_deref());
    let age = |t: Option<Instant>| t.map(|t| t.elapsed().as_millis() as u64);
    Status {
        board: link.id.clone(),
        board_kind: link.board_kind,
        boards: devices.infos(primary.as_deref()),
        primary: primary.clone(),
        link: link.kind,
        port: link.port.clone(),
        message: link.message.clone(),
        last: link.last.clone(),
        sent: link.sent.clone(),
        sent_age_ms: age(link.sent_at),
        connected_age_ms: age(link.connected_at),
        firmware: link.firmware.clone(),
        logs: link.logs.iter().cloned().collect(),
        voice: link.voice.clone(),
        transcripts: link.transcripts.iter().cloned().collect(),
        port_setting: state.settings.lock().unwrap().port.clone(),
        faces: screens.faces,
        rotation: screens.rotation,
        follow_pointer: state.follow_pointer.load(Ordering::Relaxed),
    }
}

#[tauri::command]
fn list_ports() -> Vec<PortInfo> {
    serial::list_ports()
}

#[tauri::command]
async fn set_port(app: AppHandle, port: Option<String>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        state.settings.lock().unwrap().port = port.clone();
        state.save_settings();
        if state.device_busy.load(Ordering::SeqCst) {
            // The bridge comes back with the new setting once esptool is done.
            return;
        }
        restart_bridge(&app, port);
    })
    .await
    .map_err(|e| e.to_string())
}

/// Stop the bridge and start it again, with `port`.
fn restart_bridge(app: &AppHandle, port: Option<String>) {
    let state = app.state::<AppState>();
    // Dropping the old manager joins its threads and releases the ports.
    let old = state.manager.lock().unwrap().take();
    drop(old);
    *state.devices.lock().unwrap() = Devices { host: Link { kind: "searching", ..Default::default() }, ..Default::default() };
    let manager = start_bridge(app, port);
    *state.manager.lock().unwrap() = Some(manager);
}

/// USB, WiFi or both (`auto`): the bridge starts again the new way.
#[tauri::command]
async fn set_transport(app: AppHandle, transport: Transport) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let port = {
            let mut settings = state.settings.lock().unwrap();
            settings.transport = transport;
            settings.port.clone()
        };
        state.save_settings();
        if !state.device_busy.load(Ordering::SeqCst) {
            restart_bridge(&app, port);
        }
    })
    .await
    .map_err(|e| e.to_string())
}

/// A board this computer can reach on WiFi, without its key.
#[derive(Serialize)]
struct PairedBoard {
    id: String,
    kind: companion_core::BoardKind,
    ssid: Option<String>,
}

#[derive(Serialize)]
struct WifiInfo {
    transport: Transport,
    paired: Vec<PairedBoard>,
    /// What the board says about its WiFi (`wifi/status`); `None` when it
    /// doesn't answer, or its firmware predates WiFi (1.4.3), with `error`.
    board: Option<serde_json::Value>,
    error: Option<String>,
}

/// One `wifi/...` request to the board: through the bridge, or over USB.
fn wifi_call(board: &Board, method: &str, params: serde_json::Value, timeout: std::time::Duration) -> Result<serde_json::Value, String> {
    board.session(|call| call(method, params.clone(), timeout)).map_err(|e| match e {
        companion_core::CallError::Rpc { code: -32001, .. } => "plug the board in with the USB cable to change its WiFi".to_string(),
        companion_core::CallError::Rpc { code: -32601, .. } => "the board's firmware has no WiFi: update it to 1.4.3 or newer".to_string(),
        other => other.to_string(),
    })
}

const WIFI_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

#[tauri::command]
async fn wifi_info(app: AppHandle, board: Option<String>) -> Result<WifiInfo, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let transport = state.settings.lock().unwrap().transport;
        let paired = wifi::load_paired().into_iter().map(|b| PairedBoard { id: b.id, kind: b.kind, ssid: b.ssid }).collect();
        let (board, error) = match wifi_call(&state.board(board), "wifi/status", serde_json::json!({}), WIFI_TIMEOUT) {
            Ok(status) => (Some(status), None),
            Err(e) => (None, Some(e)),
        };
        WifiInfo { transport, paired, board, error }
    })
    .await
    .map_err(|e| e.to_string())
}

/// The networks the board sees, strongest first.
#[tauri::command]
async fn wifi_scan(state: State<'_, AppState>, board: Option<String>) -> Result<serde_json::Value, String> {
    let board = state.board(board);
    tauri::async_runtime::spawn_blocking(move || wifi_call(&board, "wifi/scan", serde_json::json!({}), companion_core::hub::SCAN_TIMEOUT))
        .await
        .map_err(|e| e.to_string())?
}

/// Keep the pairing key a `wifi/set` or `wifi/pair` answer carries, with the
/// board's kind: it picks the service and the handshake used on WiFi.
fn keep_key(board: &Board, reply: &serde_json::Value, ssid: Option<&str>) -> Result<(), String> {
    let (Some(id), Some(key)) = (reply["id"].as_str(), reply["key"].as_str()) else { return Ok(()) };
    let kind = board.kind().map_err(|e| format!("couldn't tell which board this is: {e}"))?;
    wifi::remember(id, key, ssid.or(reply["ssid"].as_str()), kind).map_err(|e| format!("couldn't keep the pairing key: {e}"))
}

/// Put the board on network `ssid` (over USB) and pair this computer with it.
#[tauri::command]
async fn wifi_setup(state: State<'_, AppState>, board: Option<String>, ssid: String, password: String) -> Result<serde_json::Value, String> {
    let board = state.board(board);
    tauri::async_runtime::spawn_blocking(move || {
        let reply = wifi_call(&board, "wifi/set", serde_json::json!({"ssid": ssid, "password": password}), WIFI_TIMEOUT)?;
        keep_key(&board, &reply, Some(&ssid))?;
        Ok(reply)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Turn the board's WiFi on or off, keeping its network.
#[tauri::command]
async fn wifi_enable(state: State<'_, AppState>, board: Option<String>, on: bool) -> Result<serde_json::Value, String> {
    let board = state.board(board);
    tauri::async_runtime::spawn_blocking(move || {
        let reply = wifi_call(&board, "wifi/set", serde_json::json!({"enabled": on}), WIFI_TIMEOUT)?;
        keep_key(&board, &reply, None)?;
        Ok(reply)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Make the board forget its network and its key: no computer reaches it on WiFi any more.
#[tauri::command]
async fn wifi_forget(state: State<'_, AppState>, board: Option<String>) -> Result<serde_json::Value, String> {
    let board = state.board(board);
    tauri::async_runtime::spawn_blocking(move || {
        let reply = wifi_call(&board, "wifi/forget", serde_json::json!({}), WIFI_TIMEOUT)?;
        if let Some(id) = reply["id"].as_str() {
            wifi::forget(id).map_err(|e| e.to_string())?;
        }
        Ok(reply)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Pair this computer with a board whose WiFi is set up already (by another computer).
#[tauri::command]
async fn wifi_pair(state: State<'_, AppState>, board: Option<String>) -> Result<(), String> {
    let board = state.board(board);
    tauri::async_runtime::spawn_blocking(move || {
        let reply = wifi_call(&board, "wifi/pair", serde_json::json!({}), WIFI_TIMEOUT)?;
        let status = wifi_call(&board, "wifi/status", serde_json::json!({}), WIFI_TIMEOUT).unwrap_or_default();
        keep_key(&board, &reply, status["ssid"].as_str())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The faces of DualEye `board` (an id), or of the one the UI shows.
#[tauri::command]
fn set_faces(state: State<AppState>, faces: Faces, board: Option<String>) {
    let id = board.or_else(|| state.current_id());
    let screens = state.screens(id.as_deref());
    state.set_screens(id.as_deref(), ScreenSettings { faces, ..screens });
}

#[tauri::command]
fn set_rotation(state: State<AppState>, rotation: Rotations, board: Option<String>) {
    let id = board.or_else(|| state.current_id());
    let screens = state.screens(id.as_deref());
    state.set_screens(id.as_deref(), ScreenSettings { rotation, ..screens });
}

/// The boards found, for the device switcher.
#[tauri::command]
fn boards(state: State<AppState>) -> Vec<DeviceInfo> {
    let primary = state.primary.lock().unwrap().clone();
    state.devices.lock().unwrap().infos(primary.as_deref())
}

/// Show `board` (a target or an id) in the UI: the `bridge` events and the
/// commands without `board` follow it. `None` goes back to the primary one.
#[tauri::command]
fn show_board(app: AppHandle, state: State<AppState>, board: Option<String>) -> Vec<DeviceInfo> {
    {
        let mut devices = state.devices.lock().unwrap();
        let target = board.and_then(|b| devices.find(&b).and_then(|l| l.target.clone()));
        devices.shown = target;
    }
    let infos = boards(state);
    let _ = app.emit("boards", &infos);
    infos
}

/// Make `board` (an id) the primary one; `None` leaves it to the order they connected in.
#[tauri::command]
fn set_primary(app: AppHandle, state: State<AppState>, board: Option<String>) -> Vec<DeviceInfo> {
    *state.primary.lock().unwrap() = board.clone();
    state.settings.lock().unwrap().primary = board;
    state.save_settings();
    let infos = boards(state);
    let _ = app.emit("boards", &infos);
    infos
}

/// The eyes face looks at the mouse pointer, or about on its own.
#[tauri::command]
fn set_follow_pointer(state: State<AppState>, on: bool) {
    state.follow_pointer.store(on, Ordering::Relaxed);
    state.settings.lock().unwrap().ignore_pointer = !on;
    state.save_settings();
}

/// The app's language: `None` follows the system's.
#[tauri::command]
fn ui_language(state: State<AppState>) -> Option<String> {
    state.settings.lock().unwrap().ui_language.clone()
}

/// The tray menu's items, in the app's language.
#[derive(Deserialize)]
struct TrayLabels {
    show: String,
    quit: String,
}

/// Keep the app's language and put the tray menu in it.
#[tauri::command]
fn set_ui_language(app: AppHandle, state: State<AppState>, language: Option<String>, tray: TrayLabels) -> Result<(), String> {
    let changed = {
        let mut settings = state.settings.lock().unwrap();
        let changed = settings.ui_language != language;
        settings.ui_language = language;
        changed
    };
    if changed {
        state.save_settings();
    }
    if let Some(icon) = app.tray_by_id(TRAY_ID) {
        icon.set_menu(Some(tray_menu(&app, &tray).map_err(|e| e.to_string())?)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Where the first-run setup and the tour stand.
#[derive(Serialize)]
struct Onboarding {
    done: bool,
    tour_version: u32,
}

#[tauri::command]
fn onboarding(state: State<AppState>) -> Onboarding {
    let settings = state.settings.lock().unwrap();
    Onboarding { done: settings.onboarding_done, tour_version: settings.tour_version }
}

/// Mark the setup done (or to do again), and the tour's version seen.
#[tauri::command]
fn set_onboarding(state: State<AppState>, done: Option<bool>, tour_version: Option<u32>) {
    {
        let mut settings = state.settings.lock().unwrap();
        if let Some(done) = done {
            settings.onboarding_done = done;
        }
        if let Some(v) = tour_version {
            settings.tour_version = v;
        }
    }
    state.save_settings();
}

/// Where the pointer is, as the board gets it: the mirror's eyes look there too.
#[tauri::command]
fn pointer_gaze() -> Option<(f32, f32)> {
    pointer::gaze()
}

/// The cover the snapshots name `id`, as a data URL, once the bridge has it.
#[tauri::command]
fn music_cover(state: State<AppState>, id: u32) -> Option<String> {
    state.music.cover().filter(|c| c.id == id).map(|c| c.data_url.to_string())
}

/// Play, pause or skip the music playing on this computer.
#[tauri::command]
async fn music_control(state: State<'_, AppState>, action: String) -> Result<String, String> {
    let music = state.music.clone();
    let action: music::Control = action.parse()?;
    tauri::async_runtime::spawn_blocking(move || music.control(action)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn readings(app: AppHandle) -> Result<Vec<Reading>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let mut slot = state.collector.lock().unwrap();
        let collector = slot.get_or_insert_with(Collector::new);
        collector.readings()
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn power_helper_status() -> power_helper::HelperStatus {
    tauri::async_runtime::spawn_blocking(power_helper::status).await.unwrap_or(power_helper::HelperStatus {
        state: power_helper::HelperState::Unavailable,
        error: None,
    })
}

#[tauri::command]
async fn set_power_helper(on: bool) -> Result<power_helper::HelperStatus, String> {
    tauri::async_runtime::spawn_blocking(move || power_helper::set(on)).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn open_power_helper_settings() {
    power_helper::open_settings();
}

/// One bundled firmware image.
#[derive(Serialize)]
struct Bundled {
    kind: BoardKind,
    size: usize,
    /// Its app descriptor: the version is what a board of `kind` gets when
    /// flashed. `None` when it isn't that board's firmware ([`firmware::check_image`]).
    image: Option<ImageInfo>,
}

#[derive(Serialize)]
struct FirmwareInfo {
    /// One per kind of board.
    bundled: Vec<Bundled>,
    /// `None` until esptool has been set up (done on first identify/flash).
    esptool: Option<Esptool>,
}

#[tauri::command]
async fn firmware_info(app: AppHandle) -> Result<FirmwareInfo, String> {
    let dir = app.state::<AppState>().esptool_dir.clone();
    let esptool = tauri::async_runtime::spawn_blocking(move || Esptool::installed(&dir)).await.map_err(|e| e.to_string())?;
    let bundled = BoardKind::ALL
        .into_iter()
        .map(|kind| Bundled { kind, size: firmware_for(kind).len(), image: firmware::check_image(kind, firmware_for(kind)).ok() })
        .collect();
    Ok(FirmwareInfo { bundled, esptool })
}

#[tauri::command]
async fn identify_board(app: AppHandle, port: Option<String>) -> Result<ChipInfo, String> {
    with_device(app, port, |app, tool, port| tool.chip_info(port, |e| emit_flash(app, e))).await
}

/// Flash `kind`'s firmware. Refused when the connected board says it is
/// another kind, or when the bundled image isn't that kind's: either would
/// leave the board with another board's display and pins.
#[tauri::command]
async fn flash_board(app: AppHandle, port: Option<String>, kind: BoardKind) -> Result<(), String> {
    let image = firmware_for(kind);
    firmware::check_image(kind, image)?;
    {
        let state = app.state::<AppState>();
        let target = port.clone().or_else(|| state.settings.lock().unwrap().port.clone()).or_else(|| current_usb_port(&state));
        let devices = state.devices.lock().unwrap();
        if let Some(link) = target.as_deref().and_then(|t| devices.boards.get(t))
            && link.kind == "connected"
            && let Some(running) = link.firmware.as_ref().and_then(BoardFirmware::kind)
            && running != kind
        {
            return Err(format!("this board is a {running}: refusing to flash {kind} firmware on it"));
        }
    }
    with_device(app, port, move |app, tool, port| {
        let path = std::env::temp_dir().join(format!("companion-firmware-{}.bin", kind.profile().slug));
        fs::write(&path, image)?;
        let result = tool.flash(port, &path, |e| emit_flash(app, e));
        let _ = fs::remove_file(&path);
        result
    })
    .await
}

#[derive(Serialize)]
struct McpInfo {
    /// What MCP clients run: this executable with `args`.
    command: Option<String>,
    args: Vec<&'static str>,
    hub: Option<HubStatus>,
    hub_error: Option<String>,
    /// Claude Code has it among its MCP servers.
    registered: bool,
    config_path: Option<PathBuf>,
}

fn mcp_info_of(state: &AppState) -> McpInfo {
    let config = mcpconfig::status();
    McpInfo {
        command: std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned()),
        args: vec![mcp::FLAG],
        hub: state.hub.as_ref().map(|h| h.status()),
        hub_error: state.hub_error.clone(),
        registered: config.registered,
        config_path: config.config_path,
    }
}

#[tauri::command]
fn mcp_info(state: State<AppState>) -> McpInfo {
    mcp_info_of(&state)
}

/// Add this app to Claude Code's MCP servers, or take it out.
#[tauri::command]
fn claude_mcp(state: State<AppState>, connect: bool) -> Result<McpInfo, String> {
    if connect {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        mcpconfig::connect(&exe).map_err(|e| e.to_string())?;
    } else {
        mcpconfig::disconnect().map_err(|e| e.to_string())?;
    }
    Ok(mcp_info_of(&state))
}

#[tauri::command]
fn claude_link() -> LinkStatus {
    statusline::status()
}

/// Make this binary Claude Code's status line, chaining to the one it replaces.
#[tauri::command]
fn claude_connect() -> Result<LinkStatus, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    statusline::connect(&exe).map_err(|e| e.to_string())
}

#[tauri::command]
fn claude_disconnect() -> Result<LinkStatus, String> {
    statusline::disconnect().map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct LastAlert {
    text: String,
    error: Option<String>,
    age_s: u64,
}

#[derive(Serialize)]
struct ClaudeAlertsInfo {
    settings: AlertSettings,
    hooks: HooksStatus,
    /// The bridge can speak (voice on with spoken replies, and Piper ready).
    can_speak: bool,
    last: Option<LastAlert>,
}

fn claude_alerts_info_of(state: &AppState) -> ClaudeAlertsInfo {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    let last = state.devices.lock().unwrap().host.last_alert.clone();
    ClaudeAlertsInfo {
        settings: state.claude_alerts.lock().unwrap().clone(),
        hooks: hooks::status(),
        can_speak: state.voice.lock().unwrap().tts.is_some(),
        last: last.map(|(text, error, at)| LastAlert { text, error, age_s: now.saturating_sub(at) / 1000 }),
    }
}

#[tauri::command]
fn claude_alerts_info(state: State<AppState>) -> ClaudeAlertsInfo {
    claude_alerts_info_of(&state)
}

#[tauri::command]
fn set_claude_alerts(state: State<AppState>, settings: AlertSettings) -> ClaudeAlertsInfo {
    *state.claude_alerts.lock().unwrap() = settings.clone();
    state.settings.lock().unwrap().claude_alerts = settings;
    state.save_settings();
    claude_alerts_info_of(&state)
}

/// Add this binary to Claude Code's hooks, or take it out.
#[tauri::command]
fn claude_hooks(state: State<AppState>, connect: bool) -> Result<ClaudeAlertsInfo, String> {
    if connect {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        hooks::connect(&exe).map_err(|e| e.to_string())?;
    } else {
        hooks::disconnect().map_err(|e| e.to_string())?;
    }
    Ok(claude_alerts_info_of(&state))
}

/// Give a sample "Claude needs you" the way a hook would.
#[tauri::command]
async fn test_claude_alert() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
        // A session of its own, so it isn't taken for a repeat.
        let event = serde_json::json!({
            "hook_event_name": "Notification",
            "notification_type": "permission_prompt",
            "session_id": format!("dualeye-test-{now}"),
            "cwd": "/DualEye",
        });
        board().claude_hook(event).map(drop).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The Timers tab: what's running and where it shows.
#[derive(Serialize)]
struct TimersInfo {
    timers: Vec<TimerInfo>,
    pomodoro: Option<Pomodoro>,
    show_on: ShowOn,
    /// The bridge says what a timer was for when it's up.
    can_speak: bool,
}

fn timers_info_of(state: &AppState) -> TimersInfo {
    let t = &state.timers;
    TimersInfo { timers: t.list(), pomodoro: t.pomodoro(), show_on: t.show_on(), can_speak: state.voice.lock().unwrap().tts.is_some() }
}

#[tauri::command]
fn timers_info(state: State<AppState>) -> TimersInfo {
    timers_info_of(&state)
}

/// Run one of `Timers::tools` (`set_timer`, `control_timer`, `update_timer`...), in the alerts' language.
#[tauri::command]
fn timer_tool(state: State<AppState>, name: String, arguments: serde_json::Value) -> Result<TimersInfo, String> {
    let language = state.claude_alerts.lock().unwrap().language.clone();
    state.timers.call_tool(&name, &arguments, &language).ok_or_else(|| format!("unknown timer tool {name}"))??;
    Ok(timers_info_of(&state))
}

#[tauri::command]
fn set_timer_screen(state: State<AppState>, show_on: ShowOn) -> TimersInfo {
    state.timers.set_show_on(show_on);
    timers_info_of(&state)
}

/// Silence whatever rings.
#[tauri::command]
fn dismiss_timers(state: State<AppState>) -> TimersInfo {
    state.timers.dismiss();
    timers_info_of(&state)
}

/// The serial port of the board the UI shows, when it's on USB.
fn current_usb_port(state: &AppState) -> Option<String> {
    let primary = state.primary.lock().unwrap().clone();
    let target = state.devices.lock().unwrap().current(primary.as_deref())?.target.clone()?;
    wifi::target(&target).is_none().then_some(target)
}

fn resume(state: &AppState, port: &str) {
    if let Some(manager) = state.manager.lock().unwrap().as_ref() {
        manager.resume(port);
    }
}

fn emit_flash(app: &AppHandle, event: FlashEvent) {
    let _ = app.emit("flash", &event);
}

/// Set esptool up if needed, stop the bridge, hand the port to esptool, then
/// start the bridge again.
async fn with_device<T: Send + 'static>(
    app: AppHandle,
    port: Option<String>,
    job: impl FnOnce(&AppHandle, &Esptool, &str) -> std::io::Result<T> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        if state.device_busy.swap(true, Ordering::SeqCst) {
            return Err("esptool is already talking to the board".to_string());
        }
        let result = (|| -> Result<T, String> {
            // Streaming carries on while Python and esptool download.
            let tool = setup::ensure(&state.esptool_dir, |e| emit_flash(&app, e)).map_err(|e| format!("setting up esptool: {e}"))?;
            let port = port
                .or_else(|| state.settings.lock().unwrap().port.clone())
                .or_else(|| current_usb_port(&state))
                .or_else(serial::detect_board)
                .ok_or("no single board found on USB: pick its port")?;
            if wifi::target(&port).is_some() {
                return Err("esptool needs the board on USB: plug it in with the cable".into());
            }
            // The window stays on this board while it's away, rather than
            // switching to another one.
            {
                let primary = state.primary.lock().unwrap().clone();
                let mut devices = state.devices.lock().unwrap();
                if devices.current(primary.as_deref()).and_then(|l| l.target.as_deref()) == Some(port.as_str()) {
                    devices.shown = Some(port.clone());
                }
            }
            // Only this port's session stops: the other boards carry on.
            let paused = state.manager.lock().unwrap().as_ref().is_none_or(|m| m.pause(&port));
            if !paused {
                resume(&state, &port);
                return Err(format!("{port} is still in use: try again"));
            }
            let result = job(&app, &tool, &port).map_err(|e| e.to_string());
            resume(&state, &port);
            result
        })();
        state.device_busy.store(false, Ordering::SeqCst);
        result
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|result| result)
}

fn start_bridge(app: &AppHandle, port: Option<String>) -> BoardManager {
    let handle = app.clone();
    let state = app.state::<AppState>();
    let (faces, rotation, hub, voice) = (state.faces.clone(), state.rotation.clone(), state.hub.clone(), state.voice.clone());
    let (claude_alerts, timers) = (state.claude_alerts.clone(), state.timers.clone());
    let (music, follow_pointer) = (state.music.clone(), state.follow_pointer.clone());
    let (quips, board_settings, primary) = (state.quips.clone(), state.board_settings.clone(), state.primary.clone());
    let transport = state.settings.lock().unwrap().transport;
    let config = BridgeConfig {
        port,
        transport,
        faces,
        rotation,
        board_settings,
        primary,
        hub,
        voice,
        claude_alerts,
        timers,
        music,
        follow_pointer,
        quips,
        ..Default::default()
    };
    // Every board found; a port in the settings pins one.
    BoardManager::spawn(config, None, move |event| on_bridge_event(&handle, &event))
}

/// Keep `event`, act on it, and forward it to the UI.
fn on_bridge_event(app: &AppHandle, event: &ManagerEvent) {
    let mut shown = event.target.is_none();
    if let Some(state) = app.try_state::<AppState>() {
        let primary = state.primary.lock().unwrap().clone();
        {
            let mut devices = state.devices.lock().unwrap();
            let before = devices.current(primary.as_deref()).and_then(|l| l.target.clone());
            devices.record(event);
            let after = devices.current(primary.as_deref()).and_then(|l| l.target.clone());
            // A board's going is shown when it was the one shown, or the last.
            let gone = matches!(event.event, BridgeEvent::Disconnected { .. });
            shown |= event.target == after || (gone && (event.target == before || devices.boards.is_empty()));
        }
        if let BridgeEvent::VoiceError { message } = &event.event {
            // Which sidecar failed: Piper's and llama.cpp's errors say so.
            let status = if message.starts_with("piper") {
                &state.tts_state
            } else if message.starts_with("llama-server") {
                &state.llm_state
            } else {
                &state.stt_state
            };
            *status.lock().unwrap() = ("error", Some(message.clone()));
        }
        // Faces and rotation came from a board (at connect, or an MCP client
        // changed them): keep them for the next launch too. The bridge has
        // them already.
        if let BridgeEvent::Settings { faces, rotation } = &event.event {
            let screens = ScreenSettings { faces: *faces, rotation: *rotation };
            let mut settings = state.settings.lock().unwrap();
            match &event.board {
                Some(id) => {
                    settings.boards.insert(id.clone(), screens);
                }
                None => {
                    settings.faces = *faces;
                    settings.rotation = *rotation;
                }
            }
            drop(settings);
            state.save_settings();
        }
        if matches!(event.event, BridgeEvent::Connected { .. } | BridgeEvent::Firmware { .. } | BridgeEvent::Disconnected { .. }) {
            let _ = app.emit("boards", state.devices.lock().unwrap().infos(primary.as_deref()));
        }
    }
    if shown && let BridgeEvent::Snapshot { snapshot, .. } = &event.event {
        update_tray(app, snapshot);
    }
    if shown {
        let _ = app.emit("bridge", event);
    }
    let _ = app.emit("manager", event);
}

fn update_tray(app: &AppHandle, s: &Snapshot) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    let temp = |t: Option<f32>| t.map_or_else(|| "—".to_string(), |t| format!("{t:.0}°"));
    let text = format!("CPU {} · GPU {}", temp(s.cpu.temp_c), temp(s.gpu.temp_c));
    let boards = app.try_state::<AppState>().map_or(0, |state| state.devices.lock().unwrap().boards.len());
    let boards = if boards > 1 { format!(" · {boards} boards") } else { String::new() };
    let _ = tray.set_tooltip(Some(format!("Companion Hub — {text}{boards}")));
}

fn show_main(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

fn tray_menu(app: &AppHandle, labels: &TrayLabels) -> tauri::Result<Menu<tauri::Wry>> {
    let show = MenuItem::with_id(app, "show", &labels.show, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", &labels.quit, true, None::<&str>)?;
    Menu::with_items(app, &[&show, &quit])
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    // English until the window loads and sends the app's language.
    let menu = tray_menu(app, &TrayLabels { show: "Show DualEye".into(), quit: "Quit".into() })?;
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Companion Hub")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    instances::wait_for_predecessor();
    tauri::Builder::default()
        // Closing the window leaves the app in the tray; launching it again must
        // not start a second bridge fighting over the serial port.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if !instances::on_relaunch(app, args.first().map(String::as_str)) {
                show_main(app);
            }
        }))
        .setup(|app| {
            // We hold the single instance now; older copies must let go of the port.
            instances::stop_strays();
            check_hardware();
            if let Some(me) = instances::SelfBinary::capture(app.handle()) {
                app.manage(me);
            }
            instances::watch_for_updates(app.handle().clone());
            if instances::start_hidden() {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.hide();
                }
            }
            let settings_path = app.path().app_config_dir().ok().map(|d| d.join("settings.json"));
            let esptool_dir = app.path().app_local_data_dir()?.join("esptool");
            let settings: Settings = settings_path
                .as_ref()
                .and_then(|p| fs::read_to_string(p).ok())
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let port = settings.port.clone();
            let faces = Arc::new(Mutex::new(settings.faces));
            let rotation = Arc::new(Mutex::new(settings.rotation));
            let board_settings = Arc::new(Mutex::new(settings.boards.clone()));
            let primary = Arc::new(Mutex::new(settings.primary.clone()));
            let claude_alerts = Arc::new(Mutex::new(settings.claude_alerts.clone()));
            let follow_pointer = Arc::new(AtomicBool::new(!settings.ignore_pointer));
            let quips = Arc::new(Mutex::new(settings.quips));
            let (hub, hub_error) = match Hub::start() {
                Ok(hub) => (Some(hub), None),
                Err(e) => (None, Some(e.to_string())),
            };
            app.manage(AppState {
                devices: Mutex::new(Devices { host: Link { kind: "searching", ..Default::default() }, ..Default::default() }),
                manager: Mutex::new(None),
                settings: Mutex::new(settings),
                settings_path,
                faces,
                rotation,
                board_settings,
                primary,
                hub,
                hub_error,
                collector: Mutex::new(None),
                device_busy: AtomicBool::new(false),
                esptool_dir,
                voice: Arc::default(),
                claude_alerts,
                timers: Arc::new(Timers::open(timers::default_file())),
                music: Arc::default(),
                follow_pointer,
                quips,
                stt_state: Arc::new(Mutex::new(("off", None))),
                tts_state: Arc::new(Mutex::new(("off", None))),
                llm_state: Arc::new(Mutex::new(("off", None))),
                installing: Mutex::new(BTreeMap::new()),
                download: Mutex::new(None),
                cancel_download: AtomicBool::new(false),
            });
            apply_voice(&app.state::<AppState>());
            let handle = app.handle();
            // A logout or `kill` quits like the tray's Quit, so the port is
            // released and the whisper-server sidecar stopped.
            let quitter = handle.clone();
            let _ = ctrlc::set_handler(move || quitter.exit(0));
            let manager = start_bridge(handle, port);
            *app.state::<AppState>().manager.lock().unwrap() = Some(manager);
            build_tray(handle)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // Keep streaming to the board when the window is closed.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![status, list_ports, set_port, set_transport, wifi_info, wifi_scan, wifi_setup, wifi_enable, wifi_forget, wifi_pair, set_faces, set_rotation, readings, firmware_info, identify_board, flash_board, claude_link, claude_connect, claude_disconnect, claude_alerts_info, set_claude_alerts, claude_hooks, test_claude_alert, mcp_info, claude_mcp, voice_info, set_voice, set_api_key, check_api_key, open_key_page, elevenlabs_voices, download_model, cancel_download, delete_model, install_engine, board_voice, set_board_volume, set_board_eyes, set_board_idle_eyes, set_board_wake_sound, set_board_pet, board_pet, quips_info, set_quips, play_board_scene, play_board_sound, test_voice, send_image, clear_image, image_preview, power_helper_status, set_power_helper, open_power_helper_settings, timers_info, timer_tool, set_timer_screen, dismiss_timers, set_follow_pointer, ui_language, set_ui_language, onboarding, set_onboarding, pointer_gaze, music_cover, music_control, boards, show_board, set_primary, yoru_settings, yoru_set_setting, yoru_react, yoru_show_menu])
        .build(tauri::generate_context!())
        .expect("failed to build the DualEye app")
        .run(|app, event| match event {
            tauri::RunEvent::Exit => {
                // Release the serial port before the process goes away.
                if let Some(state) = app.try_state::<AppState>() {
                    state.manager.lock().unwrap().take();
                    // Stops the whisper-server, Piper and llama-server sidecars,
                    // even if a reply being spoken still holds them.
                    let mut voice = state.voice.lock().unwrap();
                    voice.stt.take().inspect(|s| s.shutdown());
                    voice.tts.take().inspect(|t| t.shutdown());
                    voice.agent.take().inspect(|a| a.llm().shutdown());
                    drop(voice);
                    state.cancel_download.store(true, Ordering::SeqCst);
                    if let Some(hub) = &state.hub {
                        hub.set_unavailable("the DualEye app is quitting");
                    }
                }
            }
            // macOS: the Dock icon, or opening the app while it runs.
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => {
                if !instances::on_relaunch(app, None) {
                    show_main(app);
                }
            }
            _ => {}
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The update offer compares the board against this; a merged image without it never offers one.
    /// Each image must be its own board's: the app refuses to flash it otherwise.
    #[test]
    fn bundled_firmware_reports_its_version() {
        let versions = [
            (BoardKind::DualEye, include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../firmware/dualeye/version.txt"))),
            (BoardKind::Yoru, include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../firmware/yoru/version.txt"))),
        ];
        for (kind, want) in versions {
            let info = firmware::check_image(kind, firmware_for(kind)).unwrap();
            assert_eq!(info.version, want.trim(), "{kind}: rebuild the firmware (idf.py build merge-bin), then tools/fetch_firmware.sh");
        }
    }

    fn event(target: Option<&str>, board: Option<&str>, event: BridgeEvent) -> ManagerEvent {
        ManagerEvent { target: target.map(str::to_string), board: board.map(str::to_string), board_kind: Some(BoardKind::Yoru), event }
    }

    #[test]
    fn each_board_keeps_its_own_record() {
        let mut devices = Devices::default();
        devices.record(&event(None, None, BridgeEvent::Waiting { reason: "board not found on USB".into() }));
        assert_eq!(devices.host.kind, "searching");
        devices.record(&event(Some("/dev/a"), None, BridgeEvent::Connected { port: "/dev/a".into() }));
        devices.record(&event(Some("/dev/b"), Some("bb"), BridgeEvent::Connected { port: "/dev/b".into() }));
        devices.record(&event(Some("/dev/a"), Some("aa"), BridgeEvent::BoardLog { line: "hi".into() }));
        assert_eq!(devices.find("aa").unwrap().logs, ["hi"]);
        assert!(devices.find("/dev/b").unwrap().logs.is_empty());
        // The first connected, unless the primary one is there.
        assert_eq!(devices.current(None).unwrap().id.as_deref(), Some("aa"));
        assert_eq!(devices.current(Some("bb")).unwrap().id.as_deref(), Some("bb"));
        // The board picked in the switcher wins over both, and is waited for while it's away.
        devices.shown = Some("/dev/a".into());
        assert_eq!(devices.current(Some("bb")).unwrap().id.as_deref(), Some("aa"));
        devices.shown = Some("/dev/flashing".into());
        assert!(devices.current(Some("bb")).is_none());
        devices.shown = None;
        let infos = devices.infos(Some("bb"));
        assert_eq!(infos.iter().map(|i| (i.primary, i.current)).collect::<Vec<_>>(), [(false, false), (true, true)]);
        // Gone: the other one is shown; the last one gone, the host says so.
        let gone = || BridgeEvent::Disconnected { port: String::new(), reason: "unplugged".into(), permission_denied: false };
        devices.record(&event(Some("/dev/a"), Some("aa"), gone()));
        assert_eq!(devices.current(None).unwrap().id.as_deref(), Some("bb"));
        devices.record(&event(Some("/dev/b"), Some("bb"), gone()));
        assert!(devices.current(None).is_none());
        assert_eq!((devices.host.kind, devices.host.message.as_deref()), ("offline", Some("unplugged")));
    }

    /// A new install gets the setup; someone upgrading (a settings file without the field) doesn't.
    #[test]
    fn only_new_installs_get_the_setup() {
        assert!(!Settings::default().onboarding_done);
        let old: Settings = serde_json::from_str(r#"{"port":null}"#).unwrap();
        assert!(old.onboarding_done);
        assert_eq!(old.tour_version, 0);
    }
}
