//! Meetings in the app (docs/meetings.md): what can be recorded, the
//! recording that runs, the meetings kept, their transcripts, and the
//! events that keep the Meetings page current (`meeting`: levels, errors,
//! the end, the transcription's progress).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, State};
use yoru_core::llm::{self, Llm, LlmConfig};
use yoru_core::meetings::{self, Event, Format, Meeting, Recording, Request, Settings, Sources, Summary, SummaryBy, Transcript, When};

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
    if app.state::<Meetings>().active.lock().unwrap().is_some() {
        return Err("a recording is already running".into());
    }
    let events = app.clone();
    let recording = blocking(move || {
        Recording::start(&request, move |event| {
            let done = match &event {
                Event::Done { meeting } => Some(meeting.id.clone()),
                _ => None,
            };
            if done.is_some() {
                events.state::<Meetings>().active.lock().unwrap().take();
            }
            let _ = events.emit("meeting", &event);
            if let Some(id) = done {
                after_recording(&events, id);
            }
        })
    })
    .await?;
    let meeting = recording.meeting();
    *app.state::<Meetings>().active.lock().unwrap() = Some(recording);
    Ok(meeting)
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

/// Transcribe a new recording or import on its own, unless the settings
/// say to wait to be asked or the model isn't there yet.
fn after_recording(app: &AppHandle, id: String) {
    if meetings::settings().transcribe == When::Manual {
        return;
    }
    let app = app.clone();
    thread::spawn(move || {
        if meetings::model_present() {
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
        let language = Some(settings.language).filter(|l| !l.is_empty());
        let result = meetings::transcribe(&id, language.as_deref(), settings.speakers, |track, phase, fraction| {
            let event = Event::Progress { id: id.clone(), track: track.into(), phase: phase.into(), fraction };
            let _ = app.emit("meeting", &event);
        });
        state.queued.lock().unwrap().remove(&id);
        let event = match result {
            Ok(meeting) => Event::Transcribed { meeting },
            Err(e) => Event::Failed { id: id.clone(), message: e.to_string() },
        };
        let transcribed = matches!(event, Event::Transcribed { .. });
        let _ = app.emit("meeting", &event);
        drop(_turn);
        if transcribed && meetings::settings().auto_summary && summarizer(&app).is_ok() {
            queue_summary(&app, id);
        }
    });
}

/// Put meeting `id` in line for a summary, after any transcript being made.
fn queue_summary(app: &AppHandle, id: String) {
    let key = format!("summary:{id}");
    if !app.state::<Meetings>().queued.lock().unwrap().insert(key.clone()) {
        return;
    }
    let app = app.clone();
    thread::spawn(move || {
        let state = app.state::<Meetings>();
        let _turn = state.worker.lock().unwrap();
        let result = summarizer(&app).and_then(|model| {
            let (you, person) = if crate::app_language(&app.state::<AppState>()).starts_with("it") { ("Tu", "Persona") } else { ("You", "Person") };
            let progress = |fraction| {
                let _ = app.emit("meeting", &Event::Summarizing { id: id.clone(), fraction });
            };
            let complete = |prompt: &str, schema: &Value| model.complete(&app, prompt, schema);
            let summary = meetings::summarize(&id, you, person, &model.label, model.chunk, &complete, progress);
            model.done();
            summary.map_err(|e| e.to_string())
        });
        state.queued.lock().unwrap().remove(&key);
        let event = match result.and_then(|summary| meetings::get(&id).map(|meeting| (meeting, summary)).map_err(|e| e.to_string())) {
            Ok((meeting, summary)) => Event::Summarized { meeting, summary },
            Err(message) => Event::SummaryFailed { id, message },
        };
        let _ = app.emit("meeting", &event);
    });
}

/// What writes a summary: the local model or the assistant's brain.
struct Summarizer {
    /// For people: "Qwen3.5 4B (this computer)", "OpenAI gpt-6-luna".
    label: String,
    /// The transcript's characters it reads at once: the local model's
    /// context is 8K tokens, Groq's free tier counts 8K a minute.
    chunk: usize,
    /// A local model started only for this, to stop after.
    own: Option<Arc<Llm>>,
    /// The voice's local model, when it runs one: only one llama-server
    /// at a time (they share a pid file).
    shared: Option<Arc<yoru_core::Agent>>,
    brain: bool,
}

impl Summarizer {
    fn complete(&self, app: &AppHandle, prompt: &str, schema: &Value) -> Result<String, String> {
        if self.brain {
            return app.state::<AppState>().assistant.complete_json(prompt, schema);
        }
        let llm = self.own.as_deref().or(self.shared.as_ref().map(|a| a.llm())).ok_or("no local model")?;
        let request = json!({
            "messages": [{"role": "user", "content": prompt}],
            "response_format": {"type": "json_object", "schema": schema},
            "max_tokens": 2048,
            "temperature": 0.3,
        });
        let message = llm.chat(&request).map_err(|e| e.to_string())?;
        Ok(message["content"].as_str().unwrap_or_default().to_string())
    }

    fn done(&self) {
        if let Some(llm) = &self.own {
            llm.shutdown();
        }
    }
}

/// The summarizer the settings ask for, or why there is none.
fn summarizer(app: &AppHandle) -> Result<Summarizer, String> {
    let state = app.state::<AppState>();
    match meetings::settings().summary_by {
        SummaryBy::Brain => {
            let settings = state.assistant.settings();
            let brain = settings.brain().filter(|b| b.has_key()).ok_or("no brain is set up (Settings, Brain)")?;
            let chunk = if matches!(brain, yoru_core::assistant::Brain::Groq | yoru_core::assistant::Brain::Ollama) { 12_000 } else { 120_000 };
            Ok(Summarizer { label: format!("{} {}", brain.name(), settings.model_of(brain)), chunk, own: None, shared: None, brain: true })
        }
        SummaryBy::Local => {
            let shared = state.voice.lock().unwrap().agent.clone().filter(|a| a.llm().config().is_local());
            let (own, model) = match &shared {
                Some(agent) => (None, agent.llm().config().model_path().map(|p| p.to_path_buf())),
                None => {
                    let local = llm::local_fallback().ok_or("no local language model: download Qwen3.5 in the meetings' settings")?;
                    let model = Some(local.model.clone());
                    (Some(Arc::new(Llm::new(LlmConfig::Local(local)))), model)
                }
            };
            let name = model.and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).unwrap_or_default();
            Ok(Summarizer { label: name, chunk: 12_000, own, shared, brain: false })
        }
    }
}

#[tauri::command]
pub fn meeting_summarize(app: AppHandle, id: String) {
    queue_summary(&app, id);
}

#[tauri::command]
pub fn meeting_summary(id: String) -> Result<Option<Summary>, String> {
    meetings::summary(&id).map_err(|e| e.to_string())
}

/// What can summarize: whether the local model is there, and the brain's name.
#[derive(Serialize)]
pub struct Summarizers {
    local: bool,
    brain: Option<String>,
}

#[tauri::command]
pub fn meeting_summarizers(state: State<'_, AppState>) -> Summarizers {
    let settings = state.assistant.settings();
    Summarizers {
        local: llm::local_fallback().is_some() || state.voice.lock().unwrap().agent.as_ref().is_some_and(|a| a.llm().config().is_local()),
        brain: settings.brain().filter(|b| b.has_key()).map(|b| format!("{} · {}", b.name(), settings.model_of(b))),
    }
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
pub fn rename_speaker(id: String, key: String, name: String) -> Result<Transcript, String> {
    meetings::rename_speaker(&id, &key, &name).map_err(|e| e.to_string())
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
