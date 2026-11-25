//! The meeting detection in the app (docs/meetings.md, Detection): every
//! few seconds, while it is on, [`Detector`] looks at the Mac's calendar and at
//! the apps on the microphone, and what it decides is done here: the
//! question on the Allow card (the notch opens for it, the board shows it),
//! the recording, and its end.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager};
use yoru_core::hostlog;
use yoru_core::meetings::detect::{self, Action, CalendarMeeting, Detector, MicUser, MicWatch};
use yoru_core::meetings::{self, Request};

use crate::AppState;
use crate::meetings as recordings;

const TICK: Duration = Duration::from_secs(5);
/// The calendar is read this often.
const CALENDAR_EVERY: Duration = Duration::from_secs(5 * 60);
/// A question waits this long for an answer.
const ASK_WAIT: Duration = Duration::from_secs(2 * 60);
/// The microphone's helper, if it ended, is started again after this.
const RESTART_AFTER: Duration = Duration::from_secs(60);

#[derive(Default)]
struct Shared {
    detector: Mutex<Detector>,
    users: Mutex<Vec<MicUser>>,
}

pub fn start(app: &AppHandle) {
    let app = app.clone();
    let _ = thread::Builder::new().name("yoru-meeting-detect".into()).spawn(move || run(app));
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn log(line: &str) {
    hostlog::line("meetings", line);
}

fn run(app: AppHandle) {
    let shared = Arc::new(Shared::default());
    let mut watch: Option<(MicWatch, Arc<AtomicBool>, Instant)> = None;
    let mut calendar_at: Option<Instant> = None;
    loop {
        thread::sleep(TICK);
        let settings = meetings::settings();
        if !settings.detect {
            watch = None;
            calendar_at = None;
            shared.users.lock().unwrap().clear();
            continue;
        }
        let dead = watch.as_ref().is_some_and(|(_, ended, at)| ended.load(Ordering::Relaxed) && at.elapsed() >= RESTART_AFTER);
        if watch.is_none() || dead {
            watch = None;
            let ended = Arc::new(AtomicBool::new(false));
            let (users, gone) = (shared.clone(), ended.clone());
            match MicWatch::start(move |apps| *users.users.lock().unwrap() = apps, move || gone.store(true, Ordering::Relaxed)) {
                Ok(w) => watch = Some((w, ended, Instant::now())),
                Err(e) => log(&format!("detection: the microphone's helper: {e}")),
            }
        }
        if calendar_at.is_none_or(|at| at.elapsed() >= CALENDAR_EVERY) {
            calendar_at = Some(Instant::now());
            if let Some(found) = calendar() {
                shared.detector.lock().unwrap().set_meetings(found);
            }
        }
        shared.detector.lock().unwrap().set_auto(settings.auto_record);
        let users = shared.users.lock().unwrap().clone();
        let action = shared.detector.lock().unwrap().step(now_ms(), &users, recordings::recording(&app));
        if let Some(action) = action {
            act(&app, &shared, action);
        }
    }
}

/// The meetings in the Mac's calendar now and in the next hours; `None`
/// without access to it (the microphone still works alone).
fn calendar() -> Option<Vec<CalendarMeeting>> {
    detect::calendar(now_ms()).map_err(|e| log(&format!("detection: the calendar: {e}"))).ok()
}

/// The words of the questions, in the language Yoru speaks.
struct Words {
    tool: &'static str,
    italian: bool,
}

impl Words {
    fn of(app: &AppHandle) -> Words {
        let italian = app.state::<AppState>().assistant.language().starts_with("it");
        Words { tool: if italian { "Riunione" } else { "Meeting" }, italian }
    }

    fn heads_up(&self, m: &CalendarMeeting) -> String {
        let (title, at) = (titled(&m.title), detect::clock(m.start));
        if self.italian { format!("{title} alle {at}: la registro quando parte la chiamata?") } else { format!("{title} at {at}: record it when the call starts?") }
    }

    fn call(&self, app: &MicUser, m: Option<&CalendarMeeting>) -> String {
        match (m, self.italian) {
            (Some(m), true) => format!("{} è iniziata su {}: la registro?", titled(&m.title), app.name),
            (Some(m), false) => format!("{} started on {}: record it?", titled(&m.title), app.name),
            (None, true) => format!("Chiamata su {}: la registro?", app.name),
            (None, false) => format!("A call on {}: record it?", app.name),
        }
    }
}

fn titled(title: &str) -> String {
    if title.is_empty() { "?".into() } else { format!("«{title}»") }
}

fn act(app: &AppHandle, shared: &Arc<Shared>, action: Action) {
    match action {
        Action::HeadsUp(m) => {
            log(&format!("detection: {} starts at {}, asking", m.title, detect::clock(m.start)));
            let text = Words::of(app).heads_up(&m);
            ask(app, shared, text, move |shared, _| shared.detector.lock().unwrap().armed(&m));
        }
        Action::Ask { app: caller, meeting } => {
            log(&format!("detection: {} took the microphone, asking", caller.name));
            let text = Words::of(app).call(&caller, meeting.as_ref());
            let title = meeting.map(|m| m.title);
            ask(app, shared, text, move |shared, app| record(app, shared, &caller, title));
        }
        Action::Record { app: caller, title } => {
            log(&format!("detection: {} took the microphone, recording as asked", caller.name));
            record(app, shared, &caller, Some(title));
        }
        Action::Stop => {
            log("detection: the call let the microphone go, stopping the recording");
            recordings::stop(app);
        }
    }
}

/// Ask on the Allow card, on a thread of its own; `yes` runs on Allow.
fn ask(app: &AppHandle, shared: &Arc<Shared>, text: String, yes: impl FnOnce(&Shared, &AppHandle) + Send + 'static) {
    let (app, shared) = (app.clone(), shared.clone());
    let tool = Words::of(&app).tool;
    thread::spawn(move || {
        let confirmations = app.state::<AppState>().confirmations.clone();
        let answer = confirmations.ask(tool, &text, Some(&text), ASK_WAIT);
        log(&format!("detection: {}", match answer {
            Some(true) => "allowed",
            Some(false) => "denied",
            None => "no answer",
        }));
        if answer == Some(true) {
            yes(&shared, &app);
        }
        shared.detector.lock().unwrap().answered();
    });
}

/// Record `caller`'s sound and the microphone.
fn record(app: &AppHandle, shared: &Shared, caller: &MicUser, title: Option<String>) {
    if recordings::recording(app) {
        return;
    }
    let request = Request { system: false, app: Some(caller.id.clone()), microphones: vec!["default".into()], title: title.filter(|t| !t.is_empty()) };
    match recordings::record(app, request) {
        Ok(_) => shared.detector.lock().unwrap().following(&caller.id),
        Err(e) => log(&format!("detection: recording {}: {e}", caller.name)),
    }
}
