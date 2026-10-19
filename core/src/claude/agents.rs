//! What each Claude Code session is doing, step by step, and the permission
//! requests waiting for an answer, for Yoru's home in the app and the board.
//!
//! The steps come from the hooks ([`super::hooks`]): `PreToolUse` and
//! `PostToolUse` say which file was read or edited (with the lines added and
//! removed) and which command ran; `Notification` and `PermissionRequest`
//! that the session waits for you; `Stop` that its turn is over.
//!
//! A `PermissionRequest` hook is the only one that waits: it hands the
//! request to [`Agents::ask`], which holds it until someone answers in the
//! app or on the board ([`Agents::answer`]), or gives up so Claude Code asks
//! in its own terminal as usual.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Steps kept for each session, newest last.
const STEPS: usize = 8;
/// Sessions kept, the most recently active ones.
const SESSIONS: usize = 8;
/// A session quiet this long is dropped from the feed.
const FORGET: Duration = Duration::from_secs(2 * 3600);
/// Length of a step's text (a command, a file name) and of a task.
const TEXT: usize = 72;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// Between a prompt and the end of its turn.
    Work,
    /// It waits for a permission or an answer.
    Wait,
    /// The turn ended.
    Done,
    /// Started, nothing asked yet.
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Read,
    Edit,
    Write,
    Run,
    Search,
    Web,
    Agent,
    Ask,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub kind: StepKind,
    /// The tool, as Claude Code names it (`Edit`, `Bash`, `mcp__x__y`).
    pub tool: String,
    /// What it worked on: a file name, a command, a pattern.
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed: Option<u32>,
    /// `None` while it runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    /// Unix ms.
    pub at: u64,
    /// Pairs a `PostToolUse` with its `PreToolUse`.
    #[serde(skip)]
    use_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSession {
    pub id: String,
    /// The folder it runs in, as the person knows the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub state: AgentState,
    /// The start of the latest prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// What it waits for, while `state` is `wait`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting: Option<String>,
    pub steps: Vec<Step>,
    /// When `state` began, Unix ms.
    pub since: u64,
    pub updated: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Deny,
}

/// A permission request waiting for an answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Approval {
    /// Ours, to answer it by.
    pub id: String,
    pub session: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub tool: String,
    pub kind: StepKind,
    /// The command, the file, what the tool would do.
    pub text: String,
    /// Claude Code's own description of a command, when it gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Unix ms.
    pub asked: u64,
    /// When it stops waiting and Claude Code asks in the terminal, Unix ms.
    pub expires: u64,
}

/// What the app and the board are shown.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentsView {
    /// Most recently active first.
    pub sessions: Vec<AgentSession>,
    /// Oldest first.
    pub approvals: Vec<Approval>,
}

/// Told every time the view changes.
pub type AgentsListener = Arc<dyn Fn(AgentsView) + Send + Sync>;

struct Pending {
    approval: Approval,
    answer: Option<Option<Decision>>,
}

#[derive(Default)]
struct Inner {
    sessions: VecDeque<AgentSession>,
    pending: Vec<Pending>,
    next: u64,
}

/// The feed and the approvals. Shared between the bridge (which feeds it
/// from the hooks) and its frontend (which shows it and answers).
#[derive(Default)]
pub struct Agents {
    inner: Mutex<Inner>,
    answered: Condvar,
    listener: Mutex<Option<AgentsListener>>,
}

impl std::fmt::Debug for Agents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Agents").finish_non_exhaustive()
    }
}

impl Agents {
    pub fn view(&self) -> AgentsView {
        let mut inner = self.inner.lock().unwrap();
        inner.prune(SystemTime::now());
        inner.view()
    }

    pub fn set_listener(&self, listener: Option<AgentsListener>) {
        *self.listener.lock().unwrap() = listener;
    }

    /// A hook's JSON, as Claude Code sent it.
    pub fn on_hook(&self, json: &Value) {
        let changed = self.inner.lock().unwrap().on_hook(json, SystemTime::now());
        if changed {
            self.changed();
        }
    }

