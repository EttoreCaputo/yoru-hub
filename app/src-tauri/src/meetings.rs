//! Meetings in the app (docs/meetings.md): what can be recorded, the
//! recording that runs, the meetings kept, their transcripts, and the
//! events that keep the Meetings page current (`meeting`: levels, errors,
//! the end, the transcription's progress).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use yoru_core::assistant::{self, Brain};
use yoru_core::meetings::{self, Event, Format, Meeting, Recording, Request, Settings, Sources, Summary, Transcript, When};

use crate::AppState;

/// The one recording at a time, and the transcriptions waiting or running.
#[derive(Default)]
pub struct Meetings {
    active: Mutex<Option<Recording>>,
    /// Meetings queued for a transcript, the running one included.
    queued: Mutex<BTreeSet<String>>,
    /// Held while a transcript is made: one at a time, the Neural Engine is one.
    worker: Mutex<()>,
    downloading: Mutex<bool>,
}

impl Meetings {
    /// The app is quitting: stop, and give the tracks a moment to be finalized.
    pub fn shutdown(&self) {
        if let Some(recording) = self.active.lock().unwrap().take() {
            recording.stop_and_wait(Duration::from_secs(5));
        }
    }
}

/// What every board shows of the meetings: `state` as
/// [`meetings::BoardMeeting::set`] takes it.
fn board(app: &AppHandle, state: &str, source: Option<&str>) {
    app.state::<AppState>().assistant.meeting.set(state, source);
}

/// The recording of `request` started: the boards show it with the app's
/// name and icon (looked up on a thread: `sources` asks the helper).
fn board_recording(app: &AppHandle, request: &Request) {
    if request.system || request.app.is_none() {
        board(app, "recording", Some("Computer"));
        return;
    }
    let (app, id) = (app.clone(), request.app.clone().unwrap_or_default());
    thread::spawn(move || {
        let found = meetings::sources().ok().and_then(|s| s.apps.into_iter().find(|a| a.id == id));
        let name = found.as_ref().map_or_else(|| id.rsplit('.').next().unwrap_or(&id).to_string(), |a| a.name.clone());
        if let Some(b64) = found.and_then(|a| a.icon) {
            yoru_core::icons::remember_base64(&name, &b64);
        }
        board(&app, "recording", Some(&name));
    });
}

/// At launch: finalize what an app that quit while recording left behind.
pub fn start(app: &AppHandle) {
    app.manage(Meetings::default());
    let _ = thread::Builder::new().name("yoru-meetings-recover".into()).spawn(meetings::recover);
}

fn blocking<T: Send + 'static>(job: impl FnOnce() -> std::io::Result<T> + Send + 'static) -> impl std::future::Future<Output = Result<T, String>> {
    async move { tauri::async_runtime::spawn_blocking(job).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string()) }
}

#[tauri::command]
pub async fn meeting_sources() -> Result<Sources, String> {
    blocking(meetings::sources).await
}

#[tauri::command]
pub fn meetings_list() -> Vec<Meeting> {
    meetings::list()
}

/// The recording that runs, if one does.
#[tauri::command]
pub fn meeting_active(state: State<'_, Meetings>) -> Option<Meeting> {
    state.active.lock().unwrap().as_ref().map(Recording::meeting)
}

/// Start recording; returns once every track runs (macOS may ask the user
/// for the microphone first).
#[tauri::command]
pub async fn meeting_start(app: AppHandle, request: Request) -> Result<Meeting, String> {
    tauri::async_runtime::spawn_blocking(move || record(&app, request)).await.map_err(|e| e.to_string())?
}

