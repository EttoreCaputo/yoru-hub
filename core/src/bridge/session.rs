//! One board's connection: the link and its reader, the protocol v2
//! handshake (again after a reboot), the voice pipeline, the pointer for its
//! eyes, and each snapshot the sampler publishes, made the board's by its
//! [`BoardBehaviour`] and sent. What differs by kind of board is left to the
//! behaviour, picked from the board's `hello`.

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
use crate::voice::{self, Hearing, Receiving, Session, Speaker};

use super::alerts::{AlertTarget, Targets};
use super::behaviour::{self, BoardBehaviour, Ctx, GazeWanted};
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
        thread::Builder::new().name("companion-voice".into()).spawn(move || {
            if let Ok(session) = link_slot_rx.recv() {
                voice::run(&config, &session, &sink, rx);
            }
        })?;
        tx
    };
    let floor = targets.floor.clone();
    let number = info.number;
    let receiving = Mutex::new(Receiving::gated(pipeline, Box::new(move || floor.claim(number))));
    let link = Arc::new({
        let heard = heard.clone();
        let sink = on_event.clone();
        let speaker = speaker.clone();
        let voice_state = voice_state.clone();
        let timers = config.timers.clone();
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
                sink(BridgeEvent::Listening {
                    id: params["id"].as_u64().unwrap_or(0) as u8,
                    trigger: params["trigger"].as_str().unwrap_or("wake").to_string(),
                });
            }
            LinkEvent::Notification { method, params } if method == "utterance_end" => receiving.lock().unwrap().end(&params),
            LinkEvent::Notification { method, params } if method == "playback_end" => speaker.playback_end(params),
            LinkEvent::Audio(payload) => receiving.lock().unwrap().audio(&payload),
            LinkEvent::Notification { .. } | LinkEvent::Closed(_) => {}
        })?
    });
    let board_kind = Arc::new(Mutex::new(BoardKind::default()));
    let board_id = Arc::new(Mutex::new(None::<String>));
    let floor = targets.floor.clone();
    let _ = link_slot_tx.send(Session {
        link: Arc::downgrade(&link),
        speaker: speaker.clone(),
        snapshot: latest.voice.clone(),
        settings_changed: voice_changed_settings.clone(),
        tools: Mutex::default(),
        kind: board_kind.clone(),
        board: board_id.clone(),
        conversation_over: Box::new(move || floor.release(number)),
        timers: config.timers.clone(),
        music: config.music.clone(),
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

    // The eyes follow the pointer: a thread of its own, at its own pace,
    // asking the board's behaviour when (once there is one).
    let board_ready = Arc::new(AtomicBool::new(false));
    let session_over = Arc::new(AtomicBool::new(false));
    let gaze_wanted = Arc::new(Mutex::new(None::<GazeWanted>));
    let _gaze = {
        let (link, ready, over) = (Arc::downgrade(&link), board_ready.clone(), session_over.clone());
        let (wanted, follow) = (gaze_wanted.clone(), config.follow_pointer.clone());
        thread::Builder::new().name("companion-gaze".into()).spawn(move || gaze_loop(&link, &ready, &over, &wanted, &follow))?;
        OnGone(session_over)
    };

    let ctx = Ctx { link: &link, config, on_event };
    let mut behaviour: Option<(BoardKind, Box<dyn BoardBehaviour>)> = None;
    let mut ready = false;
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
            if let Some((_, b)) = behaviour.as_mut() {
                b.on_reboot();
            }
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
                    if behaviour.as_ref().is_none_or(|(k, _)| *k != kind) {
                        behaviour = Some((kind, behaviour::for_kind(kind)));
                    }
                    let (_, b) = behaviour.as_mut().expect("just set");
                    ready = true;
                    info.ready.store(true, Ordering::Relaxed);
                    board_ready.store(true, Ordering::Relaxed);
                    unanswered = 0;
                    if let Some(hub) = &config.hub {
                        hub.attach(link.clone(), hello.clone(), port, say_fn(&link, &speaker, config), hub_changed_settings.clone());
                    }
                    targets.set(info.number, AlertTarget { id: hello.id.clone(), link: Arc::downgrade(&link), kind, speaker: speaker.clone(), voice_state: voice_state.clone() });
                    report(BoardFirmware::Version { kind, version: hello.firmware.clone(), idf: hello.idf.clone(), protocol: hello.protocol });
                    // The voice asks the board for its tools while it streams the
                    // audio, too busy to answer in time, and falls back on the
                    // list kept on disk: keep it this firmware's.
                    if let Ok(tools) = link.list_tools(TOOL_TIMEOUT) {
                        crate::link::save_tools_cache(kind, &tools);
                    }
                    *gaze_wanted.lock().unwrap() = Some(b.gaze(config));
                    b.on_ready(&ctx, &hello)?;
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

        let changed = hub_changed_settings.swap(false, Ordering::Relaxed) | voice_changed_settings.swap(false, Ordering::Relaxed);
        if ready && let Some((_, b)) = behaviour.as_mut() {
            b.sync(&ctx, changed)?;
        }

        let Some((number, shared)) = latest.newer(seen, WAKE) else { continue };
        seen = number;
        let mut snapshot = (*shared).clone();
        if let Some((_, b)) = behaviour.as_mut() {
            b.tailor(&ctx, &mut snapshot, ready)?;
        }
        let sent = ready && snapshot.is_sendable();
        if sent {
            link.send(Channel::Metrics, &snapshot.to_payload())?;
        }
        on_event(BridgeEvent::Snapshot { snapshot, sent });
    }
    Ok(())
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

/// While the board's eyes want the pointer ([`BoardBehaviour::gaze`]), send it
/// each time it moves. The board takes a pointer that stands still for
/// someone gone, and dozes off.
fn gaze_loop(link: &Weak<Link>, ready: &AtomicBool, over: &AtomicBool, wanted: &Mutex<Option<GazeWanted>>, follow: &AtomicBool) {
    let mut sent: Option<(f32, f32)> = None;
    while !over.load(Ordering::Relaxed) {
        let eyes = wanted.lock().unwrap().as_ref().is_some_and(|w| w());
        if !(ready.load(Ordering::Relaxed) && follow.load(Ordering::Relaxed) && eyes) {
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
    Arc::new(move |text: &str, language: Option<&str>| {
        let tts = voice.lock().unwrap().tts.clone().ok_or("the bridge isn't speaking: turn on spoken replies in the app, or run companion --tts")?;
        let link = link.upgrade().ok_or("the board disconnected")?;
        let language = language.unwrap_or_else(|| intents::guess_language(text));
        let spoken = speaker.speak(&link, &tts, text, language)?;
        Ok(serde_json::to_value(spoken).unwrap_or_default())
    })
}
