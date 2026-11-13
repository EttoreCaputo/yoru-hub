//! What a coding agent's conversation is doing now, read from what the
//! agent keeps (Claude Code's and Codex's `.jsonl` files, OpenCode's
//! database): the last thing the user asked it, its last steps (a command
//! run, a file edited), the last thing it said, and whether it is working,
//! has finished its turn or seems to wait for the user.
//!
//! Nothing here sees the agents' screens: an approval the agent asks for
//! isn't written anywhere, so "waiting" is a guess, a step started a while
//! ago with nothing written since. A long build looks the same, and the
//! words Yoru says about it are as careful.

use std::collections::HashSet;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;
use serde_json::Value;

use super::{Agent, clip, opencode_db};

/// Steps kept: the newest.
pub const STEPS: usize = 6;
/// A step started this long ago with nothing written since may be an
/// approval the agent is waiting for.
pub const STALLED_MS: u64 = 45_000;
/// An answer with nothing written after it for this long ended the turn,
/// when the agent doesn't say so itself.
const SETTLED_MS: u64 = 15_000;
/// What is read of a long conversation: its end.
const TAIL_BYTES: u64 = 768 * 1024;

/// A conversation, now.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Peek {
    /// `working`; `waiting`: a step started a while ago and nothing since
    /// (an approval, or a long command); `your_turn`: it finished its turn
    /// and waits for the user's next message; `interrupted`.
    pub state: &'static str,
    /// The last thing the user asked it.
    pub asked: Option<String>,
    /// Its last steps, oldest first.
    pub steps: Vec<String>,
    /// The last thing it said.
    pub said: Option<String>,
    /// Unix ms of its last change.
    pub updated: u64,
}

/// The conversation `id` of `agent`, now (`now` in Unix ms).
pub fn peek(home: &Path, agent: Agent, id: &str, now: u64) -> Option<Peek> {
    match agent {
        Agent::Claude => claude_file(home, id).and_then(|f| from_lines(&f, now, claude_line)),
        Agent::Codex => codex_file(home, id).and_then(|f| from_lines(&f, now, codex_line)),
        Agent::OpenCode => opencode(home, id, now),
    }
}

/// One step, in a few words: `ran cargo build`, `edited main.rs`.
pub fn step(tool: &str, input: &Value) -> String {
    let text = |k: &str| input[k].as_str().map(|v| clip(&v.replace('\n', " "), 80)).filter(|v| !v.is_empty());
    let file = || {
        ["file_path", "filePath", "path", "notebook_path"]
            .iter()
            .find_map(|k| input[*k].as_str())
            .map(|p| Path::new(p).file_name().map_or(p.to_string(), |n| n.to_string_lossy().into_owned()))
    };
    match tool.to_lowercase().as_str() {
        "bash" | "shell" | "exec_command" | "local_shell" | "command_execution" | "shell_command" => {
            format!("ran {}", command(input).unwrap_or_else(|| "a command".into()))
        }
        "edit" | "multiedit" | "write" | "notebookedit" | "patch" | "file_change" => file().map_or("edited files".into(), |f| format!("edited {f}")),
        "apply_patch" => input.as_str().and_then(patched_file).or_else(file).map_or("edited files".into(), |f| format!("edited {f}")),
        "read" | "view" => format!("read {}", file().unwrap_or_else(|| "a file".into())),
        "grep" | "glob" | "search" | "find" | "list" | "ls" | "codesearch" => format!("searched for {}", text("pattern").or(text("query")).unwrap_or_else(|| "files".into())),
        "websearch" | "web_search" => format!("searched the web for {}", text("query").unwrap_or_else(|| "something".into())),
        "webfetch" | "fetch" => format!("read {}", text("url").unwrap_or_else(|| "a web page".into())),
        "task" | "agent" => format!("handed a part to a helper: {}", text("description").or(text("prompt")).unwrap_or_default()).trim_end_matches([':', ' ']).to_string(),
        "todowrite" | "update_plan" | "todo_list" | "todoread" => "updated its plan".into(),
        "askuserquestion" | "question" => format!("asked you: {}", question(input).unwrap_or_default()).trim_end_matches([':', ' ']).to_string(),
        "exitplanmode" => "showed you its plan".into(),
        // Codex's app: a script calling its other tools (`tools.apply_patch(…)`).
        "exec" if input.is_string() => codex_script(input.as_str().unwrap_or_default()),
        _ => format!("used {tool}"),
    }
}

