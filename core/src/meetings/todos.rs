//! Every meeting's to-dos in one list: the things to do its summary found,
//! each with the meeting it came from and whether the user ticked it off.
//! What is done is kept beside the summary (`done.json`, the done items'
//! texts), so a summary made again starts with its new items open.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::Path;

use serde::Serialize;

use super::{ActionItem, folder, list, summary};

const DONE: &str = "done.json";

/// A to-do and where it came from.
#[derive(Debug, Clone, Serialize)]
pub struct Todo {
    /// The meeting's id, to open it.
    pub meeting: String,
    /// The meeting's title: the user's, else the summary's.
    pub title: String,
    /// When the meeting started, Unix milliseconds.
    pub started_at: i64,
    pub item: ActionItem,
    pub done: bool,
}

/// The to-dos of every summarized meeting, newest meeting first, each
/// meeting's in the summary's order.
pub fn todos() -> Vec<Todo> {
    let mut out = Vec::new();
    for meeting in list().into_iter().filter(|m| m.summarized_at.is_some()) {
        let Ok(Some(summary)) = summary(&meeting.id) else { continue };
        let done = folder(&meeting.id).map(|f| done_in(&f)).unwrap_or_default();
        let title = meeting.title.clone().filter(|t| !t.trim().is_empty()).unwrap_or(summary.title.clone());
        out.extend(summary.actions.into_iter().map(|item| Todo {
            meeting: meeting.id.clone(),
            title: title.clone(),
            started_at: meeting.started_at,
            done: done.contains(&item.text),
            item,
        }));
    }
    out
}

/// Tick the to-do `text` of meeting `id` off, or open it again.
pub fn set_done(id: &str, text: &str, done: bool) -> io::Result<()> {
    mark_in(&folder(id)?, text, done)
}

fn done_in(folder: &Path) -> BTreeSet<String> {
    fs::read(folder.join(DONE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn mark_in(folder: &Path, text: &str, done: bool) -> io::Result<()> {
    let mut set = done_in(folder);
    if done {
        set.insert(text.to_string());
    } else {
        set.remove(text);
    }
    // Written aside and renamed, like the meeting itself.
    let part = folder.join("done.json.part");
    fs::write(&part, serde_json::to_vec(&set)?)?;
    fs::rename(part, folder.join(DONE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_todo_is_ticked_off_and_opened_again() {
        let dir = std::env::temp_dir().join(format!("yoru-todos-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        assert!(done_in(&dir).is_empty());
        mark_in(&dir, "Send the slides", true).unwrap();
        mark_in(&dir, "Call Marco", true).unwrap();
        mark_in(&dir, "Call Marco", false).unwrap();
        assert_eq!(done_in(&dir).into_iter().collect::<Vec<_>>(), ["Send the slides"]);
        fs::remove_dir_all(dir).unwrap();
    }
}
