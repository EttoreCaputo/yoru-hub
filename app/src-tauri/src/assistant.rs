//! The assistant's side of the app: Yoru's answers, inbox, memory, routines,
//! watchers and connectors, as commands for the UI and the events that keep
//! it current (`inbox`, `chat`, `schedule`, `connectors`).
//!
//! Everything the assistant does that can take a while (a turn of the model,
//! a sign-in, a watcher) runs on a blocking thread, never on Tauri's.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};
use yoru_core::assistant::watchers::{self, ScheduleEvent, WatchEntry, Watcher};
use yoru_core::assistant::{self, Assistant, Brain, CONFIRM_WAIT, Channel, ChatLine, Host, Notification, Reply, Settings, TaskHost};
use yoru_core::claude::agents::{Agents, Decision};
use yoru_core::cloud::{self, KeySource, Provider};
use yoru_core::connectors::catalog::{self, Preset};
use yoru_core::connectors::{ConnectorConfig, ConnectorEvent, ConnectorStatus, Connectors};
use yoru_core::{Board, BoardKind, Tool};

use crate::AppState;

/// How long "do not disturb" from the tray lasts.
const DND: Duration = Duration::from_secs(60 * 60);

/// The connectors and the scheduler, started with the app.
#[derive(Default)]
pub struct Services {
    pub connectors: Mutex<Option<Arc<Connectors>>>,
    /// Why the connectors couldn't start (no data folder, a broken file).
    pub connectors_error: Mutex<Option<String>>,
    pub stop: Arc<AtomicBool>,
    scheduler: Mutex<Option<thread::JoinHandle<()>>>,
}

impl Services {
    fn connectors(&self) -> Result<Arc<Connectors>, String> {
        self.connectors.lock().unwrap().clone().ok_or_else(|| {
            self.connectors_error.lock().unwrap().clone().unwrap_or_else(|| "the connectors aren't running".into())
        })
    }

    /// Stop the scheduler and wait for it (it checks every 200 ms).
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.scheduler.lock().unwrap().take() {
            let _ = t.join();
        }
        self.connectors.lock().unwrap().take();
    }
}

/// The app's way for the assistant to reach the board and ask before acting:
/// the card goes through Claude Code's approvals, so it shows on the board and
/// in the app at once.
pub struct AppHost {
    pub board: Board,
    pub agents: Arc<Agents>,
}

impl Host for AppHost {
    fn board_tools(&self) -> Vec<Tool> {
        // `set_mic` is the user's: the assistant mustn't turn the microphone off.
        self.board.list_tools().unwrap_or_default().into_iter().filter(|t| !BoardKind::hub_only(&t.name) && t.name != "set_mic").collect()
    }

    fn call_board(&self, name: &str, arguments: &Value) -> Result<String, String> {
        let result = self.board.call_tool(name, arguments.clone()).map_err(|e| e.to_string())?;
        if result.is_error { Err(result.text()) } else { Ok(result.text()) }
    }

    fn confirm(&self, title: &str, text: &str, detail: &str) -> Option<bool> {
        self.agents.confirm(title, text, Some(detail), CONFIRM_WAIT).map(|d| d == Decision::Allow)
    }
}

/// Hook the assistant's changes to the UI, start the connectors and the
/// scheduler. Called once, from `setup`.
pub fn start(app: &AppHandle) {
    let state = app.state::<AppState>();
    let a = &state.assistant;
    {
        let app = app.clone();
        a.inbox.listen(Arc::new(move |items, _new| {
            let _ = app.emit("inbox", items);
            crate::update_tray(&app);
        }));
    }
    {
        let app = app.clone();
        a.on_chat(Arc::new(move |lines| {
            let _ = app.emit("chat", lines);
        }));
    }
    start_connectors(app);
    start_scheduler(app);
}

