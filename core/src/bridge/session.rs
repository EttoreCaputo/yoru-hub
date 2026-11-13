//! One board's connection: the link and its reader, the protocol v2
//! handshake (again after a reboot), the voice pipeline, the pointer for its
//! eyes, each snapshot the sampler publishes, and the timers and tasks
//! lists of the menu's apps whenever they change (firmware 0.11.0, tasks
//! since 0.16.0).

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::firmware::{self, BoardFirmware};
use crate::intents;
use crate::link::{CallError, Link, LinkEvent};
use crate::pointer;
use crate::profile::BoardKind;
use crate::protocol::Channel;
use crate::voice::{self, Hearing, Receiving, Session, Speaker, TurnJob};

use super::alerts::{AlertTarget, Targets};
use super::manager::SessionInfo;
use super::sampler::Latest;
use super::{BridgeConfig, BridgeEvent, EventSink, TOOL_TIMEOUT};

/// How long each `hello` of the handshake waits; the handshake repeats it every interval.
const HELLO_TIMEOUT: Duration = Duration::from_millis(500);
/// Unanswered `hello`s before also asking the protocol 1 way.
const LEGACY_QUERY_AFTER: u32 = 3;
/// Once a board has gone that long without answering, ask only this often.
const HELLO_BACKOFF: Duration = Duration::from_secs(5);
/// The longest the loop waits for a snapshot before looking after the link again.
const WAKE: Duration = Duration::from_millis(200);

/// What the reader thread learns for the session loop.
#[derive(Default)]
struct Heard {
    /// The board announced a reboot: handshake and push settings again.
    rebooted: AtomicBool,
    /// What a board that doesn't speak protocol v2 printed about itself.
    old_firmware: Mutex<Option<BoardFirmware>>,
}