/// The command a shell step ran: a string, or Codex's `["bash", "-lc", "…"]`.
fn command(input: &Value) -> Option<String> {
    let c = input.get("command").or(input.get("cmd"))?;
    let line = match c {
        Value::String(s) => s.clone(),
        Value::Array(parts) => {
            let parts: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
            match parts.as_slice() {
                [shell, flag, script] if shell.ends_with("sh") && flag.starts_with('-') => script.to_string(),
                _ => parts.join(" "),
            }
        }
        _ => return None,
    };
    let line = line.trim().replace('\n', " ");
    (!line.is_empty()).then(|| clip(&line, 80))
}

/// A script of Codex's app, by the first tool it calls.
fn codex_script(script: &str) -> String {
    // The script's strings keep their escapes.
    let script = script.replace("\\n", "\n").replace("\\\"", "\"");
    let quoted = |key: &str| {
        let start = script.find(&format!("{key}:\""))? + key.len() + 2;
        Some(clip(&script[start..start + script[start..].find('"')?], 80))
    };
    let tool = script.split("tools.").nth(1).and_then(|t| t.split('(').next()).unwrap_or_default();
    match tool {
        "apply_patch" => patched_file(&script).map_or("edited files".into(), |f| format!("edited {f}")),
        "web__run" => match (quoted("q"), quoted("ref_id")) {
            (Some(q), _) => format!("searched the web for {q}"),
            (None, Some(url)) => format!("read {url}"),
            _ => "searched the web".into(),
        },
        "" => "ran a script".into(),
        other => match quoted("cmd").or(quoted("command")) {
            Some(c) => format!("ran {c}"),
            None => format!("used {other}"),
        },
    }
}

/// The first file a patch (`*** Update File: path`) changes.
fn patched_file(patch: &str) -> Option<String> {
    patch.lines().find_map(|l| {
        let p = l.strip_prefix("*** Update File: ").or(l.strip_prefix("*** Add File: ")).or(l.strip_prefix("*** Delete File: "))?;
        Some(Path::new(p.trim()).file_name()?.to_string_lossy().into_owned())
    })
}

fn question(input: &Value) -> Option<String> {
    input["questions"][0]["question"].as_str().or(input["question"].as_str()).map(|q| clip(q, 120))
}

/// What the user typed, when it is theirs: not a tool's result, not the
/// app's own context blocks.
fn typed(text: &str) -> Option<String> {
    let t = text.trim();
    (!t.is_empty() && !t.starts_with('<') && !t.starts_with("Caveat:")).then(|| clip(&t.replace('\n', " "), 200))
}

/// What reading a conversation found so far.
#[derive(Default)]
struct Reading {
    asked: Option<String>,
    steps: Vec<String>,
    said: Option<String>,
    /// Steps started and not finished, by id.
    open: HashSet<String>,
    /// The agent said its turn ended.
    ended: bool,
    interrupted: bool,
}

impl Reading {
    fn asked(&mut self, text: String) {
        *self = Reading { asked: Some(text), ..Reading::default() };
    }

    fn step(&mut self, id: Option<&str>, line: String) {
        if self.steps.last() != Some(&line) {
            self.steps.push(line);
        }
        let excess = self.steps.len().saturating_sub(STEPS);
        self.steps.drain(..excess);
        if let Some(id) = id {
            self.open.insert(id.to_string());
        }
        self.ended = false;
    }

    fn said(&mut self, text: &str) {
        if let Some(t) = typed(text).map(|_| clip(text.trim(), 600)) {
            self.said = Some(t);
        }
    }