    /// Hold a `PermissionRequest` until it's answered, at most `wait`; `None`
    /// leaves it to Claude Code's own dialog.
    pub fn ask(&self, json: &Value, wait: Duration) -> Option<Decision> {
        let now = SystemTime::now();
        let id = {
            let mut inner = self.inner.lock().unwrap();
            inner.on_hook(json, now);
            inner.next += 1;
            let id = format!("p{}", inner.next);
            let tool = str_at(json, "tool_name").unwrap_or("?").to_string();
            let input = json.get("tool_input").cloned().unwrap_or(Value::Null);
            let step = describe(&tool, &input);
            inner.pending.push(Pending {
                approval: Approval {
                    id: id.clone(),
                    session: str_at(json, "session_id").unwrap_or_default().to_string(),
                    project: str_at(json, "cwd").and_then(project_name),
                    tool,
                    kind: step.kind,
                    text: step.text,
                    detail: input.get("description").and_then(Value::as_str).map(|d| clip(d, TEXT)),
                    asked: unix_ms(now),
                    expires: unix_ms(now + wait),
                },
                answer: None,
            });
            id
        };
        self.changed();
        let until = Instant::now() + wait;
        let mut inner = self.inner.lock().unwrap();
        let answer = loop {
            let at = inner.pending.iter().position(|p| p.approval.id == id);
            let Some(at) = at else { break None };
            if let Some(answer) = inner.pending[at].answer {
                inner.pending.remove(at);
                break answer;
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                inner.pending.remove(at);
                break None;
            }
            inner = self.answered.wait_timeout(inner, left).unwrap().0;
        };
        drop(inner);
        self.changed();
        answer
    }

    /// Ask the user to allow something Yoru's assistant is about to do (send
    /// a mail, create an event): the same card as Claude Code's requests, on
    /// the board and in the app. `None` when nobody answered within `wait`.
    pub fn confirm(&self, tool: &str, text: &str, detail: Option<&str>, wait: Duration) -> Option<Decision> {
        let now = SystemTime::now();
        let id = {
            let mut inner = self.inner.lock().unwrap();
            inner.next += 1;
            let id = format!("y{}", inner.next);
            inner.pending.push(Pending {
                approval: Approval {
                    id: id.clone(),
                    session: "yoru".into(),
                    project: Some("Yoru".into()),
                    tool: clip(tool, 40),
                    kind: StepKind::Other,
                    text: clip(text, TEXT),
                    detail: detail.map(|d| clip(d, 2000)),
                    asked: unix_ms(now),
                    expires: unix_ms(now + wait),
                },
                answer: None,
            });
            id
        };
        self.changed();
        let until = Instant::now() + wait;
        let mut inner = self.inner.lock().unwrap();
        let answer = loop {
            let Some(at) = inner.pending.iter().position(|p| p.approval.id == id) else { break None };
            if let Some(answer) = inner.pending[at].answer {
                inner.pending.remove(at);
                break answer;
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                inner.pending.remove(at);
                break None;
            }
            inner = self.answered.wait_timeout(inner, left).unwrap().0;
        };
        drop(inner);
        self.changed();
        answer
    }

    /// Answer approval `id`; false when it's no longer waiting.
    pub fn answer(&self, id: &str, decision: Decision) -> bool {
        let found = {
            let mut inner = self.inner.lock().unwrap();
            match inner.pending.iter_mut().find(|p| p.approval.id == id && p.answer.is_none()) {
                Some(p) => {
                    p.answer = Some(Some(decision));
                    true
                }
                None => false,
            }
        };
        self.answered.notify_all();
        found
    }

    fn changed(&self) {
        let listener = self.listener.lock().unwrap().clone();
        if let Some(listener) = listener {
            listener(self.view());
        }
    }
}

impl Inner {
    fn view(&self) -> AgentsView {
        AgentsView {
            sessions: self.sessions.iter().cloned().collect(),
            approvals: self.pending.iter().filter(|p| p.answer.is_none()).map(|p| p.approval.clone()).collect(),
        }
    }

    fn prune(&mut self, now: SystemTime) {
        let oldest = unix_ms(now).saturating_sub(FORGET.as_millis() as u64);
        self.sessions.retain(|s| s.updated >= oldest);
    }