/// Connector events come from the connectors' own threads, sometimes with
/// their locks held: a thread of ours reads the fresh list and sends it.
fn start_connectors(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Some(dir) = assistant::default_dir() else {
        *state.services.connectors_error.lock().unwrap() = Some("no data folder for the connectors".into());
        return;
    };
    let (tx, rx) = mpsc::channel::<ConnectorEvent>();
    let tx = Mutex::new(tx);
    match Connectors::open(dir, move |e| drop(tx.lock().unwrap().send(e))) {
        Ok(c) => {
            let c = Arc::new(c);
            state.assistant.set_external(Some(c.clone()));
            *state.services.connectors.lock().unwrap() = Some(c);
        }
        Err(e) => {
            *state.services.connectors_error.lock().unwrap() = Some(e.to_string());
            return;
        }
    }
    let app = app.clone();
    let _ = thread::Builder::new().name("yoru-connector-events".into()).spawn(move || {
        while rx.recv().is_ok() {
            // A burst (a connector and its tools) is one refresh.
            while rx.recv_timeout(Duration::from_millis(100)).is_ok() {}
            let state = app.state::<AppState>();
            if let Ok(c) = state.services.connectors() {
                let _ = app.emit("connectors", c.list());
            }
        }
    });
}

fn start_scheduler(app: &AppHandle) {
    let state = app.state::<AppState>();
    let (assistant, watchers, stop) = (state.assistant.clone(), state.watchers.clone(), state.services.stop.clone());
    let app = app.clone();
    let handle = thread::Builder::new().name("yoru-scheduler".into()).spawn(move || {
        // Jobs run with nobody watching: nothing that acts is allowed.
        watchers::run_scheduler(&assistant, &watchers, &TaskHost, &stop, &|e| {
            let _ = app.emit("schedule", &e);
        });
    });
    *state.services.scheduler.lock().unwrap() = handle.ok();
}

fn host(state: &AppState) -> AppHost {
    AppHost { board: state.board(None), agents: state.agents.clone() }
}

async fn blocking<T: Send + 'static>(app: AppHandle, job: impl FnOnce(&AppState) -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || job(&app.state::<AppState>())).await.map_err(|e| e.to_string())?
}

// The assistant.

/// A model the brain picker offers.
#[derive(Clone, Serialize)]
pub struct BrainModel {
    id: &'static str,
    note: &'static str,
}

#[derive(Clone, Serialize)]
pub struct AssistantInfo {
    settings: Settings,
    /// Its brain has a key: it can think.
    ready: bool,
    /// The brain it thinks with now (the settings', or the one with a key).
    brain: Option<Brain>,
    /// Where the Anthropic key comes from.
    key: Option<KeySource>,
    /// Where the Groq key comes from.
    groq_key: Option<KeySource>,
    groq_models: Vec<BrainModel>,
    /// Where the OpenAI key comes from.
    openai_key: Option<KeySource>,
    openai_models: Vec<BrainModel>,
    /// Groq's free limit, while it is waited out: what the user reads.
    limited: Option<String>,
    /// Where its files are (memory.md, inbox.json…).
    dir: Option<String>,
    /// "Do not disturb" until then (Unix ms).
    quiet_until: Option<u64>,
    quiet_now: bool,
}

fn info_of(a: &Assistant) -> AssistantInfo {
    AssistantInfo {
        settings: a.settings(),
        ready: a.is_ready(),
        brain: a.settings().brain(),
        key: Provider::by_id("anthropic").and_then(|p| p.key_source()).map(|(_, s)| s),
        groq_key: Provider::by_id("groq").and_then(|p| p.key_source()).map(|(_, s)| s),
        groq_models: brain_models(Brain::Groq),
        openai_key: Provider::by_id("openai").and_then(|p| p.key_source()).map(|(_, s)| s),
        openai_models: brain_models(Brain::OpenAi),
        limited: a.limited().map(|e| assistant::compat::brain_error(&e)),
        dir: a.dir().map(|d| d.display().to_string()),
        quiet_until: a.quiet_until(),
        quiet_now: a.quiet_now(),
    }
}

fn brain_models(brain: Brain) -> Vec<BrainModel> {
    assistant::compat::models(brain).into_iter().map(|(id, note)| BrainModel { id, note }).collect()
}

/// The models Ollama has on this computer; an error when it isn't running.
#[tauri::command]
pub async fn ollama_models() -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(|| cloud::list_models(&cloud::OLLAMA).map_err(|e| assistant::compat::brain_error(&e))).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn assistant_info(state: State<AppState>) -> AssistantInfo {
    info_of(&state.assistant)
}

#[tauri::command]
pub fn set_assistant_settings(state: State<AppState>, settings: Settings) -> Result<AssistantInfo, String> {
    state.assistant.set_settings(settings).map_err(|e| e.to_string())?;
    Ok(info_of(&state.assistant))
}