    fn peek(self, updated: u64, now: u64) -> Peek {
        let quiet = now.saturating_sub(updated);
        let state = if self.interrupted {
            "interrupted"
        } else if !self.open.is_empty() {
            if quiet > STALLED_MS { "waiting" } else { "working" }
        } else if self.ended || (self.said.is_some() && quiet > SETTLED_MS) {
            "your_turn"
        } else {
            "working"
        };
        Peek { state, asked: self.asked, steps: self.steps, said: self.said, updated }
    }
}

/// A conversation kept as JSON lines, read from its end.
fn from_lines(path: &Path, now: u64, read: fn(&mut Reading, &Value)) -> Option<Peek> {
    let updated = modified(path)?;
    let mut file = fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let start = length.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let mut lines = text.lines();
    if start > 0 {
        // Cut in the middle of a line.
        lines.next();
    }
    let mut reading = Reading::default();
    for line in lines {
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            read(&mut reading, &v);
        }
    }
    Some(reading.peek(updated, now))
}

fn modified(path: &Path) -> Option<u64> {
    let t = fs::metadata(path).ok()?.modified().ok()?;
    Some(t.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64)
}

/// `~/.claude/projects/<folder>/<id>.jsonl`.
fn claude_file(home: &Path, id: &str) -> Option<PathBuf> {
    let name = format!("{id}.jsonl");
    fs::read_dir(home.join(".claude/projects")).ok()?.flatten().map(|d| d.path().join(&name)).find(|p| p.is_file())
}

/// One line of Claude Code's conversation.
fn claude_line(r: &mut Reading, v: &Value) {
    if v["isSidechain"] == true {
        return;
    }
    let content = &v["message"]["content"];
    match v["type"].as_str() {
        Some("user") => {
            if let Some(text) = content.as_str() {
                if text.contains("[Request interrupted by user") {
                    r.interrupted = true;
                } else if let Some(t) = typed(text) {
                    r.asked(t);
                }
                return;
            }
            for c in content.as_array().into_iter().flatten() {
                match c["type"].as_str() {
                    Some("tool_result") => {
                        if let Some(id) = c["tool_use_id"].as_str() {
                            r.open.remove(id);
                        }
                    }
                    Some("text") => {
                        let text = c["text"].as_str().unwrap_or_default();
                        if text.contains("[Request interrupted by user") {
                            r.interrupted = true;
                        } else if let Some(t) = typed(text) {
                            r.asked(t);
                        }
                    }
                    _ => {}
                }
            }
        }
        Some("assistant") => {
            for c in content.as_array().into_iter().flatten() {
                match c["type"].as_str() {
                    Some("tool_use") => r.step(c["id"].as_str(), step(c["name"].as_str().unwrap_or("a tool"), &c["input"])),
                    Some("text") => r.said(c["text"].as_str().unwrap_or_default()),
                    _ => {}
                }
            }
            if v["message"]["stop_reason"] == "end_turn" {
                r.ended = true;
            }
        }
        // Written when a turn ends (its hooks ran, or how long it took).
        Some("system") if matches!(v["subtype"].as_str(), Some("stop_hook_summary" | "turn_duration")) => r.ended = r.open.is_empty(),
        _ => {}
    }
}

/// `~/.codex/sessions/<y>/<m>/<d>/rollout-<time>-<id>.jsonl`.
fn codex_file(home: &Path, id: &str) -> Option<PathBuf> {
    let suffix = format!("-{id}.jsonl");
    let mut stack = vec![home.join(".codex/sessions")];
    while let Some(dir) = stack.pop() {
        for e in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(&suffix)) {
                return Some(p);
            }
        }
    }
    None
}