/// Run a session with the board on `port` until it goes away, or `stop` or
/// the session's own stop ([`SessionInfo::stop`]) is set. What the board says
/// about itself goes into `info`, for the manager.
pub(super) fn run(
    port: &str,
    config: &BridgeConfig,
    stop: &AtomicBool,
    info: &SessionInfo,
    latest: &Latest,
    targets: &Targets,
    on_event: &EventSink,
) -> io::Result<()> {
    let heard = Arc::new(Heard::default());
    let speaker = Arc::new(Speaker::default());
    let voice_state = Arc::new(Mutex::new("idle".to_string()));
    let voice_changed_settings = Arc::new(AtomicBool::new(false));
    let hub_changed_settings = Arc::new(AtomicBool::new(false));
    // The pipeline needs the link to answer the board; it gets it once open.
    let (link_slot_tx, link_slot_rx) = std::sync::mpsc::channel::<Session>();
    let pipeline = {
        let config = config.voice.clone();
        let sink = on_event.clone();
        let (tx, rx) = std::sync::mpsc::channel::<Hearing>();
        thread::Builder::new().name("yoru-voice".into()).spawn(move || {
            if let Ok(session) = link_slot_rx.recv() {
                voice::run(&config, &session, &sink, rx);
            }
        })?;
        tx
    };
    // Smart Turn's judge of pauses, with the link once it's open.
    let turn_link = Arc::new(std::sync::OnceLock::<Weak<Link>>::new());
    let turn_jobs = {
        let (config, link, sink) = (config.voice.clone(), turn_link.clone(), on_event.clone());
        let (tx, rx) = std::sync::mpsc::channel::<TurnJob>();
        thread::Builder::new().name("yoru-turns".into()).spawn(move || voice::turns(&config, &link, &sink, rx))?;
        tx
    };
    let floor = targets.floor.clone();
    let number = info.number;
    let receiving = Mutex::new(Receiving::gated(pipeline, Box::new(move || floor.claim(number))));
    let board_id = Arc::new(Mutex::new(None::<String>));
    let link = Arc::new({
        let heard = heard.clone();
        let board = board_id.clone();
        let sink = on_event.clone();
        let speaker = speaker.clone();
        let voice_state = voice_state.clone();
        let timers = config.timers.clone();
        let confirmations = config.confirmations.clone();
        let assistant = config.assistant.clone();
        let voice = config.voice.clone();
        let smart_turn = move || voice.lock().unwrap().turn.is_some();
        Link::open(port, move |event| match event {
            LinkEvent::Log(line) => sink(BridgeEvent::BoardLog { line }),
            LinkEvent::Text(line) => {
                let old = firmware::parse_version_line(&line)
                    .or_else(|| firmware::is_missing_app_log(&line).then_some(BoardFirmware::Missing));
                if old.is_some() {
                    *heard.old_firmware.lock().unwrap() = old;
                }
                sink(BridgeEvent::BoardLog { line });
            }
            LinkEvent::Notification { method, .. } if method == "ready" => heard.rebooted.store(true, Ordering::Relaxed),
            LinkEvent::Notification { method, params } if method == "wake" => {
                // Whoever says the wake word has heard the alarm.
                timers.dismiss();
                sink(BridgeEvent::Wake { word: params["word"].as_str().unwrap_or_default().to_string(), volume_db: params["volume_db"].as_f64() })
            }
            LinkEvent::Notification { method, params } if method == "voice_state" => {
                if let Some(state) = params["state"].as_str() {
                    *voice_state.lock().unwrap() = state.to_string();
                    sink(BridgeEvent::VoiceState { state: state.to_string() });
                }
            }
            LinkEvent::Notification { method, params } if method == "utterance_start" => {
                receiving.lock().unwrap().start(&params);
                let (id, trigger) = (params["id"].as_u64().unwrap_or(0) as u8, params["trigger"].as_str().unwrap_or("wake").to_string());
                // Push-to-talk ends when PWR is let go, not at a pause.
                if trigger != "button" && smart_turn() {
                    let _ = turn_jobs.send(TurnJob::Start(id));
                }
                sink(BridgeEvent::Listening { id, trigger });
            }
            LinkEvent::Notification { method, params } if method == "utterance_pause" => {
                let id = params["id"].as_u64().unwrap_or(0) as u8;
                if smart_turn() && let Some(samples) = receiving.lock().unwrap().so_far(id) {
                    let _ = turn_jobs.send(TurnJob::Pause(id, samples));
                }
            }
            LinkEvent::Notification { method, params } if method == "utterance_end" => receiving.lock().unwrap().end(&params),
            LinkEvent::Notification { method, params } if method == "playback_end" => speaker.playback_end(params),
            LinkEvent::Notification { method, params } if method == "approval" => super::confirm::on_notification(&confirmations, &params),
            LinkEvent::Notification { method, params } if method == "notification_action" => {
                if let Some(assistant) = &assistant {
                    on_notification_action(assistant, &params, &sink);
                }
            }
            LinkEvent::Notification { method, params } if method == "timer_action" => {
                let (id, action) = (params["id"].as_str().unwrap_or_default(), params["action"].as_str().unwrap_or_default());
                if let Err(e) = timers.act(id, action) {
                    sink(BridgeEvent::BoardLog { line: format!("host: timer_action {id} {action}: {e}") });
                }
            }
            LinkEvent::Notification { method, params } if method == "timer_new" => {
                // A timer the board started without a host: the hub keeps it now.
                let board = board.lock().unwrap().clone();
                if let Err(e) = timers.adopt(&params, board.as_deref()) {
                    sink(BridgeEvent::BoardLog { line: format!("host: timer_new: {e}") });
                }
            }
            LinkEvent::Notification { method, params } if method == "task_run" => {
                // "Do it now" on the board: the scheduler does it, with the board and the cards.
                if let (Some(assistant), Some(id)) = (&assistant, params["id"].as_str())
                    && let Err(e) = assistant.tasks.ask_run(id)
                {
                    sink(BridgeEvent::BoardLog { line: format!("host: task_run {id}: {e}") });
                }
            }
            LinkEvent::Audio(payload) => receiving.lock().unwrap().audio(&payload),
            LinkEvent::Notification { .. } | LinkEvent::Closed(_) => {}
        })?
    });
    let _ = turn_link.set(Arc::downgrade(&link));
    let board_kind = Arc::new(Mutex::new(BoardKind::default()));
    let floor = targets.floor.clone();
    let _ = link_slot_tx.send(Session {
        link: Arc::downgrade(&link),
        speaker: speaker.clone(),
        settings_changed: voice_changed_settings.clone(),
        tools: Mutex::default(),
        kind: board_kind.clone(),
        board: board_id.clone(),
        conversation_over: Box::new(move || floor.release(number)),
        timers: config.timers.clone(),
        assistant: config.assistant.clone(),
        confirmations: config.confirmations.clone(),
        voice_state: voice_state.clone(),
    });
    // The sampler samples while a session is open; this one waits for its next snapshot.
    let _open = latest.open_session();
    let mut seen = latest.number();
    // On WiFi, where it is rather than which board.
    let label = link.peer().map_or_else(|| port.to_string(), |addr| format!("WiFi {}", addr.ip()));
    let port = label.as_str();
    *info.label.lock().unwrap() = label.clone();
    on_event(BridgeEvent::Connected { port: port.to_string() });

    let mut reported: Option<BoardFirmware> = None;
    let mut report = |firmware: BoardFirmware| {
        if reported.as_ref() != Some(&firmware) {
            reported = Some(firmware.clone());
            on_event(BridgeEvent::Firmware { firmware });
        }
    };
    let closed = |link: &Link| io::Error::new(io::ErrorKind::BrokenPipe, link.closed_reason().unwrap_or_else(|| "port closed".into()));

    // The eyes follow the pointer: a thread of its own, at its own pace. The
    // board decides when to look (not while a finger is on the screen, in a
    // conversation or asleep).
    let board_ready = Arc::new(AtomicBool::new(false));
    let session_over = Arc::new(AtomicBool::new(false));
    let _gaze = {
        let (link, ready, over) = (Arc::downgrade(&link), board_ready.clone(), session_over.clone());
        let follow = config.follow_pointer.clone();
        thread::Builder::new().name("yoru-gaze".into()).spawn(move || gaze_loop(&link, &ready, &over, &follow))?;
        OnGone(session_over)
    };

    let mut ready = false;
    let mut lists = Lists::default();
    let mut unanswered = 0u32;
    let mut next_hello = Instant::now();
    while !stop.load(Ordering::Relaxed) && !info.stop.load(Ordering::Relaxed) {
        if !link.is_alive() {
            return Err(closed(&link));
        }
        if heard.rebooted.swap(false, Ordering::Relaxed) {
            if let Some(hub) = &config.hub {
                hub.detach(port, "the board is restarting");
            }
            ready = false;
            info.ready.store(false, Ordering::Relaxed);
            board_ready.store(false, Ordering::Relaxed);
            next_hello = Instant::now();
        }
        if !ready && Instant::now() >= next_hello {
            next_hello = Instant::now() + config.interval;
            match link.hello(HELLO_TIMEOUT) {
                Ok(hello) => {
                    let kind = BoardKind::from_hello(&hello);
                    *board_kind.lock().unwrap() = kind;
                    *board_id.lock().unwrap() = hello.id.clone();
                    info.learn(&hello, kind);
                    lists = Lists::for_board(&hello.capabilities);
                    ready = true;
                    info.ready.store(true, Ordering::Relaxed);
                    board_ready.store(true, Ordering::Relaxed);
                    unanswered = 0;
                    // The notch isn't a board Claude Code drives, nor one whose
                    // tools are worth keeping: its list would replace the board's.
                    let notch = port == crate::notch::TARGET;
                    if let Some(hub) = config.hub.as_ref().filter(|_| !notch) {
                        hub.attach(link.clone(), hello.clone(), port, say_fn(&link, &speaker, config), hub_changed_settings.clone());
                    }
                    targets.set(info.number, AlertTarget { id: hello.id.clone(), link: Arc::downgrade(&link), kind, speaker: speaker.clone(), voice_state: voice_state.clone() });
                    report(BoardFirmware::Version { kind, version: hello.firmware.clone(), idf: hello.idf.clone(), protocol: hello.protocol });
                    // The voice asks the board for its tools while it streams the
                    // audio, too busy to answer in time, and falls back on the
                    // list kept on disk: keep it this firmware's.
                    if !notch && let Ok(tools) = link.list_tools(TOOL_TIMEOUT) {
                        crate::link::save_tools_cache(kind, &tools);
                    }
                    if let Some(assistant) = &config.assistant
                        && hello.capabilities.iter().any(|c| c == "inbox")
                    {
                        resend_inbox(&link, assistant);
                    }
                }
                Err(CallError::Io(e)) => return Err(e),
                Err(CallError::Closed) => return Err(closed(&link)),
                Err(_) => {
                    unanswered += 1;
                    if unanswered == LEGACY_QUERY_AFTER {
                        link.send_raw(firmware::VERSION_QUERY.as_bytes())?;
                    }
                    if unanswered > 2 * LEGACY_QUERY_AFTER {
                        next_hello = Instant::now() + HELLO_BACKOFF;
                    }
                }
            }
            if let Some(old) = heard.old_firmware.lock().unwrap().take().filter(|_| !ready) {
                report(old);
            }
        }

        // Yoru keeps its settings itself: nothing to read back or push.
        hub_changed_settings.store(false, Ordering::Relaxed);
        voice_changed_settings.store(false, Ordering::Relaxed);

        if ready {
            lists.send(&link, config);
        }

        let Some((number, shared)) = latest.newer(seen, WAKE) else { continue };
        seen = number;
        let snapshot = (*shared).clone();
        let sent = ready && snapshot.is_sendable();
        if sent {
            link.send(Channel::Metrics, &snapshot.to_payload())?;
        }
        on_event(BridgeEvent::Snapshot { snapshot, sent });
    }
    Ok(())
}

