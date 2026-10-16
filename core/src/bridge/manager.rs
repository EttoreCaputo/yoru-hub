//! Several boards at once: the [`Manager`] finds them, on USB and on WiFi,
//! runs a session for each on a thread of its own, and shares the sampler,
//! the alerts and the hub between them.
//!
//! - **Finding them.** Every Espressif device on USB is a candidate, and so is
//!   every board paired for WiFi ([`crate::wifi`]); `transport` narrows that
//!   down, and a fixed `port` replaces it.
//! - **One session per target.** A target (a port, or `wifi:<id>`) has at most
//!   one session. One that ends is tried again after a pause that grows while
//!   it keeps failing.
//! - **USB first.** A board is known by the `id` in its `hello`. When a board
//!   on USB turns out to be one already reached on WiFi, the WiFi session
//!   ends; WiFi isn't tried for a board while USB boards are still saying who
//!   they are.
//! - **A cap.** With `max_boards`, no more sessions than that run; at the cap a
//!   WiFi session still gives way to a board plugged in on USB, as the bridge
//!   always did.
//!
//! A port can be paused ([`Manager::pause`]): its session ends and none is
//! started there until it's resumed, so esptool can have the port while the
//! other boards carry on.
//!
//! Events carry the session they come from ([`ManagerEvent`]); the host's own
//! (alerts, timers, quips, the voice's start-up) carry none.

use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, Scope, ScopedJoinHandle};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::claude::alerts as claude_alerts;
use crate::link::Hello;
use crate::profile::BoardKind;
use crate::serial;
use crate::wifi;

use super::agents;
use super::alerts::{self, AlertInput, Targets};
use super::sampler::{Latest, Sampler};
use super::{BridgeConfig, BridgeEvent, EventSink, Transport, session, sleep_unless_stopped, warm_up_voice};

/// How often the manager looks for boards and at its sessions.
const LOOK_EVERY: Duration = Duration::from_millis(500);
/// A USB session younger than this that hasn't said who it is holds WiFi back.
const USB_INTRODUCTION: Duration = Duration::from_secs(10);
/// How often "no board" is said again while there is none.
const WAITING_EVERY: Duration = Duration::from_secs(5);

/// A [`BridgeEvent`] and the session it comes from.
#[derive(Debug, Clone, Serialize)]
pub struct ManagerEvent {
    /// What the session opened: a serial port, or `wifi:<id>`. `None` for
    /// the host's own events.
    pub target: Option<String>,
    /// The board's id, once its `hello` said.
    pub board: Option<String>,
    /// Its kind, likewise.
    pub board_kind: Option<BoardKind>,
    #[serde(flatten)]
    pub event: BridgeEvent,
}

pub type ManagerSink = Arc<dyn Fn(ManagerEvent) + Send + Sync>;

/// The manager on its own thread. Dropping it stops every session.
pub struct Manager {
    stop: Arc<AtomicBool>,
    control: Arc<Control>,
    thread: Option<thread::JoinHandle<()>>,
}

/// What the manager is told from outside while it runs, and what it says back.
#[derive(Default)]
struct Control {
    /// Targets that get no session.
    paused: Mutex<HashSet<String>>,
    /// Targets with a session running now.
    running: Mutex<HashSet<String>>,
}

/// How long [`Manager::pause`] waits for the session to let go of the port.
const PAUSE_WAIT: Duration = Duration::from_secs(5);

impl Manager {
    /// Drive every board found, up to `max_boards`.
    pub fn spawn(config: BridgeConfig, max_boards: Option<usize>, on_event: impl Fn(ManagerEvent) + Send + Sync + 'static) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let sink: ManagerSink = Arc::new(on_event);
        let control = Arc::new(Control::default());
        let thread = {
            let control = control.clone();
            thread::Builder::new()
                .name("companion-manager".into())
                .spawn(move || run_with(&config, max_boards, &flag, sink, &control))
                .expect("spawn manager thread")
        };
        Self { stop, control, thread: Some(thread) }
    }

    /// End the session on `target` (a serial port) and start none there
    /// until [`Manager::resume`]: whether it let go of the port in time.
    pub fn pause(&self, target: &str) -> bool {
        self.control.paused.lock().unwrap().insert(target.to_string());
        let until = Instant::now() + PAUSE_WAIT;
        while self.control.running.lock().unwrap().contains(target) {
            if Instant::now() >= until {
                return false;
            }
            thread::sleep(Duration::from_millis(50));
        }
        true
    }

    /// Sessions may run on `target` again.
    pub fn resume(&self, target: &str) {
        self.control.paused.lock().unwrap().remove(target);
    }