/// Start recording what `request` asks for, waiting until every track runs:
/// the Meetings page's Record, and the meeting detection's.
pub fn record(app: &AppHandle, request: Request) -> Result<Meeting, String> {
    if app.state::<Meetings>().active.lock().unwrap().is_some() {
        return Err("a recording is already running".into());
    }
    let events = app.clone();
    let shown = request.clone();
    let recording = Recording::start(&request, move |event| {
        let done = match &event {
            Event::Done { meeting } => Some(meeting.id.clone()),
            _ => None,
        };
        if done.is_some() {
            events.state::<Meetings>().active.lock().unwrap().take();
            board(&events, "idle", None);
        }
        let _ = events.emit("meeting", &event);
        if let Some(id) = done {
            after_recording(&events, id);
        }
    })
    .map_err(|e| e.to_string())?;
    let meeting = recording.meeting();
    *app.state::<Meetings>().active.lock().unwrap() = Some(recording);
    board_recording(app, &shown);
    let _ = app.emit("meeting", &Event::Started { meeting: meeting.clone() });
    Ok(meeting)
}

/// Whether a recording runs.
pub fn recording(app: &AppHandle) -> bool {
    app.state::<Meetings>().active.lock().unwrap().is_some()
}

/// Stop the recording that runs, if one does.
pub fn stop(app: &AppHandle) {
    if let Some(recording) = app.state::<Meetings>().active.lock().unwrap().as_ref() {
        recording.stop();
    }
}

/// Stop the recording; a `meeting` event with `done` follows.
#[tauri::command]
pub fn meeting_stop(state: State<'_, Meetings>) {
    if let Some(recording) = state.active.lock().unwrap().as_ref() {
        recording.stop();
    }
}

#[tauri::command]
pub fn rename_meeting(id: String, title: String) -> Result<Meeting, String> {
    meetings::rename(&id, &title).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_meeting(id: String) -> Result<Vec<Meeting>, String> {
    meetings::delete(&id).map_err(|e| e.to_string())?;
    Ok(meetings::list())
}

/// System Settings at the system audio permission (Privacy & Security,
/// Screen & System Audio Recording), for a recording that gets no sound.
#[tauri::command]
pub fn open_audio_privacy() -> Result<(), String> {
    std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AudioCapture")
        .status()
        .map_err(|e| e.to_string())
        .map(|_| ())
}

/// A recording brought in by something else (the Pocket): transcribed
/// like an import.
pub fn after_import(app: &AppHandle, id: String) {
    after_recording(app, id);
}

/// Transcribe a new recording or import on its own, unless the settings
/// say to wait to be asked or the model isn't there yet.
fn after_recording(app: &AppHandle, id: String) {
    if meetings::settings().transcribe == When::Manual {
        return;
    }
    let app = app.clone();
    thread::spawn(move || {
        if meetings::settings().gemini().is_some() || meetings::model_present() {
            queue(&app, id);
        }
    });
}

/// Put meeting `id` in line for a transcript, made on a thread of its own.
fn queue(app: &AppHandle, id: String) {
    if !app.state::<Meetings>().queued.lock().unwrap().insert(id.clone()) {
        return;
    }
    let app = app.clone();
    thread::spawn(move || {
        let state = app.state::<Meetings>();
        let _turn = state.worker.lock().unwrap();
        let settings = meetings::settings();
        board(&app, "transcribing", None);
        let language = Some(settings.language.as_str()).filter(|l| !l.is_empty());
        let result = if settings.transcribe_by != "local" && settings.gemini().is_none() {
            Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "unsupported meeting transcription service"))
        } else {
            meetings::transcribe(&id, settings.gemini(), language, settings.speakers, |track, phase, fraction| {
                let event = Event::Progress { id: id.clone(), track: track.into(), phase: phase.into(), fraction };
                let _ = app.emit("meeting", &event);
            })
        };
        state.queued.lock().unwrap().remove(&id);
        let event = match result {
            Ok(meeting) => {
                learn_voices(&app, &id, None);
                Event::Transcribed { meeting }
            }
            Err(e) => Event::Failed { id: id.clone(), message: e.to_string() },
        };
        let transcribed = matches!(event, Event::Transcribed { .. });
        let _ = app.emit("meeting", &event);
        drop(_turn);
        if transcribed && meetings::settings().auto_summary && summarizer(&app).is_ok() {
            queue_summary(&app, id, None);
        } else {
            board(&app, if transcribed { "done" } else { "failed" }, None);
        }
    });
}