/// What the board's Timers and Tasks apps were last sent, to send them again
/// only when they change (and on every handshake).
#[derive(Default)]
struct Lists {
    timers: Option<Sent<u64>>,
    /// The revision, and the minute: a task's next time moves on by itself.
    tasks: Option<Sent<(u64, i64)>>,
    /// The services for talking without the computer, as sent, and when they
    /// were last looked at: they're read from the settings and the memory.
    cloud: Option<Sent<String>>,
    cloud_checked: Option<Instant>,
    /// The meeting's revision (firmware 0.20.15).
    meeting: Option<Sent<u64>>,
    /// What Yoru has grown into, counted from the memory, and when it was
    /// last counted (firmware 0.20.20).
    growth: Option<Sent<crate::assistant::memory::growth::Growth>>,
    growth_checked: Option<Instant>,
}

/// How often the services sent to the board are looked at again.
const CLOUD_EVERY: Duration = Duration::from_secs(30);
/// How often the memory is counted for the board's growth.
const GROWTH_EVERY: Duration = Duration::from_secs(60);

/// One list: whether the board takes it, and what it got last.
#[derive(Default)]
struct Sent<T> {
    last: Option<T>,
}

impl Lists {
    /// Nothing sent yet, for a board with these capabilities.
    fn for_board(capabilities: &[String]) -> Self {
        let has = |c: &str| capabilities.iter().any(|x| x == c);
        Self {
            timers: has("timers").then(Sent::default),
            tasks: has("tasks").then(Sent::default),
            cloud: has("cloud").then(Sent::default),
            cloud_checked: None,
            meeting: has("meetings").then(Sent::default),
            growth: has("growth").then(Sent::default),
            growth_checked: None,
        }
    }

