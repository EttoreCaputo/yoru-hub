//! The Claude Code hooks hook-up, for [`super::alerts`].
//!
//! [`connect`] adds `<exe> --claude-hook` to `hooks` in Claude Code's
//! settings.json for the events the alerts and the agent feed
//! ([`super::agents`]) need: `UserPromptSubmit` (a turn starts), `Stop` (it
//! ended), `Notification` for a permission prompt or a question,
//! `PreToolUse`, `PostToolUse` and `PostToolUseFailure` (each step) and
//! `SessionEnd`. Those run async, so Claude never waits on them. [`run`]
//! passes the hook's JSON to the running bridge through the hub and prints
//! nothing: a `UserPromptSubmit` hook's output would end up in Claude's context.
//!
//! `PermissionRequest` is the one that isn't async: it waits for an answer
//! from the app or the board (`host/claude_permission`) and prints Claude
//! Code's decision. Without the app, or with approvals off, it answers at
//! once with nothing and Claude Code asks in its terminal as usual.
//! [`disconnect`] takes ours out and leaves the others as they were.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;
use serde_json::{Map, Value, json};

use super::statusline::{backup, parse_object, read_settings, write_atomic, write_settings};
use super::agents::Decision;
use super::{config_dir, data_dir};
use crate::hub::Board;

/// The argument that turns the app binary into the hook.
pub const FLAG: &str = "--claude-hook";

/// Our hook for each event, the `matcher` it gets, and whether Claude waits
/// for it.
const EVENTS: [(&str, Option<&str>, bool); 8] = [
    ("UserPromptSubmit", None, false),
    ("Stop", None, false),
    ("Notification", Some("permission_prompt|elicitation_dialog"), false),
    ("PreToolUse", None, false),
    ("PostToolUse", None, false),
    ("PostToolUseFailure", None, false),
    ("SessionEnd", None, false),
    ("PermissionRequest", None, true),
];

/// How long Claude Code lets the `PermissionRequest` hook wait: past the
/// longest wait the bridge allows ([`super::alerts::MAX_APPROVAL_WAIT_S`]).
const PERMISSION_TIMEOUT_S: u32 = 600;

#[derive(Debug, Clone, Serialize)]
pub struct HooksStatus {
    /// settings.json has our hook for every event.
    pub connected: bool,
    /// Seconds since Claude Code last ran the hook.
    pub last_event_s: Option<u64>,
    pub settings_path: Option<PathBuf>,
}

pub fn status() -> HooksStatus {
    Paths::default_locations().status()
}

/// Add `exe` as Claude Code's hook for the alerts' events, with a one-off
/// backup of settings.json.
pub fn connect(exe: &Path) -> io::Result<HooksStatus> {
    Paths::default_locations().connect(exe)
}

/// Take our hooks out of settings.json.
pub fn disconnect() -> io::Result<HooksStatus> {
    Paths::default_locations().disconnect()
}

struct Paths {
    settings: Option<PathBuf>,
    /// Touched on every run, for [`HooksStatus::last_event_s`].
    last_event: Option<PathBuf>,
}

impl Paths {
    fn default_locations() -> Self {
        Self { settings: config_dir().map(|d| d.join("settings.json")), last_event: last_event_path() }
    }

    fn status(&self) -> HooksStatus {
        let connected = self.settings.as_deref().and_then(read_settings).is_some_and(|s| has_ours(&s));
        let last_event_s = self
            .last_event
            .as_ref()
            .and_then(|p| fs::metadata(p).and_then(|m| m.modified()).ok())
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .map(|d| d.as_secs());
        HooksStatus { connected, last_event_s, settings_path: self.settings.clone() }
    }

    fn settings_path(&self) -> io::Result<&Path> {
        self.settings.as_deref().ok_or_else(|| io::Error::other("can't find the Claude Code config folder"))
    }

    fn connect(&self, exe: &Path) -> io::Result<HooksStatus> {
        let path = self.settings_path()?;
        let mut settings = match fs::read_to_string(path) {
            Ok(text) => parse_object(&text)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Map::new(),
            Err(e) => return Err(e),
        };
        if !has_any_ours(&settings) {
            backup(path)?;
        }
        // A moved app: replace the old command rather than adding a second one.
        remove_ours(&mut settings);
        let command = format!("\"{}\" {FLAG}", exe.display());
        let hooks = settings.entry("hooks").or_insert_with(|| Value::Object(Map::new()));
        let hooks = hooks.as_object_mut().ok_or_else(|| io::Error::other("`hooks` in settings.json is not an object"))?;
        for (event, matcher, waits) in EVENTS {
            let groups = hooks.entry(event).or_insert_with(|| Value::Array(Vec::new()));
            let groups = groups.as_array_mut().ok_or_else(|| io::Error::other(format!("`hooks.{event}` in settings.json is not a list")))?;
            let mut group = Map::new();
            if let Some(m) = matcher {
                group.insert("matcher".into(), m.into());
            }
            let hook = if waits {
                json!({"type": "command", "command": command, "timeout": PERMISSION_TIMEOUT_S})
            } else {
                json!({"type": "command", "command": command, "async": true, "timeout": 10})
            };
            group.insert("hooks".into(), json!([hook]));
            groups.push(Value::Object(group));
        }
        write_settings(path, &settings)?;
        Ok(self.status())
    }

    fn disconnect(&self) -> io::Result<HooksStatus> {
        let path = self.settings_path()?;
        if let Ok(text) = fs::read_to_string(path) {
            let mut settings = parse_object(&text)?;
            if has_any_ours(&settings) {
                remove_ours(&mut settings);
                write_settings(path, &settings)?;
            }
        }
        Ok(self.status())
    }
}

