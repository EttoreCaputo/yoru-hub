//! Watchers: what Yoru keeps an eye on for you, every so often, and the
//! scheduler that runs them and the morning recap.
//!
//! A watcher is one of two kinds:
//! - **tool**: a connector's read tool with fixed arguments ("unread mail
//!   in the inbox", "events in the next two hours"). It runs without the
//!   model; only when its output changed does the model look at it
//!   ([`triage`]), say which items are new and which are worth telling,
//!   and the new ones become notifications. Items already seen (by a key
//!   the model gives each one) are never told twice. The first run only
//!   learns what is there.
//! - **task**: words for the assistant ("look for news about the ESP32-S3
//!   on the web"), run as a job of its own ([`Assistant::run_task`]), which
//!   notifies when it finds something.
//!
//! They are kept in `watchers.json` in the data folder, with what each one
//! has seen.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Assistant, Brain, Host, Urgency, inbox::now_ms};

/// Keys remembered per watcher.
const SEEN_KEEP: usize = 500;
/// The most of a tool's output the model reads.
const OUTPUT_MAX: usize = 60_000;
/// On Groq (its free plan allows about 8,000 tokens a minute, prompt
/// included) and Ollama (a model on this computer reads a short prompt best).
const OUTPUT_MAX_TIGHT: usize = 12_000;
/// The scheduler looks at the clock this often.
const TICK: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum WatchKind {
    /// A connector's tool (its qualified name) with fixed arguments.
    Tool { tool: String, #[serde(default)] arguments: Value },
    /// A job for the assistant, in words.
    Task { task: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Watcher {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub kind: WatchKind,
    /// Minutes between runs.
    pub every_min: u32,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// What matters in what it finds, in the user's words ("only mail from
    /// people, not newsletters"); the memory adds the rest.
    #[serde(default)]
    pub instructions: String,
    /// The most a notification from it may be.
    #[serde(default = "high")]
    pub max_urgency: Urgency,
}

fn yes() -> bool {
    true
}

fn high() -> Urgency {
    Urgency::High
}

/// What a watcher has done, kept with it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WatchState {
    /// Unix ms.
    #[serde(default)]
    pub last_run: u64,
    #[serde(default)]
    pub last_error: Option<String>,
    /// Notifications it gave, in all.
    #[serde(default)]
    pub told: u32,
    #[serde(default)]
    seen: Vec<String>,
    #[serde(default)]
    digest: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatchEntry {
    pub watcher: Watcher,
    #[serde(default)]
    pub state: WatchState,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    watchers: Vec<WatchEntry>,
    /// The day the recap was last given.
    #[serde(default)]
    recap_day: Option<NaiveDate>,
}

pub struct Watchers {
    file: Option<PathBuf>,
    saved: Mutex<Saved>,
}

impl Watchers {
    pub fn open(file: Option<PathBuf>) -> Self {
        let saved = file.as_ref().and_then(|f| fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { file, saved: Mutex::new(saved) }
    }

    pub fn list(&self) -> Vec<WatchEntry> {
        self.saved.lock().unwrap().watchers.clone()
    }

    /// Add `watcher`, or change the one with its id (keeping what it saw,
    /// unless what it watches changed).
    pub fn put(&self, mut watcher: Watcher) -> Result<(), String> {
        watcher.name = watcher.name.trim().to_string();
        if watcher.name.is_empty() {
            return Err("a watcher needs a name".into());
        }
        watcher.every_min = watcher.every_min.max(5);
        if watcher.id.trim().is_empty() {
            watcher.id = slug(&watcher.name);
        }
        let mut saved = self.saved.lock().unwrap();
        match saved.watchers.iter_mut().find(|e| e.watcher.id == watcher.id) {
            Some(entry) => {
                if entry.watcher.kind != watcher.kind {
                    entry.state = WatchState::default();
                }
                entry.watcher = watcher;
            }
            None => saved.watchers.push(WatchEntry { watcher, state: WatchState::default() }),
        }
        self.save(&saved);
        Ok(())
    }

    pub fn remove(&self, id: &str) -> bool {
        let mut saved = self.saved.lock().unwrap();
        let before = saved.watchers.len();
        saved.watchers.retain(|e| e.watcher.id != id);
        let removed = saved.watchers.len() != before;
        self.save(&saved);
        removed
    }

    /// The watchers due at `now` (Unix ms).
    fn due(&self, now: u64) -> Vec<Watcher> {
        let saved = self.saved.lock().unwrap();
        saved
            .watchers
            .iter()
            .filter(|e| e.watcher.enabled && now.saturating_sub(e.state.last_run) >= u64::from(e.watcher.every_min) * 60_000)
            .map(|e| e.watcher.clone())
            .collect()
    }

    fn state(&self, id: &str) -> Option<WatchState> {
        self.saved.lock().unwrap().watchers.iter().find(|e| e.watcher.id == id).map(|e| e.state.clone())
    }

    fn update(&self, id: &str, change: impl FnOnce(&mut WatchState)) {
        let mut saved = self.saved.lock().unwrap();
        if let Some(entry) = saved.watchers.iter_mut().find(|e| e.watcher.id == id) {
            change(&mut entry.state);
        }
        self.save(&saved);
    }

    fn recap_day(&self) -> Option<NaiveDate> {
        self.saved.lock().unwrap().recap_day
    }

    fn set_recap_day(&self, day: NaiveDate) {
        let mut saved = self.saved.lock().unwrap();
        saved.recap_day = Some(day);
        self.save(&saved);
    }

    fn save(&self, saved: &Saved) {
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(saved) {
            let tmp = file.with_extension("json.tmp");
            if fs::write(&tmp, json).is_ok() {
                let _ = fs::rename(tmp, file);
            }
        }
    }
}

/// What the scheduler did, for the app's log.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScheduleEvent {
    Watched { id: String, told: u32, error: Option<String> },
    Recap { error: Option<String> },
}

/// Run the watchers and the recap until `stop`, on the current thread.
/// `host` is what their jobs may use: the board, and a card for anything
/// that acts (a job shouldn't, and is told so).
pub fn run_scheduler(assistant: &Assistant, watchers: &Watchers, host: &dyn Host, stop: &AtomicBool, on_event: &(dyn Fn(ScheduleEvent) + Send + Sync)) {
    while !stop.load(Ordering::Relaxed) {
        if assistant.is_ready() {
            let routines = assistant.settings().routines;
            let now = Local::now();
            if routines.recap_due(now, watchers.recap_day()) {
                watchers.set_recap_day(now.date_naive());
                let error = assistant.run_task(routines.recap_task(), host).err();
                on_event(ScheduleEvent::Recap { error });
            }
            for watcher in watchers.due(now_ms()) {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                on_event(run_and_record(assistant, watchers, &watcher, host));
            }
        }
        let until = std::time::Instant::now() + TICK;
        while std::time::Instant::now() < until && !stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(200));
        }
    }
}

/// Run `watcher` and keep when it ran and how it went, as the scheduler
/// does: for "run now" in an app.
pub fn run_and_record(assistant: &Assistant, watchers: &Watchers, watcher: &Watcher, host: &dyn Host) -> ScheduleEvent {
    let (told, error) = match run_one(assistant, watchers, watcher, host) {
        Ok(told) => (told, None),
        Err(e) => (0, Some(e)),
    };
    watchers.update(&watcher.id, |s| {
        s.last_run = now_ms();
        s.last_error = error.clone();
        s.told += told;
    });
    ScheduleEvent::Watched { id: watcher.id.clone(), told, error }
}

/// Run `watcher` now: how many notifications it gave.
pub fn run_one(assistant: &Assistant, watchers: &Watchers, watcher: &Watcher, host: &dyn Host) -> Result<u32, String> {
    match &watcher.kind {
        WatchKind::Task { task } => {
            let before = assistant.inbox.list().first().map(|n| n.id.clone());
            let task = format!("{task}\n\n{}\nGive each notification source \"{}\".", told_before(assistant, &watcher.id), watcher.name);
            assistant.run_task(&task, host)?;
            let new: Vec<String> = assistant.inbox.list().into_iter().take_while(|n| Some(&n.id) != before.as_ref()).map(|n| n.id).collect();
            assistant.inbox.tag(&new, &watcher.id);
            Ok(new.len() as u32)
        }
        WatchKind::Tool { tool, arguments } => {
            let external = assistant.external().ok_or("the connectors aren't running")?;
            let (output, failed) = external.call(tool, arguments, Duration::from_secs(60))?;
            if failed {
                return Err(output);
            }
            let digest = hash(&output);
            let state = watchers.state(&watcher.id).unwrap_or_default();
            if state.digest == Some(digest) {
                return Ok(0);
            }
            let baseline = state.digest.is_none();
            let items = triage(assistant, watcher, &output, &state.seen, baseline)?;
            let mut told = 0;
            for item in &items {
                if baseline || state.seen.contains(&item.key) || !item.notify {
                    continue;
                }
                let urgency = item.urgency.min(watcher.max_urgency);
                assistant.inbox.post(&watcher.name, &item.title, &item.text, urgency, item.url.clone().filter(|u| !u.is_empty()), Some(watcher.id.clone()));
                told += 1;
            }
            watchers.update(&watcher.id, |s| {
                s.digest = Some(digest);
                for item in items {
                    if !s.seen.contains(&item.key) {
                        s.seen.push(item.key);
                    }
                }
                let excess = s.seen.len().saturating_sub(SEEN_KEEP);
                s.seen.drain(..excess);
            });
            Ok(told)
        }
    }
}

/// One item the model found in a watcher's output.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Item {
    /// Stable across runs: the item's own id, or what makes it unique.
    pub key: String,
    pub notify: bool,
    pub urgency: Urgency,
    pub title: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub url: Option<String>,
}

