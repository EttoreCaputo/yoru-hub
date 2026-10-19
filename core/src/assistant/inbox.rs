//! The notifications Yoru gives you: what a watcher found worth telling, the
//! morning recap, a reminder the assistant set, Claude Code finishing. Kept
//! in `inbox.json` in the data folder (the newest 200), shown in the app's
//! Inbox and on the board's inbox (firmware 0.10.0, `show_notification`).
//!
//! The inbox only keeps them and says when they change; the bridge delivers
//! each new one to the board ([`crate::bridge`]), by its [`Urgency`].

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// The newest kept.
const KEEP: usize = 200;

/// How loudly a notification arrives.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Urgency {
    /// Only in the inbox.
    Low,
    /// A banner on the board, with a soft sound.
    #[default]
    Normal,
    /// Yoru comes to the glass and knocks.
    High,
    /// Also said out loud, and through quiet hours.
    Urgent,
}

impl Urgency {
    pub fn name(self) -> &'static str {
        match self {
            Urgency::Low => "low",
            Urgency::Normal => "normal",
            Urgency::High => "high",
            Urgency::Urgent => "urgent",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "low" => Some(Urgency::Low),
            "normal" => Some(Urgency::Normal),
            "high" => Some(Urgency::High),
            "urgent" => Some(Urgency::Urgent),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    /// Where it comes from: "Gmail", "Calendar", "Yoru", "Claude Code".
    pub source: String,
    pub title: String,
    #[serde(default)]
    pub text: String,
    pub urgency: Urgency,
    /// A link that opens it (a mail, an event, an issue).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Unix ms.
    pub at: u64,
    #[serde(default)]
    pub read: bool,
    /// The watcher or routine that made it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
}

/// What changed, for whoever mirrors the inbox (the board).
#[derive(Debug, Clone, PartialEq)]
pub enum InboxChange {
    New(Notification),
    Removed(String),
    Cleared,
}

/// Told about every change, with the whole inbox and the one that is new, if any.
pub type InboxListener = Arc<dyn Fn(&[Notification], Option<&Notification>) + Send + Sync>;

pub struct Inbox {
    file: Option<PathBuf>,
    items: Mutex<Vec<Notification>>,
    next: Mutex<u64>,
    listeners: Mutex<Vec<InboxListener>>,
    /// The bridge, which delivers each new one to the board; one at a time.
    delivery: Mutex<Option<Arc<dyn Fn(InboxChange) + Send + Sync>>>,
}

impl Inbox {
    pub fn open(file: Option<PathBuf>) -> Self {
        let items: Vec<Notification> = file.as_ref().and_then(|f| fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { file, items: Mutex::new(items), next: Mutex::new(now_ms()), listeners: Mutex::default(), delivery: Mutex::default() }
    }

    pub fn listen(&self, listener: InboxListener) {
        self.listeners.lock().unwrap().push(listener);
    }

    /// Who delivers each new notification (the bridge, while it runs).
    pub fn set_delivery(&self, delivery: Option<Arc<dyn Fn(InboxChange) + Send + Sync>>) {
        *self.delivery.lock().unwrap() = delivery;
    }

    /// Newest first.
    pub fn list(&self) -> Vec<Notification> {
        self.items.lock().unwrap().clone()
    }

    pub fn unread(&self) -> usize {
        self.items.lock().unwrap().iter().filter(|n| !n.read).count()
    }

    /// Add one; its id and time are set here. Returns the id.
    pub fn post(&self, source: &str, title: &str, text: &str, urgency: Urgency, url: Option<String>, by: Option<String>) -> String {
        let id = {
            let mut next = self.next.lock().unwrap();
            *next = (*next + 1).max(now_ms());
            format!("n{:x}", *next)
        };
        let n = Notification {
            id: id.clone(),
            source: clip(source, 24),
            title: clip(title, 120),
            text: clip(text, 2000),
            urgency,
            url,
            at: now_ms(),
            read: false,
            by,
        };
        {
            let mut items = self.items.lock().unwrap();
            items.insert(0, n.clone());
            items.truncate(KEEP);
        }
        self.changed(Some(&n));
        id
    }

    /// Say which watcher or routine made `ids`.
    pub fn tag(&self, ids: &[String], by: &str) {
        self.update(|items| {
            let mut any = false;
            for n in items.iter_mut().filter(|n| ids.contains(&n.id) && n.by.is_none()) {
                n.by = Some(by.to_string());
                any = true;
            }
            any
        });
    }

    pub fn mark_read(&self, id: &str) -> bool {
        self.update(|items| items.iter_mut().find(|n| n.id == id && !n.read).map(|n| n.read = true).is_some())
    }

    pub fn mark_all_read(&self) -> bool {
        self.update(|items| {
            let any = items.iter().any(|n| !n.read);
            items.iter_mut().for_each(|n| n.read = true);
            any
        })
    }

    pub fn dismiss(&self, id: &str) -> bool {
        let removed = self.update(|items| {
            let before = items.len();
            items.retain(|n| n.id != id);
            items.len() != before
        });
        if removed {
            self.deliver(InboxChange::Removed(id.to_string()));
        }
        removed
    }

    pub fn clear(&self) -> bool {
        let cleared = self.update(|items| {
            let any = !items.is_empty();
            items.clear();
            any
        });
        if cleared {
            self.deliver(InboxChange::Cleared);
        }
        cleared
    }

    fn deliver(&self, change: InboxChange) {
        let delivery = self.delivery.lock().unwrap().clone();
        if let Some(deliver) = delivery {
            deliver(change);
        }
    }

    fn update(&self, change: impl FnOnce(&mut Vec<Notification>) -> bool) -> bool {
        let changed = change(&mut self.items.lock().unwrap());
        if changed {
            self.changed(None);
        }
        changed
    }

    fn changed(&self, new: Option<&Notification>) {
        let items = self.list();
        if let Some(file) = &self.file {
            if let Some(dir) = file.parent() {
                let _ = fs::create_dir_all(dir);
            }
            if let Ok(json) = serde_json::to_vec_pretty(&items) {
                let tmp = file.with_extension("json.tmp");
                if fs::write(&tmp, json).is_ok() {
                    let _ = fs::rename(tmp, file);
                }
            }
        }
        let listeners = self.listeners.lock().unwrap().clone();
        for listener in listeners {
            listener(&items, new);
        }
        if let Some(new) = new {
            self.deliver(InboxChange::New(new.clone()));
        }
    }
}

fn clip(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn posts_reads_and_dismisses() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("inbox.json");
        let inbox = Inbox::open(Some(file.clone()));
        let news = Arc::new(AtomicUsize::new(0));
        let seen = news.clone();
        inbox.listen(Arc::new(move |_, new| {
            if new.is_some() {
                seen.fetch_add(1, Ordering::Relaxed);
            }
        }));
        let a = inbox.post("Gmail", "Mail from Marco", "About Friday", Urgency::Normal, None, Some("gmail-unread".into()));
        let b = inbox.post("Calendar", "Standup in 10 minutes", "", Urgency::High, None, None);
        assert_ne!(a, b);
        assert_eq!(inbox.list()[0].id, b);
        assert_eq!(inbox.unread(), 2);
        assert!(inbox.mark_read(&a));
        assert!(!inbox.mark_read(&a));
        assert_eq!(news.load(Ordering::Relaxed), 2);
        let reopened = Inbox::open(Some(file));
        assert_eq!(reopened.unread(), 1);
        assert!(reopened.dismiss(&b));
        assert_eq!(reopened.list().len(), 1);
    }

    #[test]
    fn urgency_names() {
        assert_eq!(Urgency::parse("urgent"), Some(Urgency::Urgent));
        assert_eq!(serde_json::to_value(Urgency::High).unwrap(), "high");
        assert!(Urgency::Low < Urgency::Urgent);
        assert_eq!(clip("abcdef", 4), "abc…");
    }
}