/// One line of Codex's conversation.
fn codex_line(r: &mut Reading, v: &Value) {
    let p = &v["payload"];
    match (v["type"].as_str(), p["type"].as_str()) {
        (Some("event_msg"), Some("user_message")) => {
            if let Some(t) = p["message"].as_str().and_then(typed).filter(|t| r.asked.as_deref() != Some(t.as_str())) {
                r.asked(t);
            }
        }
        (Some("response_item"), Some("message")) => {
            let texts = p["content"].as_array().into_iter().flatten().filter_map(|c| c["text"].as_str());
            match p["role"].as_str() {
                Some("user") => {
                    if let Some(t) = texts.into_iter().find_map(typed).filter(|t| r.asked.as_deref() != Some(t.as_str())) {
                        r.asked(t);
                    }
                }
                Some("assistant") => {
                    if let Some(t) = texts.into_iter().next_back() {
                        r.said(t);
                    }
                }
                _ => {}
            }
        }
        (Some("response_item"), Some("function_call" | "local_shell_call")) => {
            let args = p["arguments"].as_str().and_then(|a| serde_json::from_str::<Value>(a).ok()).unwrap_or_else(|| p["action"].clone());
            r.step(p["call_id"].as_str(), step(p["name"].as_str().unwrap_or("shell"), &args));
        }
        (Some("response_item"), Some("custom_tool_call")) => r.step(p["call_id"].as_str(), step(p["name"].as_str().unwrap_or("a tool"), &p["input"])),
        (Some("response_item"), Some(kind)) if kind.ends_with("_output") => {
            if let Some(id) = p["call_id"].as_str() {
                r.open.remove(id);
            }
        }
        (Some("event_msg"), Some("task_started")) => r.ended = false,
        (Some("event_msg"), Some("task_complete")) => {
            if let Some(t) = p["last_agent_message"].as_str() {
                r.said(t);
            }
            r.open.clear();
            r.ended = true;
        }
        (Some("event_msg"), Some("turn_aborted")) => r.interrupted = true,
        _ => {}
    }
}