    /// Fold one hook event into its session; false when it changed nothing.
    fn on_hook(&mut self, json: &Value, now: SystemTime) -> bool {
        let (Some(event), Some(id)) = (str_at(json, "hook_event_name"), str_at(json, "session_id")) else { return false };
        let now_ms = unix_ms(now);
        if event == "SessionEnd" {
            let before = self.sessions.len();
            self.sessions.retain(|s| s.id != id);
            self.settle(id);
            return self.sessions.len() != before;
        }
        let session = self.session(id, now_ms);
        if session.project.is_none() {
            session.project = str_at(json, "cwd").and_then(project_name);
        }
        session.updated = now_ms;
        let tool = str_at(json, "tool_name").unwrap_or_default();
        let input = json.get("tool_input").cloned().unwrap_or(Value::Null);
        let use_id = str_at(json, "tool_use_id").map(str::to_string);
        let mut settled = false;
        match event {
            "SessionStart" => {}
            "UserPromptSubmit" => {
                session.set_state(AgentState::Work, now_ms);
                session.task = str_at(json, "prompt").map(|p| clip(p.trim(), TEXT)).filter(|p| !p.is_empty());
                session.steps.clear();
                settled = true;
            }
            "PreToolUse" => {
                // A tool runs: whatever it waited for was answered.
                session.set_state(AgentState::Work, now_ms);
                settled = true;
                let step = Step { at: now_ms, use_id, ..describe(tool, &input) };
                session.push(step);
            }
            "PostToolUse" | "PostToolUseFailure" => {
                let ok = event == "PostToolUse";
                let counted = json.get("tool_response").and_then(patch_counts);
                let step = match session.steps.iter_mut().rev().find(|s| s.use_id.is_some() && s.use_id == use_id) {
                    Some(step) => step,
                    None => {
                        session.push(Step { at: now_ms, use_id, ..describe(tool, &input) });
                        session.steps.last_mut().expect("just pushed")
                    }
                };
                step.ok = Some(ok);
                if let Some((added, removed)) = counted {
                    (step.added, step.removed) = (Some(added), Some(removed));
                }
            }
            "Notification" => {
                let kind = str_at(json, "notification_type").unwrap_or_default();
                if !matches!(kind, "permission_prompt" | "elicitation_dialog" | "agent_needs_input") {
                    return false;
                }
                session.set_state(AgentState::Wait, now_ms);
                session.waiting = str_at(json, "message").map(|m| clip(m, TEXT)).or_else(|| session.waiting.take());
            }
            "PermissionRequest" => {
                session.set_state(AgentState::Wait, now_ms);
                let step = describe(tool, &input);
                session.waiting = Some(format!("{}: {}", step.tool, step.text));
            }
            "Stop" => {
                session.set_state(AgentState::Done, now_ms);
                settled = true;
            }
            _ => return false,
        }
        if settled {
            // It moved on without us: the request was answered in the terminal.
            self.settle(id);
        }
        self.sessions.make_contiguous().sort_by(|a, b| b.updated.cmp(&a.updated));
        self.sessions.truncate(SESSIONS);
        self.prune(now);
        true
    }

    fn session(&mut self, id: &str, now_ms: u64) -> &mut AgentSession {
        let at = match self.sessions.iter().position(|s| s.id == id) {
            Some(at) => at,
            None => {
                self.sessions.push_front(AgentSession {
                    id: id.to_string(),
                    project: None,
                    state: AgentState::Idle,
                    task: None,
                    waiting: None,
                    steps: Vec::new(),
                    since: now_ms,
                    updated: now_ms,
                });
                0
            }
        };
        &mut self.sessions[at]
    }

    /// Requests of `session` still waiting are over.
    fn settle(&mut self, session: &str) {
        for p in self.pending.iter_mut().filter(|p| p.approval.session == session && p.answer.is_none()) {
            p.answer = Some(None);
        }
    }
}

impl AgentSession {
    fn set_state(&mut self, state: AgentState, now_ms: u64) {
        if self.state != state {
            self.state = state;
            self.since = now_ms;
        }
        if state != AgentState::Wait {
            self.waiting = None;
        }
    }

    fn push(&mut self, step: Step) {
        self.steps.push(step);
        if self.steps.len() > STEPS {
            self.steps.remove(0);
        }
    }
}