    /// The targets with a session running now.
    pub fn running(&self) -> Vec<String> {
        self.control.running.lock().unwrap().iter().cloned().collect()
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

impl Drop for Manager {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// What a session tells the manager about its board, and how the manager stops it.
#[derive(Default)]
pub(super) struct SessionInfo {
    /// Numbers the session among all of them, for the alert targets.
    pub number: u64,
    /// The manager wants this session to end (its board is on USB now).
    pub stop: AtomicBool,
    /// The board answered `hello`.
    pub ready: AtomicBool,
    id: Mutex<Option<String>>,
    kind: Mutex<Option<BoardKind>>,
    /// What the session calls its port in events and to hub clients.
    pub label: Mutex<String>,
}

impl SessionInfo {
    /// What the board's `hello` said about it.
    pub fn learn(&self, hello: &Hello, kind: BoardKind) {
        *self.id.lock().unwrap() = hello.id.clone();
        *self.kind.lock().unwrap() = Some(kind);
    }

    fn id(&self) -> Option<String> {
        self.id.lock().unwrap().clone()
    }
}

/// Wrap `sink` so each event carries `target` and what `info` knows by then.
fn tagged(sink: &ManagerSink, target: Option<String>, info: Option<Arc<SessionInfo>>) -> EventSink {
    let sink = sink.clone();
    Arc::new(move |event| {
        let (board, board_kind) = info.as_ref().map_or((None, None), |i| (i.id(), *i.kind.lock().unwrap()));
        sink(ManagerEvent { target: target.clone(), board, board_kind, event })
    })
}

/// Run the manager on the current thread until `stop` is set.
pub fn run(config: &BridgeConfig, max_boards: Option<usize>, stop: &AtomicBool, sink: ManagerSink) {
    run_with(config, max_boards, stop, sink, &Control::default());
}

fn run_with(config: &BridgeConfig, max_boards: Option<usize>, stop: &AtomicBool, sink: ManagerSink, control: &Control) {
    let host = tagged(&sink, None, None);
    warm_up_voice(config, &host);
    let (alert_tx, alert_rx) = mpsc::channel::<AlertInput>();
    let targets = Targets::default();
    if let Some(hub) = &config.hub {
        let tx = Mutex::new(alert_tx.clone());
        let feed = config.agents.clone();
        hub.set_claude_hook(Some(Arc::new(move |json| {
            feed.on_hook(&json);
            if let Some(event) = claude_alerts::parse_hook(&json) {
                let _ = tx.lock().unwrap().send(AlertInput::Hook(event, Instant::now()));
            }
        })));
        hub.set_timers(Some(config.timers.clone()));
        hub.set_primary(Some(config.primary.clone()));
    }
    let views = agents::listen(config);
    let latest = Latest::default();
    let sampler = Sampler::new(config);
    let shared = Shared { config, stop, latest: &latest, targets: &targets, sink: &sink, host: &host, control };
    thread::scope(|scope| {
        let _ = thread::Builder::new()
            .name("companion-alerts".into())
            .spawn_scoped(scope, || alerts::alert_loop(config, stop, &targets, alert_rx, &host));
        let _ = thread::Builder::new()
            .name("companion-agents".into())
            .spawn_scoped(scope, || agents::agents_loop(stop, &targets, views, &host));
        let _ = thread::Builder::new()
            .name("companion-sampler".into())
            .spawn_scoped(scope, || sampler.run(config, stop, &latest, &alert_tx, &targets));
        manage(scope, &shared, max_boards);
    });
    if let Some(hub) = &config.hub {
        hub.set_claude_hook(None);
        hub.set_claude_permission(None);
        hub.set_timers(None);
        hub.set_primary(None);
    }
}

/// What every session gets.
struct Shared<'a> {
    config: &'a BridgeConfig,
    stop: &'a AtomicBool,
    latest: &'a Latest,
    targets: &'a Targets,
    sink: &'a ManagerSink,
    host: &'a EventSink,
    control: &'a Control,
}

/// A session running on its thread.
struct Running<'scope> {
    target: String,
    info: Arc<SessionInfo>,
    started: Instant,
    thread: ScopedJoinHandle<'scope, io::Result<()>>,
}

impl Running<'_> {
    fn is_wifi(&self) -> bool {
        wifi::target(&self.target).is_some()
    }
}

/// When a target may be tried again, and the pause after that.
struct Retry {
    at: Instant,
    delay: Duration,
}