fn is_ours(hook: &Value) -> bool {
    hook.get("command").and_then(Value::as_str).is_some_and(|c| c.contains(FLAG))
}

/// The hook entries of each group of each event.
fn entries(settings: &Map<String, Value>) -> impl Iterator<Item = (&str, &Value)> {
    let hooks = settings.get("hooks").and_then(Value::as_object);
    hooks.into_iter().flatten().flat_map(|(event, groups)| {
        let groups = groups.as_array().into_iter().flatten();
        groups.flat_map(move |g| g.get("hooks").and_then(Value::as_array).into_iter().flatten().map(move |h| (event.as_str(), h)))
    })
}

fn has_ours(settings: &Map<String, Value>) -> bool {
    EVENTS.iter().all(|(event, _, _)| entries(settings).any(|(e, h)| e == *event && is_ours(h)))
}

fn has_any_ours(settings: &Map<String, Value>) -> bool {
    entries(settings).any(|(_, h)| is_ours(h))
}

/// Take our entries out, then the groups, events and `hooks` left empty by it.
fn remove_ours(settings: &mut Map<String, Value>) {
    let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) else { return };
    for groups in hooks.values_mut() {
        let Some(groups) = groups.as_array_mut() else { continue };
        groups.retain_mut(|group| {
            let Some(list) = group.get_mut("hooks").and_then(Value::as_array_mut) else { return true };
            let before = list.len();
            list.retain(|h| !is_ours(h));
            !(list.is_empty() && before > 0)
        });
    }
    hooks.retain(|_, groups| groups.as_array().is_none_or(|g| !g.is_empty()));
    if hooks.is_empty() {
        settings.remove("hooks");
    }
}

fn last_event_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("claude-hook-last"))
}

/// The hook itself: hand the JSON on `input` to the running bridge. Never
/// fails loudly; without the app running it does nothing. Only a
/// `PermissionRequest` that was answered prints, Claude Code's decision.
pub fn run(mut input: impl Read) {
    let mut json = Vec::new();
    if input.read_to_end(&mut json).is_err() {
        return;
    }
    if let Some(path) = last_event_path() {
        let _ = write_atomic(&path, b"");
    }
    let Ok(event) = serde_json::from_slice::<Value>(&json) else { return };
    let board = Board::new(None, &format!("companion-hook/{}", env!("CARGO_PKG_VERSION")));
    if event.get("hook_event_name").and_then(Value::as_str) == Some("PermissionRequest") {
        if let Ok(Some(decision)) = board.claude_permission(event) {
            println!("{}", permission_output(decision));
        }
        return;
    }
    let _ = board.claude_hook(event);
}

/// What a `PermissionRequest` hook prints for `decision`.
fn permission_output(decision: Decision) -> Value {
    let decision = match decision {
        Decision::Allow => json!({"behavior": "allow"}),
        Decision::Deny => json!({"behavior": "deny", "message": "The user said no from Companion Hub."}),
    };
    json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": decision}})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(dir: &tempfile::TempDir) -> Paths {
        Paths { settings: Some(dir.path().join("settings.json")), last_event: None }
    }

    #[test]
    fn connect_keeps_other_hooks_and_disconnect_restores() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(&dir);
        let theirs = json!({"model": "opus", "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}});
        fs::write(p.settings.as_ref().unwrap(), theirs.to_string()).unwrap();

        assert!(p.connect(Path::new("/opt/companion-hub/companion-hub")).unwrap().connected);
        let now: Value = serde_json::from_str(&fs::read_to_string(p.settings.as_ref().unwrap()).unwrap()).unwrap();
        assert_eq!(now["model"], "opus");
        assert_eq!(now["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert_eq!(now["hooks"]["Stop"][0]["hooks"][0]["command"], "say done");
        assert_eq!(now["hooks"]["Notification"][0]["matcher"], "permission_prompt|elicitation_dialog");
        assert_eq!(now["hooks"]["UserPromptSubmit"][0]["hooks"][0]["async"], true);
        // The approvals wait: not async, with a long timeout.
        assert!(now["hooks"]["PermissionRequest"][0]["hooks"][0].get("async").is_none());
        assert_eq!(now["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"], PERMISSION_TIMEOUT_S);
        assert!(dir.path().join("settings.json.companion-hub-backup").exists());

        // Connecting again from a moved app replaces ours.
        p.connect(Path::new("/Applications/Companion Hub.app/companion-hub")).unwrap();
        let now: Value = serde_json::from_str(&fs::read_to_string(p.settings.as_ref().unwrap()).unwrap()).unwrap();
        assert_eq!(now["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert_eq!(now["hooks"]["Stop"][1]["hooks"][0]["command"], "\"/Applications/Companion Hub.app/companion-hub\" --claude-hook");

        assert!(!p.disconnect().unwrap().connected);
        let back: Value = serde_json::from_str(&fs::read_to_string(p.settings.as_ref().unwrap()).unwrap()).unwrap();
        assert_eq!(back, theirs);
    }

    #[test]
    fn permission_decisions() {
        let allow = permission_output(Decision::Allow);
        assert_eq!(allow["hookSpecificOutput"]["hookEventName"], "PermissionRequest");
        assert_eq!(allow["hookSpecificOutput"]["decision"]["behavior"], "allow");
        assert_eq!(permission_output(Decision::Deny)["hookSpecificOutput"]["decision"]["behavior"], "deny");
    }

    #[test]
    fn disconnect_removes_hooks_it_emptied() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(&dir);
        p.connect(Path::new("/x/companion-hub")).unwrap();
        p.disconnect().unwrap();
        let back: Value = serde_json::from_str(&fs::read_to_string(p.settings.as_ref().unwrap()).unwrap()).unwrap();
        assert!(back.get("hooks").is_none());
    }
}