/// A step for `tool` with `input`, as the feed shows it.
fn describe(tool: &str, input: &Value) -> Step {
    let s = |k: &str| input.get(k).and_then(Value::as_str);
    let file = |k: &str| s(k).map(file_name).unwrap_or_default();
    let (kind, text, added, removed) = match tool {
        "Read" => (StepKind::Read, file("file_path"), None, None),
        "Edit" => {
            let (added, removed) = (lines(s("new_string")), lines(s("old_string")));
            (StepKind::Edit, file("file_path"), Some(added), Some(removed))
        }
        "MultiEdit" => {
            let edits = input.get("edits").and_then(Value::as_array).into_iter().flatten();
            let (added, removed) = edits.fold((0, 0), |(a, r), e| {
                (a + lines(e.get("new_string").and_then(Value::as_str)), r + lines(e.get("old_string").and_then(Value::as_str)))
            });
            (StepKind::Edit, file("file_path"), Some(added), Some(removed))
        }
        "Write" => (StepKind::Write, file("file_path"), Some(lines(s("content"))), None),
        "NotebookEdit" => (StepKind::Edit, file("notebook_path"), None, None),
        "Bash" => (StepKind::Run, s("command").map(|c| c.lines().next().unwrap_or_default().trim().to_string()).unwrap_or_default(), None, None),
        "Grep" | "Glob" => (StepKind::Search, s("pattern").unwrap_or_default().to_string(), None, None),
        "WebFetch" => (StepKind::Web, s("url").map(host_of).unwrap_or_default(), None, None),
        "WebSearch" => (StepKind::Web, s("query").unwrap_or_default().to_string(), None, None),
        "Task" | "Agent" => (StepKind::Agent, s("description").unwrap_or_default().to_string(), None, None),
        "AskUserQuestion" => {
            let question = input.pointer("/questions/0/question").and_then(Value::as_str);
            (StepKind::Ask, question.unwrap_or_default().to_string(), None, None)
        }
        other => {
            // `mcp__server__tool` reads better as `server · tool`.
            let text = other.strip_prefix("mcp__").map(|rest| rest.replacen("__", " · ", 1)).unwrap_or_default();
            (StepKind::Other, text, None, None)
        }
    };
    Step { kind, tool: tool.to_string(), text: clip(&text, TEXT), added, removed, ok: None, at: 0, use_id: None }
}

/// Lines added and removed, from an edit's `structuredPatch` when the tool's
/// response has one. Not documented by Claude Code: read leniently.
fn patch_counts(response: &Value) -> Option<(u32, u32)> {
    let hunks = response.get("structuredPatch")?.as_array()?;
    let lines = hunks.iter().filter_map(|h| h.get("lines")?.as_array()).flatten().filter_map(Value::as_str);
    Some(lines.fold((0, 0), |(a, r), l| match l.as_bytes().first() {
        Some(b'+') => (a + 1, r),
        Some(b'-') => (a, r + 1),
        _ => (a, r),
    }))
}

fn lines(text: Option<&str>) -> u32 {
    text.map_or(0, |t| t.lines().count() as u32)
}

fn file_name(path: &str) -> String {
    Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split('/').next().unwrap_or(rest).to_string()
}

fn project_name(cwd: &str) -> Option<String> {
    Path::new(cwd).file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty())
}

/// At most `max` characters, with an ellipsis when cut.
fn clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", text[..at].trim_end()),
        None => text.to_string(),
    }
}

fn str_at<'a>(json: &'a Value, key: &str) -> Option<&'a str> {
    json.get(key).and_then(Value::as_str)
}

