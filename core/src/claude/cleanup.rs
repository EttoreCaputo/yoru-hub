//! Take out of Claude Code's settings.json what older versions of the app
//! put there: a hook (`"<exe>" --claude-hook`) on several events and the
//! status line (`"<exe>" --claude-statusline`). Those flags are gone, so
//! Claude Code would run a command that fails on every event.
//!
//! [`remove_legacy`] runs at the app's start. It touches only entries whose
//! command is exactly one of those two, run by Yoru's own executable, and
//! leaves every other hook and setting as it was. The status line that was
//! there before Yoru's is put back. Once nothing of ours is left it changes
//! nothing, so running it at every start is a one-off in effect.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::{config_dir, parse_object, write_settings};

const HOOK_FLAG: &str = "--claude-hook";
const STATUSLINE_FLAG: &str = "--claude-statusline";
/// The executables that set them: the app and the CLI.
const EXECUTABLES: [&str; 2] = ["yoru-app", "yoru"];

/// What [`remove_legacy`] took out.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Removed {
    /// Hook entries removed.
    pub hooks: usize,
    /// The status line was ours: put back as it was before, or removed.
    pub status_line: bool,
}

/// Remove Yoru's old hooks and status line from Claude Code's settings.json,
/// and the files they left in Yoru's folder.
pub fn remove_legacy() -> io::Result<Removed> {
    let data = crate::paths::data_dir();
    let removed = remove_at(config_dir().map(|d| d.join("settings.json")).as_deref(), data.as_deref())?;
    if let Some(data) = data {
        for file in ["claude-status.json", "claude-hook-last", PREVIOUS] {
            remove_if_exists(&data.join(file))?;
        }
    }
    Ok(removed)
}

/// Where the status line that was there before Yoru's was kept.
const PREVIOUS: &str = "claude-statusline-previous.json";

fn remove_at(settings: Option<&Path>, data: Option<&Path>) -> io::Result<Removed> {
    let Some(path) = settings else { return Ok(Removed::default()) };
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Removed::default()),
        Err(e) => return Err(e),
    };
    let mut settings = parse_object(&text)?;
    let removed = Removed { hooks: remove_hooks(&mut settings), status_line: remove_status_line(&mut settings, data.map(|d| d.join(PREVIOUS))) };
    if removed != Removed::default() {
        write_settings(path, &settings)?;
    }
    Ok(removed)
}

/// `"<dir>/yoru-app" <flag>`, as the app wrote it, and nothing else.
fn is_ours(command: Option<&str>, flag: &str) -> bool {
    let Some(exe) = command.and_then(|c| c.trim().strip_suffix(flag)).map(str::trim_end) else { return false };
    let Some(exe) = exe.strip_prefix('"').and_then(|e| e.strip_suffix('"')) else { return false };
    // By hand rather than with `Path`: a Windows path reads the same on any system.
    let name = exe.rsplit(['/', '\\']).next().unwrap_or_default();
    let name = name.strip_suffix(".exe").unwrap_or(name);
    EXECUTABLES.contains(&name)
}

fn command(entry: &Value) -> Option<&str> {
    entry.get("command").and_then(Value::as_str)
}

/// Take our hook entries out, then the groups and events left empty by it.
fn remove_hooks(settings: &mut Map<String, Value>) -> usize {
    let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) else { return 0 };
    let mut removed = 0;
    let mut emptied = Vec::new();
    for (event, groups) in hooks.iter_mut() {
        let Some(groups) = groups.as_array_mut() else { continue };
        let before = groups.len();
        groups.retain_mut(|group| {
            let Some(list) = group.get_mut("hooks").and_then(Value::as_array_mut) else { return true };
            let had = list.len();
            list.retain(|h| !is_ours(command(h), HOOK_FLAG));
            removed += had - list.len();
            !(list.is_empty() && had > list.len())
        });
        if groups.is_empty() && before > 0 {
            emptied.push(event.clone());
        }
    }
    for event in emptied {
        hooks.remove(&event);
    }
    if removed > 0 && hooks.is_empty() {
        settings.remove("hooks");
    }
    removed
}

