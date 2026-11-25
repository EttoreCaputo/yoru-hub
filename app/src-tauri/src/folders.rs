//! Folders on Home (`yoru_core::assistant::folders`): the list, and
//! creating, renaming, deleting them and putting conversations and meetings
//! in them. Each command answers with the folders as they are now.

use tauri::State;
use yoru_core::assistant::folders::{FolderView, Kind};

use crate::AppState;

#[tauri::command]
pub fn folders(state: State<AppState>) -> FolderView {
    state.assistant.folders.view()
}

#[tauri::command]
pub fn create_folder(state: State<AppState>, name: String) -> Result<FolderView, String> {
    state.assistant.folders.create(&name)
}

#[tauri::command]
pub fn rename_folder(state: State<AppState>, id: String, name: String) -> Result<FolderView, String> {
    state.assistant.folders.rename(&id, &name)
}

/// What was in it goes back to no folder.
#[tauri::command]
pub fn delete_folder(state: State<AppState>, id: String) -> Result<FolderView, String> {
    state.assistant.folders.delete(&id)
}

/// Put a conversation or a meeting in `folder`, or in none.
#[tauri::command]
pub fn move_to_folder(state: State<AppState>, kind: Kind, id: String, folder: Option<String>) -> Result<FolderView, String> {
    state.assistant.folders.put(kind, &id, folder.as_deref())
}
