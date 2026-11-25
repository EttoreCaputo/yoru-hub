//! iCloud sync between Macs (docs/icloud.md): the bridge between the core's
//! stores ([`yoru_core::assistant::synced::stores`]) and the CloudKit engine
//! in `audio/Sources/YoruSync`, linked into the app. Every few seconds what
//! changed here goes out; what comes in is applied to the stores, and the
//! pages are told.

use std::collections::BTreeMap;
use std::ffi::{CStr, CString, c_char};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use yoru_core::sync::{Field, Record, Store};

use crate::AppState;

/// How often what changed here is looked for.
const PUSH_EVERY: Duration = Duration::from_secs(5);
/// How often iCloud is asked, on top of its pushes.
const FETCH_EVERY: Duration = Duration::from_secs(300);
/// Records per message to the engine.
const BATCH: usize = 200;

unsafe extern "C" {
    fn yoru_sync_start(dir: *const c_char, zones: *const c_char, callback: extern "C" fn(*const c_char, *const c_char));
    fn yoru_sync_save(json: *const c_char);
    fn yoru_sync_fetch();
    fn yoru_sync_account();
    fn yoru_sync_entitled() -> bool;
}

/// A record as the engine takes it: the fields as one JSON string.
#[derive(Serialize, Deserialize)]
struct Wire {
    zone: String,
    kind: String,
    uid: String,
    changed: i64,
    by: String,
    deleted: bool,
    data: String,
}

impl From<&Record> for Wire {
    fn from(r: &Record) -> Self {
        Self {
            zone: r.zone.clone(),
            kind: r.kind.clone(),
            uid: r.uid.clone(),
            changed: r.changed,
            by: r.by.clone(),
            deleted: r.deleted,
            data: serde_json::to_string(&r.fields).unwrap_or_default(),
        }
    }
}

impl From<Wire> for Record {
    fn from(w: Wire) -> Self {
        let fields: BTreeMap<String, Field> = serde_json::from_str(&w.data).unwrap_or_default();
        Self { zone: w.zone, kind: w.kind, uid: w.uid, changed: w.changed, by: w.by, deleted: w.deleted, fields }
    }
}

/// What the Settings page shows.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Status {
    pub running: bool,
    /// The build carries the iCloud entitlement (a signed build with the profile).
    pub entitled: bool,
    /// `available`, `none`, `restricted`, `unavailable`, `out`, `switched`…
    pub account: String,
    /// Unix ms of the last records sent and received.
    pub last_sent: i64,
    pub last_fetched: i64,
    pub sent: u64,
    pub received: u64,
    pub errors: Vec<String>,
    /// Unix ms of the last error: older than the last records sent or
    /// received, it's over.
    pub last_error: i64,
    /// This Mac's id.
    pub device: String,
    /// The Mac that fires timers, tasks and watchers, and whether it's this one.
    pub doer: String,
    pub is_doer: bool,
}

struct Sync {
    stores: Vec<Arc<dyn Store>>,
    doer: Arc<yoru_core::doer::Doer>,
    app: AppHandle,
    status: Mutex<Status>,
}

static SYNC: OnceLock<Sync> = OnceLock::new();
/// Turned off while running: nothing goes out or comes in until it's on
/// again (the engine itself stays up until the app quits).
static PAUSED: AtomicBool = AtomicBool::new(false);

pub fn set_paused(paused: bool) {
    PAUSED.store(paused, Ordering::SeqCst);
    if let Some(sync) = SYNC.get() {
        sync.status.lock().unwrap().running = !paused;
        sync.doer.set_active(!paused);
    }
    if !paused {
        fetch_now();
    }
}

/// Whether this build can sync at all.
pub fn entitled() -> bool {
    cfg!(target_os = "macos") && unsafe { yoru_sync_entitled() }
}

/// The status, for the Settings page.
pub fn status() -> Status {
    match SYNC.get() {
        Some(s) => {
            let mut st = s.status.lock().unwrap().clone();
            st.doer = s.doer.holder().0;
            st.is_doer = yoru_core::doer::is_doer();
            st
        }
        None => Status { entitled: entitled(), ..Default::default() },
    }
}

/// Start syncing, once per run; later calls do nothing. Off until the user
/// turns it on (`Settings::icloud`), and only in a build that may.
pub fn start(app: &AppHandle) {
    if SYNC.get().is_some() || !entitled() {
        return;
    }
    let state = app.state::<AppState>();
    let Some(dir) = state.assistant.dir().cloned() else { return };
    let mut stores = yoru_core::assistant::synced::stores(&state.assistant, Some(state.watchers.clone()));
    let device = yoru_core::sync::device(Some(&dir));
    let doer = yoru_core::doer::doer(|| device.clone());
    stores.push(doer.clone());
    let status = Status { running: true, entitled: true, device, ..Default::default() };
    if SYNC.set(Sync { stores, doer: doer.clone(), app: app.clone(), status: Mutex::new(status) }).is_err() {
        return;
    }
    doer.set_active(!PAUSED.load(Ordering::SeqCst));
    let sync_dir = dir.join("sync");
    let _ = std::fs::create_dir_all(&sync_dir);
    let (Ok(path), Ok(zones)) = (CString::new(sync_dir.to_string_lossy().as_bytes()), CString::new(yoru_core::assistant::synced::ZONES.join(","))) else { return };
    unsafe {
        yoru_sync_start(path.as_ptr(), zones.as_ptr(), on_event);
        yoru_sync_account();
        yoru_sync_fetch();
    }
    std::thread::Builder::new().name("icloud".into()).spawn(run).ok();
    log(&format!("started ({})", Path::new(&sync_dir).display()));
}