/// Find boards and keep a session running for each, until `stop`.
fn manage<'scope, 'env>(scope: &'scope Scope<'scope, 'env>, shared: &'env Shared<'env>, max_boards: Option<usize>) {
    let numbers = AtomicU64::new(0);
    let mut running: Vec<Running<'scope>> = Vec::new();
    let mut retry: HashMap<String, Retry> = HashMap::new();
    let mut waiting: Option<(String, Instant)> = None;
    while !shared.stop.load(Ordering::Relaxed) {
        let paused = shared.control.paused.lock().unwrap().clone();
        for r in running.iter().filter(|r| paused.contains(&r.target)) {
            r.info.stop.store(true, Ordering::Relaxed);
        }
        reap(&mut running, &mut retry, shared);
        *shared.control.running.lock().unwrap() = running.iter().map(|r| r.target.clone()).collect();
        let (mut usb, wifi_targets, why_none) = candidates(shared.config);
        usb.retain(|p| !paused.contains(p));
        prefer_usb(&running, &usb, shared.config, max_boards, shared.host);

        // USB first, then WiFi once the USB boards have said who they are.
        let introducing = running.iter().any(|r| !r.is_wifi() && !r.info.ready.load(Ordering::Relaxed) && r.started.elapsed() < USB_INTRODUCTION);
        let on_usb: Vec<String> = running.iter().filter(|r| !r.is_wifi()).filter_map(|r| r.info.id()).collect();
        let wifi_targets = wifi_targets.into_iter().filter(|t| !introducing && !wifi::target(t).flatten().is_some_and(|id| on_usb.iter().any(|u| u == id)));
        for target in usb.iter().cloned().chain(wifi_targets) {
            if max_boards.is_some_and(|n| running.len() >= n) {
                break;
            }
            if running.iter().any(|r| r.target == target) || retry.get(&target).is_some_and(|r| Instant::now() < r.at) {
                continue;
            }
            if let Some(started) = start(scope, shared, &target, numbers.fetch_add(1, Ordering::Relaxed)) {
                shared.control.running.lock().unwrap().insert(target.clone());
                running.push(started);
            }
        }

        if running.is_empty() {
            let reason = why_none.unwrap_or("connecting to the board");
            if waiting.as_ref().is_none_or(|(r, at)| r != reason || at.elapsed() >= WAITING_EVERY) {
                if let Some(hub) = &shared.config.hub {
                    hub.set_unavailable(reason);
                }
                (shared.host)(BridgeEvent::Waiting { reason: reason.into() });
                waiting = Some((reason.to_string(), Instant::now()));
            }
        } else {
            waiting = None;
        }
        sleep_unless_stopped(LOOK_EVERY, shared.stop);
    }
    for r in &running {
        r.info.stop.store(true, Ordering::Relaxed);
    }
    for r in running {
        let _ = r.thread.join();
    }
}

/// The targets to run sessions for: on USB, on WiFi, and why there are none.
fn candidates(config: &BridgeConfig) -> (Vec<String>, Vec<String>, Option<&'static str>) {
    if let Some(port) = &config.port {
        return match wifi::target(port) {
            Some(_) => (Vec::new(), vec![port.clone()], None),
            None => (vec![port.clone()], Vec::new(), None),
        };
    }
    let usb = if config.transport == Transport::Wifi { Vec::new() } else { serial::board_ports() };
    let wifi_targets: Vec<String> =
        if config.transport == Transport::Usb { Vec::new() } else { wifi::load_paired().into_iter().map(|b| format!("{}:{}", wifi::SCHEME, b.id)).collect() };
    let why = match config.transport {
        _ if !usb.is_empty() || !wifi_targets.is_empty() => None,
        Transport::Wifi => Some("no board set up for WiFi: set its WiFi up over USB first"),
        _ => Some("board not found on USB"),
    };
    (usb, wifi_targets, why)
}

/// End the WiFi sessions of boards that are on USB now. At the cap, a WiFi
/// session also gives way to any board on USB that has no session yet (only
/// with `transport` auto and no fixed port).
fn prefer_usb(running: &[Running], usb: &[String], config: &BridgeConfig, max_boards: Option<usize>, host: &EventSink) {
    let ready_on_usb: Vec<String> =
        running.iter().filter(|r| !r.is_wifi() && r.info.ready.load(Ordering::Relaxed)).filter_map(|r| r.info.id()).collect();
    let at_cap = max_boards.is_some_and(|n| running.len() >= n);
    let usb_waiting = config.port.is_none() && config.transport == Transport::Auto && usb.iter().any(|p| !running.iter().any(|r| &r.target == p));
    for r in running.iter().filter(|r| r.is_wifi() && !r.info.stop.load(Ordering::Relaxed)) {
        let same_board = r.info.id().is_some_and(|id| ready_on_usb.contains(&id));
        if same_board || (at_cap && usb_waiting) {
            host(BridgeEvent::BoardLog { line: "host: the board is on USB, switching to it".into() });
            r.info.stop.store(true, Ordering::Relaxed);
        }
    }
}