/// Ask the model which items of `output` are new and worth telling.
fn triage(assistant: &Assistant, watcher: &Watcher, output: &str, seen: &[String], baseline: bool) -> Result<Vec<Item>, String> {
    let max = if matches!(assistant.settings().brain(), Some(Brain::Groq | Brain::Ollama)) { OUTPUT_MAX_TIGHT } else { OUTPUT_MAX };
    let output: String = output.chars().take(max).collect();
    let seen = if seen.is_empty() { "(none)".to_string() } else { seen.iter().rev().take(200).cloned().collect::<Vec<_>>().join("\n") };
    let prompt = format!(
        "You are Yoru, the user's personal assistant, checking \"{name}\" for them. Below is what it returned now, \
the keys of the items already seen, and what the user cares about.\n\n\
List every item in the output (a mail, an event, an issue…) with a key that stays the same for that item across runs \
(its id if it has one). For each item whose key is not among the seen ones, decide whether the user would want to be told now \
(notify), and how urgently: low (just keep it in the inbox), normal, high (time-sensitive: someone waiting on them, \
something starting soon), urgent (can't wait; rare). Respect what the memory says to tell and not to tell. \
Write the title (a few words) and the text (one or two sentences) in the user's language.\n\n\
<watcher_instructions>\n{instructions}\n</watcher_instructions>\n\n<memory>\n{memory}\n</memory>\n\n\
<seen_keys>\n{seen}\n</seen_keys>\n\n<output>\n{output}\n</output>",
        name = watcher.name,
        instructions = if watcher.instructions.trim().is_empty() { "(none)" } else { watcher.instructions.trim() },
        memory = assistant.memory.for_prompt(),
    );
    let prompt = if baseline { format!("{prompt}\n\nThis is the first run: just list the items; nothing will be told.") } else { prompt };
    let schema = json!({
        "type": "object",
        "properties": {"items": {"type": "array", "items": {
            "type": "object",
            "properties": {
                "key": {"type": "string"},
                "notify": {"type": "boolean"},
                "urgency": {"type": "string", "enum": ["low", "normal", "high", "urgent"]},
                "title": {"type": "string"},
                "text": {"type": "string"},
                "url": {"type": "string"},
            },
            "required": ["key", "notify", "urgency", "title", "text", "url"],
            "additionalProperties": false,
        }}},
        "required": ["items"],
        "additionalProperties": false,
    });
    parse_items(&assistant.complete_json(&prompt, &schema)?)
}