    /// Send what changed since last time. A list the board refused is tried
    /// again at the next change, not in a loop.
    fn send(&mut self, link: &Link, config: &BridgeConfig) {
        if let Some(sent) = &mut self.timers {
            let revision = config.timers.revision();
            if sent.last != Some(revision) {
                sent.last = Some(revision);
                let _ = link.call_tool("set_timers", config.timers.board_list(), TOOL_TIMEOUT);
            }
        }
        if let (Some(sent), Some(assistant)) = (&mut self.tasks, &config.assistant) {
            // Every change, and every ten minutes for the times to come.
            let now = (assistant.tasks.revision(), chrono::Local::now().timestamp() / 600);
            if sent.last != Some(now) {
                sent.last = Some(now);
                let _ = link.call_tool("set_tasks", assistant.tasks.board_list(), TOOL_TIMEOUT);
            }
        }
        if let (Some(sent), Some(assistant)) = (&mut self.meeting, &config.assistant) {
            let revision = assistant.meeting.revision();
            if sent.last != Some(revision) {
                sent.last = Some(revision);
                let (state, source) = assistant.meeting.board_state();
                if state == "recording" && !source.is_empty() {
                    crate::icons::send(link, &source);
                }
                let _ = link.call_tool("set_meeting", json!({"state": state, "source": source}), TOOL_TIMEOUT);
            }
        }
        if let (Some(sent), Some(assistant)) = (&mut self.growth, &config.assistant)
            && self.growth_checked.is_none_or(|t| t.elapsed() >= GROWTH_EVERY)
        {
            self.growth_checked = Some(Instant::now());
            let growth = assistant.memory.growth();
            if sent.last != Some(growth)
                && let Ok(result) = link.call_tool("set_growth", json!(growth), TOOL_TIMEOUT)
                && !result.is_error
            {
                sent.last = Some(growth);
                if let Some(new) = result.structured_content.as_ref().and_then(|s| s["new"].as_array()).filter(|n| !n.is_empty()) {
                    eprintln!("growth: the board unlocked {new:?}");
                }
            }
        }
        if let Some(sent) = &mut self.cloud
            && self.cloud_checked.is_none_or(|t| t.elapsed() >= CLOUD_EVERY)
        {
            self.cloud_checked = Some(Instant::now());
            let services = super::cloud::services(config);
            let text = services.to_string();
            // Refused or lost: tried again next time.
            if sent.last.as_ref() != Some(&text) && link.call("cloud/set", services, TOOL_TIMEOUT).is_ok() {
                sent.last = Some(text);
            }
        }
    }
}

