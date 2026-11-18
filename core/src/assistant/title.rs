//! The conversation's title, shown above the chat in the app.
//!
//! The brain names the conversation after Yoru's first answer, and names it
//! again as it goes on (when the log has doubled since the last time), so the
//! title follows what the conversation turned into. A title the user typed
//! stays: Yoru doesn't rename it until the conversation is forgotten.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::ChatLine;

/// Lines of the log the brain reads to name it: the newest ones.
const READ_LINES: usize = 24;
/// Characters of each line it reads.
const LINE_CHARS: usize = 400;
/// Characters of a title.
const MAX_CHARS: usize = 60;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatTitle {
    /// Empty: not named yet.
    pub text: String,
    /// The user typed it: Yoru leaves it alone.
    #[serde(default)]
    pub by_user: bool,
    /// The log's length when it was named.
    #[serde(default)]
    pub lines: usize,
}

#[derive(Debug, Default)]
pub struct Titles {
    file: Option<PathBuf>,
    title: Mutex<ChatTitle>,
    /// A naming is on its way: one at a time.
    busy: AtomicBool,
}

impl Titles {
    pub fn open(file: Option<PathBuf>) -> Self {
        let title = file.as_ref().and_then(|f| fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { file, title: Mutex::new(title), busy: AtomicBool::new(false) }
    }

    pub fn get(&self) -> ChatTitle {
        self.title.lock().unwrap().clone()
    }

    /// The user's title; an empty one gives the naming back to Yoru.
    pub fn set_by_user(&self, text: &str, lines: usize) -> ChatTitle {
        let text = tidy(text);
        let by_user = !text.is_empty();
        self.put(ChatTitle { text, by_user, lines: if by_user { lines } else { 0 } })
    }

    pub fn clear(&self) {
        self.put(ChatTitle::default());
    }

    /// Another conversation's title, opened.
    pub fn replace(&self, title: ChatTitle) {
        self.put(title);
    }

    /// Whether the conversation, `log` long and last said by `last_from`,
    /// is due a (new) name; if so, takes the turn to name it.
    pub fn due(&self, log: usize, last_from: Option<&str>) -> bool {
        let t = self.get();
        let grown = if t.text.is_empty() { log >= 2 } else { log >= t.lines.max(1) * 2 };
        grown && !t.by_user && last_from == Some("yoru") && !self.busy.swap(true, Ordering::SeqCst)
    }

    /// The naming that [`Self::due`] started is over: `text`, if it found one.
    /// A title the user typed meanwhile, or a forgotten conversation, wins.
    pub fn named(&self, text: Option<&str>, lines: usize, log_now: usize) -> Option<ChatTitle> {
        self.busy.store(false, Ordering::SeqCst);
        let text = tidy(text?);
        let now = self.get();
        if text.is_empty() || now.by_user || log_now < lines {
            return None;
        }
        Some(self.put(ChatTitle { text, by_user: false, lines }))
    }

    fn put(&self, title: ChatTitle) -> ChatTitle {
        *self.title.lock().unwrap() = title.clone();
        if let Some(file) = &self.file {
            if let Some(dir) = file.parent() {
                let _ = fs::create_dir_all(dir);
            }
            if let Ok(json) = serde_json::to_vec(&title) {
                let _ = fs::write(file, json);
            }
        }
        title
    }
}

/// What the brain is asked, in `language` (a name: "Italian").
pub fn prompt(log: &[ChatLine], language: &str, before: &str) -> String {
    let lines: Vec<String> = log
        .iter()
        .rev()
        .take(READ_LINES)
        .rev()
        .map(|l| format!("{}: {}", if l.from == "you" { "User" } else { "Yoru" }, l.text.chars().take(LINE_CHARS).collect::<String>()))
        .collect();
    let before = if before.is_empty() { String::new() } else { format!("Its title so far: \"{before}\". Keep it if it still fits; change it if the conversation moved on.\n") };
    format!(
        "Give this conversation between the user and Yoru, their desk assistant, a short title: 2 to 5 words, in {language}, \
         naming what it is about (not \"Conversation\" or \"Chat\"), no quotes, no final period.\n{before}\n{}",
        lines.join("\n")
    )
}

pub fn schema() -> Value {
    json!({"type": "object", "properties": {"title": {"type": "string"}}, "required": ["title"], "additionalProperties": false})
}

/// The title in the brain's answer.
pub fn parse(answer: &str) -> Option<String> {
    serde_json::from_str::<Value>(answer).ok()?["title"].as_str().map(str::to_string)
}

/// One line, no quotes or final period, not too long.
fn tidy(text: &str) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = one.trim_matches(|c: char| matches!(c, '"' | '\'' | '“' | '”' | '«' | '»')).trim_end_matches('.').trim();
    trimmed.chars().take(MAX_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_after_the_first_answer_then_when_doubled() {
        let t = Titles::open(None);
        assert!(!t.due(1, Some("you")));
        assert!(!t.due(2, Some("you")));
        assert!(t.due(2, Some("yoru")));
        // One naming at a time.
        assert!(!t.due(2, Some("yoru")));
        t.named(Some("\"Morning news.\""), 2, 2);
        assert_eq!(t.get().text, "Morning news");
        assert!(!t.due(3, Some("yoru")));
        assert!(t.due(4, Some("yoru")));
    }

    #[test]
    fn the_users_title_stays() {
        let t = Titles::open(None);
        t.set_by_user("  My plan  ", 4);
        assert!(!t.due(40, Some("yoru")));
        assert_eq!(t.get().text, "My plan");
        // Emptied, it's Yoru's again.
        t.set_by_user("", 4);
        assert!(t.due(4, Some("yoru")));
    }

    #[test]
    fn a_late_name_loses_to_a_forget() {
        let t = Titles::open(None);
        assert!(t.due(6, Some("yoru")));
        t.clear();
        assert!(t.named(Some("Old"), 6, 0).is_none());
        assert_eq!(t.get().text, "");
    }
}