/// Put meeting `id` in line for a summary, after any transcript being made,
/// in `style` (`meetings::STYLES`), else the settings' style.
fn queue_summary(app: &AppHandle, id: String, style: Option<String>) {
    let key = format!("summary:{id}");
    if !app.state::<Meetings>().queued.lock().unwrap().insert(key.clone()) {
        return;
    }
    let app = app.clone();
    thread::spawn(move || {
        let state = app.state::<Meetings>();
        let _turn = state.worker.lock().unwrap();
        board(&app, "summarizing", None);
        // The agent reads the transcript and saves the summary with the
        // app's tools (the summarize-meeting skill); its steps aren't
        // counted, so the bar shows only that it started.
        let _ = app.emit("meeting", &Event::Summarizing { id: id.clone(), fraction: 0.1 });
        let style = style.unwrap_or_else(|| meetings::settings().summary_style);
        let result = summarizer(&app).and_then(|_| app.state::<AppState>().assistant.summarize_meeting(&id, &style));
        state.queued.lock().unwrap().remove(&key);
        let event = match result.and_then(|summary| meetings::get(&id).map(|meeting| (meeting, summary)).map_err(|e| e.to_string())) {
            Ok((meeting, summary)) => {
                // Its decisions and to-dos go to the memory, at the next sleep.
                app.state::<AppState>().assistant.learn_meeting(meeting.title.as_deref().unwrap_or(&summary.title), &summary.for_memory());
                Event::Summarized { meeting, summary }
            }
            Err(message) => Event::SummaryFailed { id, message },
        };
        board(&app, if matches!(event, Event::Summarized { .. }) { "done" } else { "failed" }, None);
        let _ = app.emit("meeting", &event);
    });
}

/// Whether the agent can summarize: a brain with its key, or why not.
fn summarizer(app: &AppHandle) -> Result<(), String> {
    let settings = app.state::<AppState>().assistant.settings();
    settings.brain().filter(|b| b.has_key()).map(|_| ()).ok_or_else(|| "no brain is set up (Settings, Brain)".to_string())
}

#[tauri::command]
pub fn meeting_summarize(app: AppHandle, id: String, style: Option<String>) {
    queue_summary(&app, id, style);
}

/// Every summarized meeting's to-dos, for the To-do page.
#[tauri::command]
pub fn meeting_todos() -> Vec<meetings::Todo> {
    meetings::todos()
}

#[tauri::command]
pub fn set_meeting_todo(id: String, text: String, done: bool) -> Result<Vec<meetings::Todo>, String> {
    meetings::set_todo_done(&id, &text, done).map_err(|e| e.to_string())?;
    Ok(meetings::todos())
}

#[tauri::command]
pub fn meeting_summary(id: String) -> Result<Option<Summary>, String> {
    meetings::summary(&id).map_err(|e| e.to_string())
}

/// What can summarize and transcribe: the brain's name (the agent writes
/// the summaries), and every model of the providers with a key (and
/// Ollama's, if running), for the transcriber.
#[derive(Serialize)]
pub struct Summarizers {
    brain: Option<String>,
    models: Vec<SummaryModel>,
}

/// A model that can summarize: `id` is the setting's value (`groq:qwen/…`).
#[derive(Serialize)]
pub struct SummaryModel {
    id: String,
    provider: &'static str,
    model: String,
    note: String,
}

#[tauri::command]
pub async fn meeting_summarizers(app: AppHandle) -> Result<Summarizers, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let settings = state.assistant.settings();
        let models = Brain::ALL
            .into_iter()
            .filter(|b| b.has_key())
            .flat_map(|b| assistant::chat_models(b).into_iter().map(move |(model, note)| SummaryModel { id: format!("{}:{model}", b.id()), provider: b.name(), model, note }))
            .collect();
        Summarizers {
            brain: settings.brain().filter(|b| b.has_key()).map(|b| format!("{} · {}", b.name(), settings.model_of(b))),
            models,
        }
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn meeting_transcribe(app: AppHandle, id: String) {
    queue(&app, id);
}