/// OpenCode's conversation, from its database's `session_message` (its
/// newer table; older conversations aren't read).
fn opencode(home: &Path, id: &str, now: u64) -> Option<Peek> {
    let path = opencode_db(home);
    let db = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let mut query = db.prepare("SELECT type, data, time_updated FROM session_message WHERE session_id = ?1 ORDER BY seq DESC LIMIT 80").ok()?;
    let mut rows: Vec<(String, Value, u64)> = query
        .query_map([id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?)))
        .ok()?
        .flatten()
        .filter_map(|(kind, data, at)| Some((kind, serde_json::from_str(&data).ok()?, at.max(0) as u64)))
        .collect();
    if rows.is_empty() {
        return None;
    }
    rows.reverse();
    let updated = rows.iter().map(|r| r.2).max().unwrap_or(0);
    let mut r = Reading::default();
    for (kind, data, _) in &rows {
        match kind.as_str() {
            "user" => {
                if let Some(t) = data["text"].as_str().and_then(typed) {
                    r.asked(t);
                }
            }
            "assistant" => {
                for c in data["content"].as_array().into_iter().flatten() {
                    match c["type"].as_str() {
                        Some("tool") => {
                            let id = c["id"].as_str().unwrap_or_default();
                            r.step(Some(id), step(c["name"].as_str().unwrap_or("a tool"), &c["state"]["input"]));
                            if matches!(c["state"]["status"].as_str(), Some("completed" | "error")) {
                                r.open.remove(id);
                            }
                        }
                        Some("text") => r.said(c["text"].as_str().unwrap_or_default()),
                        _ => {}
                    }
                }
            }
            "idle" => {
                r.open.clear();
                r.ended = true;
                r.interrupted = data["outcome"] == "interrupted";
            }
            _ => {}
        }
    }
    Some(r.peek(updated, now))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn steps_in_a_few_words() {
        assert_eq!(step("Bash", &json!({"command": "cargo build --release"})), "ran cargo build --release");
        assert_eq!(step("shell", &json!({"command": ["bash", "-lc", "npm test"]})), "ran npm test");
        assert_eq!(step("Edit", &json!({"file_path": "/x/src/main.rs"})), "edited main.rs");
        assert_eq!(step("apply_patch", &json!("*** Begin Patch\n*** Update File: app/ui.ts\n@@")), "edited ui.ts");
        assert_eq!(step("Task", &json!({"description": "Find the zoom code"})), "handed a part to a helper: Find the zoom code");
        assert_eq!(step("mcp__x__y", &json!({})), "used mcp__x__y");
        let patch = r#"const r = await tools.apply_patch("*** Begin Patch\n*** Add File: hello.txt\n+ciao\n*** End Patch");"#;
        assert_eq!(step("exec", &json!(patch)), "edited hello.txt");
        assert_eq!(step("exec", &json!(r#"await tools.web__run({search_query:[{q:"atac gps"}]})"#)), "searched the web for atac gps");
        assert_eq!(step("exec", &json!(r#"await tools.exec_command({cmd:"cargo test"})"#)), "ran cargo test");
    }

    fn lines(path: &Path, lines: &[Value]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, lines.iter().map(|l| l.to_string() + "\n").collect::<String>()).unwrap();
    }

    #[test]
    fn reads_claudes_conversation_now() {
        let h = std::env::temp_dir().join(format!("yoru-peek-{}", std::process::id()));
        let file = h.join(".claude/projects/-x/s1.jsonl");
        let ask = json!({"type": "user", "message": {"content": "fix the zoom"}});
        let run = json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "Let me build it."}, {"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "cargo build"}}]}});
        lines(&file, &[ask.clone(), run.clone()]);
        let now = modified(&file).unwrap();
        let p = peek(&h, Agent::Claude, "s1", now).unwrap();
        assert_eq!((p.state, p.asked.as_deref(), p.steps.as_slice()), ("working", Some("fix the zoom"), ["ran cargo build".to_string()].as_slice()));
        // The same step, long after: maybe an approval.
        assert_eq!(peek(&h, Agent::Claude, "s1", now + STALLED_MS + 1).unwrap().state, "waiting");
        let done = json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "t1"}]}});
        let said = json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "Fixed."}], "stop_reason": "end_turn"}});
        lines(&file, &[ask, run, done, said]);
        let p = peek(&h, Agent::Claude, "s1", now).unwrap();
        assert_eq!((p.state, p.said.as_deref()), ("your_turn", Some("Fixed.")));
        fs::remove_dir_all(&h).unwrap();
    }

    #[test]
    fn reads_codexs_conversation_now() {
        let h = std::env::temp_dir().join(format!("yoru-peek-codex-{}", std::process::id()));
        let file = h.join(".codex/sessions/2026/11/13/rollout-2026-11-13T10-00-00-c1.jsonl");
        lines(
            &file,
            &[
                json!({"type": "event_msg", "payload": {"type": "task_started"}}),
                json!({"type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "<environment_context>"}, {"type": "input_text", "text": "bigger menu"}]}}),
                json!({"type": "event_msg", "payload": {"type": "user_message", "message": "bigger menu"}}),
                json!({"type": "response_item", "payload": {"type": "function_call", "name": "shell", "call_id": "c1", "arguments": "{\"command\":[\"bash\",\"-lc\",\"rg menu\"]}"}}),
            ],
        );
        let now = modified(&file).unwrap();
        let p = peek(&h, Agent::Codex, "c1", now).unwrap();
        assert_eq!((p.state, p.asked.as_deref(), p.steps.first().map(String::as_str)), ("working", Some("bigger menu"), Some("ran rg menu")));
        fs::OpenOptions::new()
            .append(true)
            .open(&file)
            .and_then(|mut f| {
                use std::io::Write;
                writeln!(f, "{}", json!({"type": "response_item", "payload": {"type": "function_call_output", "call_id": "c1"}}))?;
                writeln!(f, "{}", json!({"type": "event_msg", "payload": {"type": "task_complete", "last_agent_message": "Done."}}))
            })
            .unwrap();
        let p = peek(&h, Agent::Codex, "c1", now).unwrap();
        assert_eq!((p.state, p.said.as_deref()), ("your_turn", Some("Done.")));
        fs::remove_dir_all(&h).unwrap();
    }
}