/// Start a session for `target` on a thread of the scope.
fn start<'scope, 'env>(scope: &'scope Scope<'scope, 'env>, shared: &'env Shared<'env>, target: &str, number: u64) -> Option<Running<'scope>> {
    let info = Arc::new(SessionInfo { number, ..Default::default() });
    *info.label.lock().unwrap() = target.to_string();
    let sink = tagged(shared.sink, Some(target.to_string()), Some(info.clone()));
    let thread = {
        let (info, target) = (info.clone(), target.to_string());
        thread::Builder::new()
            .name(format!("companion-session-{number}"))
            .spawn_scoped(scope, move || session::run(&target, shared.config, shared.stop, &info, shared.latest, shared.targets, &sink))
            .ok()?
    };
    Some(Running { target: target.to_string(), info, started: Instant::now(), thread })
}

/// Collect the sessions that ended: say why, free the hub and the alerts of
/// their board, and set when to try their target again.
fn reap(running: &mut Vec<Running>, retry: &mut HashMap<String, Retry>, shared: &Shared) {
    let mut i = 0;
    while i < running.len() {
        if !running[i].thread.is_finished() {
            i += 1;
            continue;
        }
        let r = running.remove(i);
        let result = r.thread.join().unwrap_or_else(|_| Err(io::Error::other("the session panicked")));
        shared.targets.remove(r.info.number);
        shared.targets.floor.release(r.info.number);
        let label = r.info.label.lock().unwrap().clone();
        if let Some(hub) = &shared.config.hub {
            let why = result.as_ref().err().map_or_else(|| "the bridge stopped".to_string(), |e| format!("board disconnected: {e}"));
            hub.detach(&label, &why);
        }
        let slot = retry.entry(r.target.clone()).or_insert(Retry { at: Instant::now(), delay: Duration::from_secs(1) });
        match result {
            // Ended on purpose (its board went to USB, its port was paused, or stop): no pause.
            Ok(()) => {
                *slot = Retry { at: Instant::now(), delay: Duration::from_secs(1) };
                if !shared.stop.load(Ordering::Relaxed) {
                    let paused = shared.control.paused.lock().unwrap().contains(&r.target);
                    let reason = if paused { "the port is paused for another program" } else { "the board is on USB now" };
                    tagged(shared.sink, Some(r.target.clone()), Some(r.info.clone()))(BridgeEvent::Disconnected {
                        port: label.clone(),
                        reason: reason.into(),
                        permission_denied: false,
                    });
                }
            }
            Err(err) => {
                tagged(shared.sink, Some(r.target.clone()), Some(r.info.clone()))(BridgeEvent::Disconnected {
                    port: label,
                    reason: err.to_string(),
                    permission_denied: err.kind() == io::ErrorKind::PermissionDenied,
                });
                if r.started.elapsed() > Duration::from_secs(10) {
                    slot.delay = Duration::from_secs(1);
                }
                slot.at = Instant::now() + slot.delay;
                slot.delay = (slot.delay * 2).min(Duration::from_secs(15));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fixed_port_is_the_only_candidate() {
        let config = BridgeConfig { port: Some("/dev/ttyACM9".into()), ..Default::default() };
        assert_eq!(candidates(&config), (vec!["/dev/ttyACM9".to_string()], vec![], None));
        let config = BridgeConfig { port: Some("wifi:a1b2c3d4e5f6".into()), ..Default::default() };
        assert_eq!(candidates(&config), (vec![], vec!["wifi:a1b2c3d4e5f6".to_string()], None));
    }

    #[test]
    fn events_carry_their_session() {
        let got = Arc::new(Mutex::new(Vec::new()));
        let sink: ManagerSink = {
            let got = got.clone();
            Arc::new(move |e: ManagerEvent| got.lock().unwrap().push(serde_json::to_value(e).unwrap()))
        };
        let info = Arc::new(SessionInfo::default());
        let session = tagged(&sink, Some("/dev/ttyACM0".into()), Some(info.clone()));
        session(BridgeEvent::Connected { port: "/dev/ttyACM0".into() });
        *info.id.lock().unwrap() = Some("a1b2c3d4e5f6".into());
        *info.kind.lock().unwrap() = Some(BoardKind::Yoru);
        session(BridgeEvent::BoardLog { line: "hi".into() });
        tagged(&sink, None, None)(BridgeEvent::Waiting { reason: "none".into() });
        let got = got.lock().unwrap();
        assert_eq!(got[0]["kind"], "connected");
        assert_eq!(got[0]["target"], "/dev/ttyACM0");
        assert_eq!(got[0]["board"], serde_json::Value::Null);
        assert_eq!(got[1]["board"], "a1b2c3d4e5f6");
        assert_eq!(got[1]["board_kind"], "yoru");
        assert_eq!(got[2]["target"], serde_json::Value::Null);
        assert_eq!(got[2]["kind"], "waiting");
    }
}