fn unix_ms(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use std::thread;

    use serde_json::json;

    use super::*;

    fn hook(event: &str, extra: Value) -> Value {
        let mut v = json!({"hook_event_name": event, "session_id": "s1", "cwd": "/home/me/yoru"});
        v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        v
    }

    #[test]
    fn a_turn_step_by_step() {
        let agents = Agents::default();
        agents.on_hook(&hook("UserPromptSubmit", json!({"prompt": "Fix the menu swipe"})));
        agents.on_hook(&hook("PreToolUse", json!({"tool_name": "Read", "tool_use_id": "a", "tool_input": {"file_path": "/x/main/menu.c"}})));
        agents.on_hook(&hook("PostToolUse", json!({"tool_name": "Read", "tool_use_id": "a", "tool_input": {"file_path": "/x/main/menu.c"}})));
        let edit = json!({"tool_name": "Edit", "tool_use_id": "b", "tool_input": {"file_path": "/x/main/menu.c", "old_string": "a\nb", "new_string": "a\nb\nc\nd"}});
        agents.on_hook(&hook("PreToolUse", edit.clone()));
        agents.on_hook(&hook("PostToolUse", edit));
        agents.on_hook(&hook("PreToolUse", json!({"tool_name": "Bash", "tool_use_id": "c", "tool_input": {"command": "idf.py build\necho done"}})));
        agents.on_hook(&hook("PostToolUseFailure", json!({"tool_name": "Bash", "tool_use_id": "c", "tool_input": {}})));

        let view = agents.view();
        let s = &view.sessions[0];
        assert_eq!(s.project.as_deref(), Some("yoru"));
        assert_eq!(s.task.as_deref(), Some("Fix the menu swipe"));
        assert_eq!(s.state, AgentState::Work);
        assert_eq!(s.steps.len(), 3);
        assert_eq!((s.steps[0].kind, s.steps[0].text.as_str(), s.steps[0].ok), (StepKind::Read, "menu.c", Some(true)));
        assert_eq!((s.steps[1].added, s.steps[1].removed), (Some(4), Some(2)));
        assert_eq!((s.steps[2].text.as_str(), s.steps[2].ok), ("idf.py build", Some(false)));

        agents.on_hook(&hook("Notification", json!({"notification_type": "permission_prompt", "message": "Claude needs your permission to use Bash"})));
        assert_eq!(agents.view().sessions[0].state, AgentState::Wait);
        agents.on_hook(&hook("Stop", json!({})));
        let s = &agents.view().sessions[0];
        assert_eq!((s.state, s.waiting.as_ref()), (AgentState::Done, None));
        agents.on_hook(&hook("SessionEnd", json!({"reason": "other"})));
        assert!(agents.view().sessions.is_empty());
    }

    #[test]
    fn patch_counts_win_over_the_input() {
        let response = json!({"structuredPatch": [{"lines": [" a", "-b", "+c", "+d"]}]});
        assert_eq!(patch_counts(&response), Some((2, 1)));
        assert_eq!(patch_counts(&json!({"filePath": "x"})), None);
        assert_eq!(describe("mcp__yoru__react", &json!({})).text, "yoru · react");
        assert_eq!(describe("WebFetch", &json!({"url": "https://docs.rs/serde/latest"})).text, "docs.rs");
        assert_eq!(clip("abcdef", 3), "abc…");
    }

    #[test]
    fn an_approval_waits_for_its_answer() {
        let agents = Arc::new(Agents::default());
        let request = hook("PermissionRequest", json!({"tool_name": "Bash", "tool_input": {"command": "rm -rf build", "description": "Clean the build"}}));
        let asking = {
            let agents = agents.clone();
            thread::spawn(move || agents.ask(&request, Duration::from_secs(5)))
        };
        let approval = loop {
            if let Some(a) = agents.view().approvals.first().cloned() {
                break a;
            }
            thread::sleep(Duration::from_millis(5));
        };
        assert_eq!((approval.text.as_str(), approval.detail.as_deref()), ("rm -rf build", Some("Clean the build")));
        assert_eq!(agents.view().sessions[0].state, AgentState::Wait);
        assert!(agents.answer(&approval.id, Decision::Deny));
        assert_eq!(asking.join().unwrap(), Some(Decision::Deny));
        assert!(agents.view().approvals.is_empty());
        assert!(!agents.answer(&approval.id, Decision::Allow));
    }

    #[test]
    fn an_approval_gives_up() {
        let agents = Agents::default();
        let request = hook("PermissionRequest", json!({"tool_name": "Write", "tool_input": {"file_path": "/a/b.txt", "content": "x"}}));
        assert_eq!(agents.ask(&request, Duration::from_millis(30)), None);
        assert!(agents.view().approvals.is_empty());
    }
}