/// Hold the loud notifications back for `minutes` (0 ends it).
#[tauri::command]
pub fn set_do_not_disturb(app: AppHandle, state: State<AppState>, minutes: u64) -> AssistantInfo {
    snooze(&state.assistant, Duration::from_secs(minutes * 60));
    crate::update_tray(&app);
    info_of(&state.assistant)
}

fn snooze(a: &Assistant, span: Duration) {
    let until = (!span.is_zero()).then(|| crate::now_ms() + span.as_millis() as u64);
    a.set_quiet_until(until);
}

/// The tray's "do not disturb for an hour", or its end.
pub fn toggle_dnd(app: &AppHandle) {
    let state = app.state::<AppState>();
    let span = if state.assistant.quiet_until().is_some() { Duration::ZERO } else { DND };
    snooze(&state.assistant, span);
    crate::update_tray(app);
    let _ = app.emit("assistant", info_of(&state.assistant));
}

/// Something typed to Yoru, answered in the conversation.
#[tauri::command]
pub async fn ask(app: AppHandle, text: String) -> Result<Reply, String> {
    blocking(app, move |state| {
        let language = state.settings.lock().unwrap().ui_language.clone();
        state.assistant.respond(&text, language.as_deref(), Channel::Text, &host(state))
    })
    .await
}

#[tauri::command]
pub fn chat(state: State<AppState>) -> Vec<ChatLine> {
    state.assistant.chat()
}

#[tauri::command]
pub fn forget_conversation(state: State<AppState>) {
    state.assistant.forget_conversation();
}

// The inbox.

#[tauri::command]
pub fn inbox(state: State<AppState>) -> Vec<Notification> {
    state.assistant.inbox.list()
}

#[tauri::command]
pub fn inbox_read(state: State<AppState>, id: String) -> bool {
    state.assistant.inbox.mark_read(&id)
}

#[tauri::command]
pub fn inbox_read_all(state: State<AppState>) -> bool {
    state.assistant.inbox.mark_all_read()
}

#[tauri::command]
pub fn inbox_dismiss(state: State<AppState>, id: String) -> bool {
    state.assistant.inbox.dismiss(&id)
}

#[tauri::command]
pub fn inbox_clear(state: State<AppState>) -> bool {
    state.assistant.inbox.clear()
}

/// Open what notification `id` is about, in the browser, and mark it read.
#[tauri::command]
pub fn open_notification(state: State<AppState>, id: String) -> Result<(), String> {
    let n = state.assistant.inbox.list().into_iter().find(|n| n.id == id).ok_or("that notification is gone")?;
    state.assistant.inbox.mark_read(&id);
    match n.url {
        Some(url) => open_link(&url),
        None => Ok(()),
    }
}

/// Open a web page the UI links to (a service's docs, a notification's page).
#[tauri::command]
pub fn open_web(url: String) -> Result<(), String> {
    open_link(&url)
}

/// A link that came from a service: only web addresses are opened, never a
/// file or an app's scheme.
pub fn open_link(url: &str) -> Result<(), String> {
    let lower = url.trim().to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return Err(format!("not a web address: {url}"));
    }
    crate::open_in_browser(url.trim())
}

// The memory.

#[tauri::command]
pub fn memory(state: State<AppState>) -> String {
    state.assistant.memory.text()
}

#[tauri::command]
pub fn set_memory(state: State<AppState>, text: String) -> Result<(), String> {
    state.assistant.memory.set_text(&text).map_err(|e| e.to_string())
}

// Watchers and routines.

#[tauri::command]
pub fn watchers(state: State<AppState>) -> Vec<WatchEntry> {
    state.watchers.list()
}

#[tauri::command]
pub fn put_watcher(state: State<AppState>, watcher: Watcher) -> Result<Vec<WatchEntry>, String> {
    state.watchers.put(watcher)?;
    Ok(state.watchers.list())
}

#[tauri::command]
pub fn remove_watcher(state: State<AppState>, id: String) -> Vec<WatchEntry> {
    state.watchers.remove(&id);
    state.watchers.list()
}

/// Run watcher `id` now; the `schedule` event says how it went, as for a
/// scheduled run.
#[tauri::command]
pub async fn run_watcher(app: AppHandle, id: String) -> Result<ScheduleEvent, String> {
    let emitter = app.clone();
    let event = blocking(app, move |state| {
        let entry = state.watchers.list().into_iter().find(|e| e.watcher.id == id).ok_or("no such watcher")?;
        Ok(watchers::run_and_record(&state.assistant, &state.watchers, &entry.watcher, &TaskHost))
    })
    .await?;
    let _ = emitter.emit("schedule", &event);
    Ok(event)
}