/// Put back the status line Yoru's replaced, or take ours out.
fn remove_status_line(settings: &mut Map<String, Value>, previous: Option<PathBuf>) -> bool {
    if !is_ours(settings.get("statusLine").and_then(command), STATUSLINE_FLAG) {
        return false;
    }
    let previous = previous
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .filter(|p| p.is_object() && !is_ours(command(p), STATUSLINE_FLAG));
    match previous {
        Some(line) => settings.insert("statusLine".into(), line),
        None => settings.remove("statusLine"),
    };
    true
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const HOOK: &str = "\"/Applications/Yoru.app/Contents/MacOS/yoru-app\" --claude-hook";
    const LINE: &str = "\"/x/target/release/bundle/macos/Yoru.app/Contents/MacOS/yoru-app\" --claude-statusline";

    fn ours(event_matcher: Option<&str>) -> Value {
        let mut group = json!({"hooks": [{"type": "command", "command": HOOK, "async": true, "timeout": 10}]});
        if let Some(m) = event_matcher {
            group["matcher"] = json!(m);
        }
        group
    }

    #[test]
    fn recognises_only_our_commands() {
        assert!(is_ours(Some(HOOK), HOOK_FLAG));
        assert!(is_ours(Some("\"C:\\Yoru\\yoru-app.exe\" --claude-hook"), HOOK_FLAG));
        assert!(is_ours(Some(LINE), STATUSLINE_FLAG));
        assert!(!is_ours(Some(HOOK), STATUSLINE_FLAG));
        // Someone else's program with the same flag, or ours with more after it.
        assert!(!is_ours(Some("\"/opt/dualeye/companion\" --claude-hook"), HOOK_FLAG));
        assert!(!is_ours(Some("\"/x/yoru-app\" --claude-hook --verbose"), HOOK_FLAG));
        assert!(!is_ours(Some("say done"), HOOK_FLAG));
        assert!(!is_ours(None, HOOK_FLAG));
    }

    #[test]
    fn takes_out_ours_and_keeps_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let theirs_hook = json!({"type": "command", "command": "say done"});
        let settings = json!({
            "model": "opus",
            "hooks": {
                "Stop": [{"hooks": [theirs_hook.clone()]}, ours(None)],
                "Notification": [ours(Some("permission_prompt|elicitation_dialog"))],
                "PreToolUse": [{"matcher": "Bash", "hooks": [theirs_hook.clone(), {"type": "command", "command": HOOK}]}],
            },
            "statusLine": {"type": "command", "command": LINE, "padding": 1},
        });
        fs::write(&path, settings.to_string()).unwrap();
        fs::write(dir.path().join(PREVIOUS), json!({"type": "command", "command": "~/bin/line.sh"}).to_string()).unwrap();

        let removed = remove_at(Some(&path), Some(dir.path())).unwrap();
        assert_eq!(removed, Removed { hooks: 3, status_line: true });
        let now: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            now,
            json!({
                "model": "opus",
                "hooks": {
                    "Stop": [{"hooks": [theirs_hook.clone()]}],
                    "PreToolUse": [{"matcher": "Bash", "hooks": [theirs_hook]}],
                },
                "statusLine": {"type": "command", "command": "~/bin/line.sh"},
            })
        );

        // Nothing of ours left: the file isn't written again.
        let before = fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(remove_at(Some(&path), Some(dir.path())).unwrap(), Removed::default());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
    }

    #[test]
    fn without_a_previous_status_line_ours_goes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, json!({"hooks": {"Stop": [ours(None)]}, "statusLine": {"type": "command", "command": LINE}}).to_string()).unwrap();
        assert_eq!(remove_at(Some(&path), None).unwrap(), Removed { hooks: 1, status_line: true });
        let now: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(now, json!({}));
    }

    #[test]
    fn someone_elses_settings_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let text = "{\"statusLine\": {\"type\": \"command\", \"command\": \"~/.claude/statusline.sh\"}}";
        fs::write(&path, text).unwrap();
        assert_eq!(remove_at(Some(&path), None).unwrap(), Removed::default());
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        assert_eq!(remove_at(Some(&dir.path().join("missing.json")), None).unwrap(), Removed::default());
    }
}