/// The meetings waiting for or getting a transcript.
#[tauri::command]
pub fn meetings_transcribing(state: State<'_, Meetings>) -> Vec<String> {
    state.queued.lock().unwrap().iter().cloned().collect()
}

#[tauri::command]
pub fn meeting_transcript(id: String) -> Result<Option<Transcript>, String> {
    meetings::transcript(&id).map_err(|e| e.to_string())
}

/// The transcript in Downloads; the unnamed speakers are `you` or `person`
/// and a number, in the app's language.
#[tauri::command]
pub fn meeting_export(id: String, format: Format, you: String, person: String) -> Result<String, String> {
    meetings::export(&id, format, &you, &person).map(|p| p.display().to_string()).map_err(|e| e.to_string())
}

/// Name speaker `key` of meeting `id` (empty: back to their number).
#[tauri::command]
pub fn rename_speaker(app: AppHandle, id: String, key: String, name: String) -> Result<Transcript, String> {
    let transcript = meetings::rename_speaker(&id, &key, &name).map_err(|e| e.to_string())?;
    if !name.trim().is_empty() {
        learn_voices(&app, &id, Some(key));
    }
    Ok(transcript)
}

/// Learn the voices heard in meeting `id` (only `key`'s, when given), on a
/// thread of its own: Yoru knows them when they talk to him.
fn learn_voices(app: &AppHandle, id: &str, key: Option<String>) {
    let state = app.state::<AppState>();
    let Some(printer) = state.voiceprinter.clone() else { return };
    let (assistant, id) = (state.assistant.clone(), id.to_string());
    thread::spawn(move || assistant.learn_voices(&id, &printer, key.as_deref()));
}

#[tauri::command]
pub fn meeting_settings() -> Settings {
    meetings::settings()
}

#[tauri::command]
pub fn set_meeting_settings(settings: Settings) -> Result<Settings, String> {
    meetings::set_settings(&settings).map_err(|e| e.to_string())?;
    Ok(settings)
}

/// Whether the transcription model is downloaded.
#[tauri::command]
pub async fn meeting_model() -> Result<bool, String> {
    blocking(|| Ok(meetings::model_present())).await
}

/// Fetch the transcription model; `meeting` events with `model` report it.
#[tauri::command]
pub async fn meeting_model_download(app: AppHandle) -> Result<(), String> {
    {
        let state = app.state::<Meetings>();
        let mut downloading = state.downloading.lock().unwrap();
        if *downloading {
            return Err("already downloading".into());
        }
        *downloading = true;
    }
    let events = app.clone();
    let result = blocking(move || {
        meetings::download_model(|fraction| {
            let _ = events.emit("meeting", &Event::Model { fraction });
        })
    })
    .await;
    *app.state::<Meetings>().downloading.lock().unwrap() = false;
    result
}

/// An audio or video file as a new meeting: `path`, or one picked in a
/// Finder dialog when there is none (`None` when the user cancels).
#[tauri::command]
pub async fn meeting_import(app: AppHandle, path: Option<String>) -> Result<Option<Meeting>, String> {
    let path = match path {
        Some(p) => PathBuf::from(p),
        None => match blocking(pick_file).await? {
            Some(p) => p,
            None => return Ok(None),
        },
    };
    let meeting = blocking(move || meetings::import(&path)).await?;
    after_recording(&app, meeting.id.clone());
    Ok(Some(meeting))
}

/// Finder's open dialog for audio and video, through AppleScript.
fn pick_file() -> std::io::Result<Option<PathBuf>> {
    let out = std::process::Command::new("osascript")
        .args(["-e", r#"POSIX path of (choose file of type {"public.audio", "public.movie"})"#])
        .output()?;
    // Cancel exits with an error (-128).
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok((out.status.success() && !path.is_empty()).then(|| PathBuf::from(path)))
}

/// A track's file, as bytes rather than a JSON list of numbers.
#[tauri::command]
pub async fn meeting_audio(id: String, track: String) -> Result<tauri::ipc::Response, String> {
    blocking(move || meetings::audio(&id, &track)).await.map(tauri::ipc::Response::new)
}