/// The morning recap, now.
#[tauri::command]
pub async fn recap_now(app: AppHandle) -> Result<Reply, String> {
    let emitter = app.clone();
    let result = blocking(app, |state| {
        let task = state.assistant.settings().routines.recap_task().to_string();
        state.assistant.run_task(&task, &TaskHost)
    })
    .await;
    let _ = emitter.emit("schedule", ScheduleEvent::Recap { error: result.as_ref().err().cloned() });
    result
}

// Connectors.

#[tauri::command]
pub fn connectors(state: State<AppState>) -> Result<Vec<ConnectorStatus>, String> {
    Ok(state.services.connectors()?.list())
}

#[tauri::command]
pub fn connector_presets() -> &'static [Preset] {
    catalog::presets()
}

/// Add a connector: a whole `config`, or a `preset`, with the OAuth client
/// the user made for it only when they chose to (Advanced).
#[tauri::command]
pub async fn add_connector(
    app: AppHandle,
    config: Option<ConnectorConfig>,
    preset: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
) -> Result<ConnectorStatus, String> {
    blocking(app, move |state| {
        let config = match (config, preset) {
            (Some(config), _) => config,
            (None, Some(id)) => from_preset(&id, client_id, client_secret)?,
            (None, None) => return Err("give a connector or a preset".into()),
        };
        state.services.connectors()?.add(config).map_err(|e| e.to_string())
    })
    .await
}

fn from_preset(id: &str, client_id: Option<String>, client_secret: Option<String>) -> Result<ConnectorConfig, String> {
    let preset = catalog::preset(id).ok_or_else(|| format!("no preset {id:?}"))?;
    let clean = |s: Option<String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let (client_id, client_secret) = (clean(client_id), clean(client_secret));
    if client_id.is_none() && !preset.one_click() {
        return Err(format!("this build of Yoru Hub has no {} client: give your own in Advanced", preset.name));
    }
    if client_id.is_some() && preset.needs_client_secret && client_secret.is_none() {
        return Err(format!("{} needs your OAuth client's secret too", preset.name));
    }
    Ok(preset.config(client_id, client_secret))
}

#[tauri::command]
pub async fn remove_connector(app: AppHandle, id: String) -> Result<(), String> {
    blocking(app, move |state| state.services.connectors()?.remove(&id).map_err(|e| e.to_string())).await
}

#[tauri::command]
pub async fn set_connector_enabled(app: AppHandle, id: String, enabled: bool) -> Result<(), String> {
    blocking(app, move |state| state.services.connectors()?.set_enabled(&id, enabled).map_err(|e| e.to_string())).await
}

/// Whether `tool` of connector `id` is offered to Yoru, and whether it asks first.
#[tauri::command]
pub async fn set_tool_policy(app: AppHandle, id: String, tool: String, enabled: bool, acts: bool) -> Result<(), String> {
    blocking(app, move |state| state.services.connectors()?.set_tool_policy(&id, &tool, enabled, acts).map_err(|e| e.to_string())).await
}

/// Sign in to connector `id` in the browser; returns once it's done (or
/// after five minutes, or `cancel_login`).
#[tauri::command]
pub async fn login_connector(app: AppHandle, id: String) -> Result<(), String> {
    blocking(app, move |state| {
        let c = state.services.connectors()?;
        // The sign-in page comes from the service's metadata.
        c.login(&id, &|url| drop(open_link(url))).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub fn cancel_login(state: State<AppState>, id: String) -> Result<(), String> {
    state.services.connectors()?.cancel_login(&id);
    Ok(())
}

#[tauri::command]
pub async fn logout_connector(app: AppHandle, id: String) -> Result<(), String> {
    blocking(app, move |state| state.services.connectors()?.logout(&id).map_err(|e| e.to_string())).await
}

/// A local server's named secret, in the keychain; `None` or empty removes it.
#[tauri::command]
pub async fn set_connector_secret(app: AppHandle, name: String, value: Option<String>) -> Result<(), String> {
    blocking(app, move |state| {
        let c = state.services.connectors()?;
        match value.filter(|v| !v.trim().is_empty()) {
            Some(v) => c.set_secret(&name, v.trim()),
            None => c.delete_secret(&name),
        }
        .map_err(|e| e.to_string())
    })
    .await
}