/// How many unread notifications a board that (re)connects gets back.
const RESEND: usize = 10;

/// A board that just connected gets the unread notifications it may have
/// missed, quietly: they were told when they came.
fn resend_inbox(link: &Link, assistant: &crate::Assistant) {
    let unread: Vec<_> = assistant.inbox.list().into_iter().filter(|n| !n.read).take(RESEND).collect();
    // Oldest first, so the newest ends on top.
    for n in unread.iter().rev() {
        let _ = link.call_tool("show_notification", super::alerts::notification_args(n, crate::assistant::Urgency::Low), TOOL_TIMEOUT);
    }
}

/// The board's `notification_action`: read, dismissed, judged, opened, or one of its buttons.
fn on_notification_action(assistant: &crate::Assistant, params: &serde_json::Value, sink: &EventSink) {
    let (Some(id), Some(action)) = (params["id"].as_str(), params["action"].as_str()) else { return };
    match action {
        "dismiss" => {
            assistant.inbox.dismiss(id);
        }
        "read" => {
            assistant.inbox.mark_read(id);
        }
        // The verdict takes it away, as on the board.
        "useful" | "enough" => {
            if let Some(n) = assistant.inbox.list().into_iter().find(|n| n.id == id) {
                assistant.memory.judge(&n, action == "useful");
            }
            assistant.inbox.dismiss(id);
        }
        // "open", or its one button, "Open".
        _ => {
            assistant.inbox.mark_read(id);
            let url = assistant.inbox.list().into_iter().find(|n| n.id == id).and_then(|n| n.url);
            sink(BridgeEvent::NotificationOpened { id: id.to_string(), url });
        }
    }
}

/// Sets its flag when dropped: the session is over, its threads stop.
struct OnGone(Arc<AtomicBool>);

impl Drop for OnGone {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// How often the pointer is looked at while the eyes want it, and while not.
const GAZE_EVERY: Duration = Duration::from_millis(50);
const GAZE_IDLE: Duration = Duration::from_millis(400);
/// Smaller moves than this (on the -1..1 scale) aren't sent.
const GAZE_STEP: f32 = 0.004;

/// While the board is ready and the setting is on, send the pointer each
/// time it moves. The board takes a pointer that stands still for someone
/// gone, and dozes off.
fn gaze_loop(link: &Weak<Link>, ready: &AtomicBool, over: &AtomicBool, follow: &AtomicBool) {
    let mut sent: Option<(f32, f32)> = None;
    while !over.load(Ordering::Relaxed) {
        if !(ready.load(Ordering::Relaxed) && follow.load(Ordering::Relaxed)) {
            sent = None;
            thread::sleep(GAZE_IDLE);
            continue;
        }
        let Some(link) = link.upgrade() else { return };
        if let Some((x, y)) = pointer::gaze()
            && sent.is_none_or(|(sx, sy)| (x - sx).abs() >= GAZE_STEP || (y - sy).abs() >= GAZE_STEP)
        {
            // The first one after a pause goes even if the pointer is still: it
            // tells the board the host follows the pointer.
            if link.notify("eyes/gaze", json!({"x": x, "y": y})).is_err() {
                return;
            }
            sent = Some((x, y));
        }
        drop(link);
        thread::sleep(GAZE_EVERY);
    }
}

/// `host/say` for the hub: speak with the voice config of the moment.
fn say_fn(link: &Arc<Link>, speaker: &Arc<Speaker>, config: &BridgeConfig) -> crate::hub::SayFn {
    let (link, speaker, voice) = (Arc::downgrade(link), speaker.clone(), config.voice.clone());
    Arc::new(move |text: &str, language: Option<&str>, recorded: bool| {
        let link = link.upgrade().ok_or("the board disconnected")?;
        let language = language.unwrap_or_else(|| intents::guess_language(text));
        let spoken = if recorded {
            speaker.speak_recorded(&link, text, language)?
        } else {
            let tts = voice.lock().unwrap().tts.clone().ok_or("the bridge isn't speaking: turn on spoken replies in the app, or run companion --tts")?;
            speaker.speak(&link, &tts, text, language)?
        };
        Ok(serde_json::to_value(spoken).unwrap_or_default())
    })
}