/// The user is at this Mac: with no board anywhere, it becomes the doer.
pub fn used_now() {
    if let Some(sync) = SYNC.get() {
        sync.doer.used_now();
    }
}

/// Ask iCloud now (the window came forward, the Mac woke up).
pub fn fetch_now() {
    if SYNC.get().is_some() && !PAUSED.load(Ordering::SeqCst) {
        unsafe { yoru_sync_fetch() };
    }
}

fn run() {
    let Some(sync) = SYNC.get() else { return };
    let mut fetched = Instant::now();
    let mut was_doer = None;
    loop {
        std::thread::sleep(PUSH_EVERY);
        if PAUSED.load(Ordering::SeqCst) {
            continue;
        }
        let board = {
            let state = sync.app.state::<AppState>();
            let devices = state.devices.lock().unwrap();
            devices.boards.values().any(|l| l.kind == "connected")
        };
        sync.doer.set_board(board);
        sync.doer.tick(yoru_core::sync::now_ms());
        let doer = yoru_core::doer::is_doer();
        if was_doer != Some(doer) {
            was_doer = Some(doer);
            let holder = sync.doer.holder().0;
            log(&match (doer, holder.is_empty()) {
                (true, _) => "this Mac is the doer".to_string(),
                (false, true) => "waiting to hear from iCloud before claiming the doer".to_string(),
                (false, false) => format!("the doer is {holder}"),
            });
        }
        for store in &sync.stores {
            let records = store.changes(BATCH);
            if records.is_empty() {
                continue;
            }
            let wire: Vec<Wire> = records.iter().map(Wire::from).collect();
            if let Ok(json) = serde_json::to_string(&wire).map(CString::new)
                && let Ok(json) = json
            {
                unsafe { yoru_sync_save(json.as_ptr()) };
            }
        }
        if fetched.elapsed() >= FETCH_EVERY {
            fetched = Instant::now();
            unsafe { yoru_sync_fetch() };
        }
    }
}

/// From the engine, on its own threads.
extern "C" fn on_event(event: *const c_char, text: *const c_char) {
    let (Some(sync), false, false) = (SYNC.get(), event.is_null(), text.is_null()) else { return };
    if PAUSED.load(Ordering::SeqCst) {
        return;
    }
    let event = unsafe { CStr::from_ptr(event) }.to_string_lossy().into_owned();
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned();
    let records = || -> Vec<Record> { serde_json::from_str::<Vec<Wire>>(&text).unwrap_or_default().into_iter().map(Record::from).collect() };
    let now = yoru_core::sync::now_ms();
    match event.as_str() {
        "fetched" => {
            let records = records();
            let changed: usize = sync.stores.iter().map(|s| s.apply(&records)).sum();
            {
                let mut st = sync.status.lock().unwrap();
                st.received += records.len() as u64;
                st.last_fetched = now;
            }
            if changed > 0 {
                let mut kinds: Vec<&str> = records.iter().map(|r| r.kind.as_str()).collect();
                kinds.sort_unstable();
                kinds.dedup();
                log(&format!("{changed} of {} records applied ({})", records.len(), kinds.join(", ")));
                tell_pages(&sync.app, &kinds);
            }
        }
        "sent" => {
            let records = records();
            for store in &sync.stores {
                store.sent(&records);
            }
            log(&format!("{} records sent", records.len()));
            let mut st = sync.status.lock().unwrap();
            st.sent += records.len() as u64;
            st.last_sent = now;
        }
        "status" if text == "fetched" => sync.doer.heard(),
        "account" => {
            log(&format!("account: {text}"));
            sync.status.lock().unwrap().account = text;
        }
        "error" => {
            log(&text);
            let mut st = sync.status.lock().unwrap();
            st.errors.push(text);
            st.last_error = now;
            let excess = st.errors.len().saturating_sub(20);
            st.errors.drain(..excess);
        }
        _ => {}
    }
    let _ = sync.app.emit("icloud", status());
}

/// The pages that show what came in read it again.
fn tell_pages(app: &AppHandle, kinds: &[&str]) {
    let state = app.state::<AppState>();
    if kinds.contains(&"notification") {
        let _ = app.emit("inbox", state.assistant.inbox.list());
    }
    if kinds.iter().any(|k| ["setting", "connector", "skill"].contains(k)) {
        let _ = app.emit("assistant", crate::assistant::info_of(&state.assistant));
    }
    if kinds.contains(&"chat") {
        let _ = app.emit("conversations", state.assistant.conversations());
    }
    if kinds.contains(&"task") {
        let _ = app.emit("tasks", state.assistant.tasks.list());
    }
    // The history index follows the conversations and meetings that came.
    if kinds.iter().any(|k| ["chat", "meeting"].contains(k)) {
        let assistant = state.assistant.clone();
        std::thread::spawn(move || assistant.sync_history());
    }
    let _ = app.emit("synced", kinds);
}

fn log(line: &str) {
    yoru_core::hostlog::line("icloud", line);
}