/// The triage's answer. A model without a schema to follow (JSON mode) may
/// wrap it in a code fence, or leave out what's empty.
fn parse_items(text: &str) -> Result<Vec<Item>, String> {
    #[derive(Deserialize)]
    struct Found {
        items: Vec<Item>,
    }
    let text = text.trim();
    let text = text.strip_prefix("```json").or_else(|| text.strip_prefix("```")).map_or(text, |t| t.trim_end().trim_end_matches("```"));
    serde_json::from_str::<Found>(text.trim()).map(|f| f.items).map_err(|e| format!("the model's answer wasn't the expected JSON: {e}"))
}

/// The newest notifications a watcher gave, so a task doesn't repeat itself.
fn told_before(assistant: &Assistant, id: &str) -> String {
    let told: Vec<String> = assistant.inbox.list().into_iter().filter(|n| n.by.as_deref() == Some(id)).take(20).map(|n| format!("- {}: {}", n.title, n.text)).collect();
    if told.is_empty() {
        "You haven't told the user anything for this yet.".into()
    } else {
        format!("What you already told the user for this (don't repeat it):\n{}", told.join("\n"))
    }
}

fn hash(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn slug(name: &str) -> String {
    let s: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    if s.is_empty() { format!("w{}", now_ms()) } else { s }
}

/// Shared handle, for the app and the bridge.
pub type SharedWatchers = Arc<Watchers>;

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_watcher(id: &str) -> Watcher {
        Watcher {
            id: id.into(),
            name: "Unread mail".into(),
            kind: WatchKind::Tool { tool: "gmail__search".into(), arguments: json!({"q": "is:unread"}) },
            every_min: 10,
            enabled: true,
            instructions: String::new(),
            max_urgency: Urgency::High,
        }
    }

    #[test]
    fn watchers_are_kept_and_come_due() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("watchers.json");
        let w = Watchers::open(Some(file.clone()));
        w.put(Watcher { id: String::new(), every_min: 1, ..tool_watcher("") }).unwrap();
        let list = w.list();
        assert_eq!(list[0].watcher.id, "unread-mail");
        assert_eq!(list[0].watcher.every_min, 5, "at least five minutes");
        assert_eq!(w.due(now_ms()).len(), 1);
        w.update("unread-mail", |s| s.last_run = now_ms());
        assert!(w.due(now_ms()).is_empty());
        assert_eq!(w.due(now_ms() + 6 * 60_000).len(), 1);
        let reopened = Watchers::open(Some(file));
        assert_eq!(reopened.list().len(), 1);
        assert!(reopened.remove("unread-mail"));
        assert!(w.put(Watcher { name: " ".into(), ..tool_watcher("x") }).is_err());
    }

    #[test]
    fn a_changed_target_forgets_what_it_saw() {
        let w = Watchers::open(None);
        w.put(tool_watcher("m")).unwrap();
        w.update("m", |s| s.seen.push("abc".into()));
        w.put(Watcher { instructions: "only people".into(), ..tool_watcher("m") }).unwrap();
        assert_eq!(w.state("m").unwrap().seen, vec!["abc".to_string()]);
        w.put(Watcher { kind: WatchKind::Task { task: "news".into() }, ..tool_watcher("m") }).unwrap();
        assert!(w.state("m").unwrap().seen.is_empty());
    }

    #[test]
    fn triage_answers_are_read() {
        let items = parse_items(r#"{"items": [{"key": "m1", "notify": true, "urgency": "high", "title": "Marco", "text": "Needs you", "url": ""}]}"#).unwrap();
        assert_eq!((items[0].key.as_str(), items[0].urgency), ("m1", Urgency::High));
        // JSON mode: a fence, and the optional fields left out.
        let items = parse_items("```json\n{\"items\": [{\"key\": \"e2\", \"notify\": false, \"urgency\": \"low\", \"title\": \"Standup\"}]}\n```").unwrap();
        assert_eq!(items[0].text, "");
        assert!(parse_items(r#"{"items": [{"key": "x"}]}"#).is_err());
        assert!(parse_items("Sure! Here they are.").is_err());
    }

    #[test]
    fn kinds_serialize_flat() {
        let v = serde_json::to_value(tool_watcher("m")).unwrap();
        assert_eq!(v["kind"], "tool");
        assert_eq!(v["tool"], "gmail__search");
        let task: Watcher = serde_json::from_value(json!({"id": "n", "name": "News", "kind": "task", "task": "ESP32 news", "every_min": 120})).unwrap();
        assert!(task.enabled && task.max_urgency == Urgency::High);
        assert_eq!(slug("Mail da Marco!"), "mail-da-marco");
    }
}
