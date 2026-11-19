//! The Pocket voice recorder (docs/pocket.md): its thread, and the commands
//! of the Meetings page's Pocket panel. Each recording it brings becomes a
//! meeting, transcribed like an imported file.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use yoru_core::meetings::Meeting;
use yoru_core::pocket::{self, Command, Event, Pocket, Settings, Status};

/// What the `"pocket"` event carries.
#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Payload<'a> {
    Status { status: &'a Status },
    Imported { meeting: &'a Meeting },
}

/// At launch. Not from a bare binary (`cargo run`): macOS ends a process
/// that uses Bluetooth without the bundle's usage description.
pub fn start(app: &AppHandle) {
    let bundled = std::env::current_exe().is_ok_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"));
    if !bundled {
        yoru_core::hostlog::line("pocket", "not started: Bluetooth needs the app bundle");
        return;
    }
    let events = app.clone();
    let started = Pocket::start(move |event| match event {
        Event::Status(status) => {
            let _ = events.emit("pocket", Payload::Status { status: &status });
        }
        Event::Imported(meeting) => {
            let _ = events.emit("pocket", Payload::Imported { meeting: &meeting });
            crate::meetings::after_import(&events, meeting.id.clone());
        }
    });
    match started {
        Ok(pocket) => {
            app.manage(pocket);
        }
        Err(e) => yoru_core::hostlog::line("pocket", &format!("not started: {e}")),
    }
}

fn send(app: &AppHandle, command: Command) -> Result<(), String> {
    app.try_state::<Pocket>().ok_or("The Pocket isn't available in this build")?.send(command);
    Ok(())
}

/// `None` where it can't run (a bare binary).
#[tauri::command]
pub fn pocket_status(app: AppHandle) -> Option<Status> {
    app.try_state::<Pocket>().map(|p| p.status())
}

#[tauri::command]
pub fn pocket_sync(app: AppHandle) -> Result<(), String> {
    send(&app, Command::Sync)
}

#[tauri::command]
pub fn pocket_download(app: AppHandle, names: Vec<String>) -> Result<(), String> {
    send(&app, Command::Download(names))
}

#[tauri::command]
pub fn pocket_delete(app: AppHandle, name: String) -> Result<(), String> {
    send(&app, Command::Delete(name))
}

/// Wait for a Pocket fresh from a reset (`false`: stop waiting).
#[tauri::command]
pub fn pocket_pair(app: AppHandle, on: bool) -> Result<(), String> {
    send(&app, if on { Command::Pair } else { Command::CancelPair })
}

#[tauri::command]
pub fn pocket_forget(app: AppHandle) -> Result<(), String> {
    send(&app, Command::Forget)
}

#[tauri::command]
pub fn set_pocket_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    send(&app, Command::Settings(settings))
}

/// The settings as saved, for a build where the Pocket doesn't run.
#[tauri::command]
pub fn pocket_settings() -> Settings {
    pocket::settings()
}
