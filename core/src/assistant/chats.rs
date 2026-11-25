//! The conversations put aside: every conversation but the one Yoru is in.
//!
//! The one in use is the log and title the assistant keeps
//! (`conversation.json`, `conversation_title.json`); starting another, or
//! opening an old one, puts it here, in `conversations/<id>.json`, and the
//! one opened comes out. `conversations/current` names the one in use.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use super::{ChatLine, ChatTitle};

/// Conversations kept; the oldest go first.
const KEEP: usize = 500;
/// Characters of the line a conversation shows in the list.
const PREVIEW_CHARS: usize = 140;

/// A conversation put aside, as saved.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedChat {
    pub id: String,
    #[serde(default)]
    pub title: ChatTitle,
    #[serde(default)]
    pub lines: Vec<ChatLine>,
}

/// A conversation as the app lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatSummary {
    pub id: String,
    /// Empty: not named; the app shows `preview` instead.
    pub title: String,
    pub by_user: bool,
    /// Unix ms of its first and last line.
    pub started: u64,
    pub updated: u64,
    pub lines: usize,
    /// The user's first words.
    pub preview: String,
}

/// The list the app gets: the conversation in use and every one with a line.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Conversations {
    pub current: String,
    pub list: Vec<ChatSummary>,
}

impl SavedChat {
    pub fn summary(&self) -> ChatSummary {
        let first = self.lines.iter().find(|l| l.from == "you").or(self.lines.first());
        ChatSummary {
            id: self.id.clone(),
            title: self.title.text.clone(),
            by_user: self.title.by_user,
            started: self.lines.first().map_or(0, |l| l.at),
            updated: self.lines.last().map_or(0, |l| l.at),
            lines: self.lines.len(),
            preview: first.map(|l| l.text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(PREVIEW_CHARS).collect()).unwrap_or_default(),
        }
    }
}

#[derive(Debug, Default)]
pub struct Archive {
    dir: Option<PathBuf>,
    chats: Mutex<Vec<SavedChat>>,
    current: Mutex<String>,
}

/// A new conversation's id: the time it began, never the same twice.
pub fn new_id() -> String {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = super::inbox::now_ms();
    let prev = LAST.fetch_max(now, Ordering::SeqCst);
    let id = if prev >= now { LAST.fetch_add(1, Ordering::SeqCst) + 1 } else { now };
    format!("c{id}")
}

impl Archive {
    /// The archive in `dir` (`None`: in memory only).
    pub fn open(dir: Option<PathBuf>) -> Self {
        let mut chats = Vec::new();
        let mut current = String::new();
        if let Some(dir) = &dir {
            for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "json")
                    && let Some(chat) = fs::read(&path).ok().and_then(|b| serde_json::from_slice::<SavedChat>(&b).ok())
                    && !chat.id.is_empty()
                {
                    chats.push(chat);
                }
            }
            current = fs::read_to_string(dir.join("current")).map(|s| s.trim().to_string()).unwrap_or_default();
        }
        let archive = Self { dir, chats: Mutex::new(chats), current: Mutex::new(String::new()) };
        archive.set_current(if current.is_empty() { new_id() } else { current });
        archive
    }

    pub fn current(&self) -> String {
        self.current.lock().unwrap().clone()
    }

    pub fn set_current(&self, id: String) {
        if let Some(dir) = &self.dir {
            let _ = fs::create_dir_all(dir);
            let _ = fs::write(dir.join("current"), &id);
        }
        *self.current.lock().unwrap() = id;
    }

    pub fn summaries(&self) -> Vec<ChatSummary> {
        self.chats.lock().unwrap().iter().map(SavedChat::summary).collect()
    }

    /// Every conversation put aside, as saved.
    pub fn all(&self) -> Vec<SavedChat> {
        self.chats.lock().unwrap().clone()
    }

    pub fn get(&self, id: &str) -> Option<SavedChat> {
        self.chats.lock().unwrap().iter().find(|c| c.id == id).cloned()
    }

    /// Put `chat` aside (or save it again); an empty one isn't kept.
    pub fn put(&self, chat: SavedChat) {
        if chat.lines.is_empty() {
            return;
        }
        let mut chats = self.chats.lock().unwrap();
        chats.retain(|c| c.id != chat.id);
        self.write(&chat);
        chats.push(chat);
        if chats.len() > KEEP {
            chats.sort_by_key(|c| c.lines.last().map_or(0, |l| l.at));
            let excess = chats.len() - KEEP;
            let gone: Vec<SavedChat> = chats.drain(..excess).collect();
            for chat in gone {
                self.unlink(&chat.id);
            }
        }
    }

    /// Takes `id` out of the archive (to open it, or for good).
    pub fn take(&self, id: &str) -> Option<SavedChat> {
        let mut chats = self.chats.lock().unwrap();
        let at = chats.iter().position(|c| c.id == id)?;
        self.unlink(id);
        Some(chats.remove(at))
    }

    /// A new title for `id`, the user's (empty: not named).
    pub fn rename(&self, id: &str, title: ChatTitle) -> bool {
        let mut chats = self.chats.lock().unwrap();
        let Some(chat) = chats.iter_mut().find(|c| c.id == id) else { return false };
        chat.title = title;
        self.write(chat);
        true
    }

    fn file(&self, id: &str) -> Option<PathBuf> {
        // Ids are made here, but a file name never comes from elsewhere unchecked.
        let safe = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        self.dir.as_ref().filter(|_| safe).map(|d| d.join(format!("{id}.json")))
    }

    fn write(&self, chat: &SavedChat) {
        if let (Some(file), Ok(json)) = (self.file(&chat.id), serde_json::to_vec(chat)) {
            if let Some(dir) = file.parent() {
                let _ = fs::create_dir_all(dir);
            }
            let _ = fs::write(file, json);
        }
    }

    fn unlink(&self, id: &str) {
        if let Some(file) = self.file(id) {
            let _ = fs::remove_file(file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::Channel;

    fn line(from: &str, text: &str, at: u64) -> ChatLine {
        ChatLine { from: from.into(), text: text.into(), channel: Channel::Text, actions: Vec::new(), at }
    }

    #[test]
    fn put_aside_and_taken_back() {
        let dir = std::env::temp_dir().join(format!("yoru-chats-{}", super::super::inbox::now_ms()));
        let a = Archive::open(Some(dir.clone()));
        let first = a.current();
        a.put(SavedChat { id: "c1".into(), title: ChatTitle::default(), lines: vec![line("you", "Hi   there", 5), line("yoru", "Hello", 6)] });
        a.put(SavedChat { id: "c2".into(), ..Default::default() });
        let s = a.summaries();
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].preview.as_str(), s[0].started, s[0].updated), ("Hi there", 5, 6));
        // Read again from the folder.
        let b = Archive::open(Some(dir.clone()));
        assert_eq!(b.current(), first);
        assert!(b.rename("c1", ChatTitle { text: "Greetings".into(), by_user: true, lines: 2 }));
        assert_eq!(Archive::open(Some(dir.clone())).get("c1").unwrap().title.text, "Greetings");
        assert!(b.take("c1").is_some());
        assert!(Archive::open(Some(dir.clone())).summaries().is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_name_from_elsewhere_is_no_path() {
        let a = Archive::open(Some(std::env::temp_dir()));
        assert!(a.file("../x").is_none());
        assert!(a.file("c123").is_some());
    }
}
