//! The user's coding agents on this computer, Claude Code and Codex, as one
//! of Yoru's tools (`code`): the conversations the user had with them, and a
//! prompt said to Yoru sent to one of them in the background. It can
//! continue a conversation, start a new one in a project the user already
//! has, or start one in a fresh folder outside any project. The answer comes
//! back in the inbox when the agent is done.
//!
//! Nothing here talks to the agents' apps: their command lines are run as
//! the user would run them (`claude -p`, `codex exec`), and the
//! conversations are read from the files they keep (`~/.claude/projects`,
//! `~/.codex/sessions`). Claude Code works in its `auto` permission mode,
//! where risky actions are checked; Codex in its `workspace-write` sandbox.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{Value, json};

use super::inbox::{Inbox, Urgency};

/// Session files read to list the conversations: the newest of each agent.
const NEWEST: usize = 60;
/// Conversations a listing returns.
const LISTED: usize = 12;
/// Runs remembered, finished or not.
const KEPT_RUNS: usize = 20;
/// A run is stopped after this long.
const RUN_LIMIT: Duration = Duration::from_secs(60 * 60);
/// What the inbox gets of an agent's answer.
const ANSWER_CHARS: usize = 900;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Claude,
    Codex,
}

impl Agent {
    fn parse(s: &str) -> Option<Agent> {
        match s.trim().to_lowercase().as_str() {
            "claude" | "claude code" | "claude-code" => Some(Agent::Claude),
            "codex" | "chatgpt" | "openai" | "gpt" => Some(Agent::Codex),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
        }
    }
}

/// A conversation the user had with an agent.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Session {
    pub agent: Agent,
    pub id: String,
    /// Its title, or its first message when it has none.
    pub title: String,
    /// The folder it works in.
    pub project: String,
    /// Unix ms of its last change.
    pub updated: u64,
}

/// A prompt sent to an agent.
#[derive(Debug, Clone, Serialize)]
pub struct Run {
    pub id: u64,
    pub agent: Agent,
    pub project: String,
    /// The conversation: the one continued, or the new one once the agent names it.
    pub session: Option<String>,
    pub prompt: String,
    /// `running`, `done`, `failed`, `stopped`.
    pub state: &'static str,
    /// The agent's last answer, or what went wrong.
    pub result: String,
    pub started: u64,
}

/// The runs, and where their answers go.
pub struct Coding {
    inbox: Arc<Inbox>,
    runs: Mutex<Vec<Run>>,
    /// The running agents' processes, to stop them.
    children: Mutex<HashMap<u64, Arc<Mutex<Child>>>>,
    next: Mutex<u64>,
}

impl std::fmt::Debug for Coding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Coding").finish_non_exhaustive()
    }
}

impl Coding {
    pub fn new(inbox: Arc<Inbox>) -> Arc<Self> {
        Arc::new(Self { inbox, runs: Mutex::default(), children: Mutex::default(), next: Mutex::new(0) })
    }

    /// The model's tool.
    pub fn tool() -> Value {
        json!({
            "name": "code",
            "description": "The user's coding agents on this computer, Claude Code (claude) and Codex (codex). \
sessions: their conversations, newest first, each with its id, title and project folder; query keeps those whose title, folder or first \
message has its words. send: give an agent a prompt, run in the background: to continue a conversation pass its session id (find it with \
sessions first); for a new conversation in one of the user's projects pass project (its name or folder); with neither, a new conversation \
in a fresh folder outside any project. The prompt goes in the user's words, whole, in their language: don't summarize or add to it. \
The agent works on its own (it may take minutes): tell the user it started; its answer comes to the inbox when it's done. \
runs: what was sent and how it went (an answer you can read out). stop: stop a running one by its run id.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["sessions", "send", "runs", "stop"]},
                    "agent": {"type": "string", "enum": ["claude", "codex"], "description": "Default claude; for sessions, both when left out"},
                    "query": {"type": "string", "description": "sessions: words of the title, project or first message"},
                    "session": {"type": "string", "description": "send: the id of the conversation to continue"},
                    "project": {"type": "string", "description": "send: a project's name or folder, for a new conversation in it"},
                    "prompt": {"type": "string", "description": "send: what to tell the agent"},
                    "run": {"type": "integer", "description": "stop: the run's id"},
                },
                "required": ["action"],
            },
        })
    }

    /// Run the `code` tool.
    pub fn call_tool(self: &Arc<Self>, input: &Value) -> Result<String, String> {
        let text = |k: &str| input[k].as_str().map(str::trim).filter(|s| !s.is_empty());
        let agent = text("agent").and_then(Agent::parse);
        let home = crate::paths::home_dir().ok_or("no home folder")?;
        match input["action"].as_str().unwrap_or("sessions") {
            "sessions" => {
                let all = sessions(&home, agent);
                let found = matching(&all, text("query").unwrap_or_default());
                if found.is_empty() {
                    return Ok(if all.is_empty() { "no conversations found".into() } else { "no conversation matches; try fewer words".into() });
                }
                Ok(json!(found.iter().map(|s| json!({"agent": s.agent, "id": s.id, "title": s.title, "project": s.project, "updated": local(s.updated)})).collect::<Vec<_>>()).to_string())
            }
            "send" => {
                let prompt = text("prompt").ok_or("prompt is needed")?;
                let (agent, project, session) = if let Some(id) = text("session") {
                    let all = sessions(&home, None);
                    let s = all.iter().find(|s| s.id == id && agent.is_none_or(|a| a == s.agent)).ok_or(format!("no conversation {id}: list them with sessions"))?;
                    (s.agent, PathBuf::from(&s.project), Some(s.id.clone()))
                } else if let Some(name) = text("project") {
                    (agent.unwrap_or(Agent::Claude), project_folder(&home, name)?, None)
                } else {
                    (agent.unwrap_or(Agent::Claude), fresh_folder(&home, prompt)?, None)
                };
                let run = self.start(agent, &project, session.as_deref(), prompt)?;
                Ok(format!("sent to {} (run {run}) in {}: it works on its own; its answer comes to the inbox", agent.name(), project.display()))
            }
            "runs" => {
                let runs = self.runs.lock().unwrap().clone();
                if runs.is_empty() {
                    return Ok("nothing sent yet".into());
                }
                Ok(json!(runs.iter().rev().map(|r| json!({"run": r.id, "agent": r.agent, "project": r.project, "session": r.session, "prompt": r.prompt, "state": r.state, "result": clip(&r.result, ANSWER_CHARS), "started": local(r.started)})).collect::<Vec<_>>()).to_string())
            }
            "stop" => {
                let id = input["run"].as_u64().ok_or("run is needed")?;
                let child = self.children.lock().unwrap().get(&id).cloned().ok_or(format!("run {id} isn't running"))?;
                let _ = child.lock().unwrap().kill();
                self.finish(id, "stopped", "stopped by the user".into(), None);
                Ok(format!("stopped run {id}"))
            }
            other => Err(format!("unknown action {other}")),
        }
    }

    /// Send `prompt` to `agent` in `project` (continuing `session`), in the
    /// background. The run's id.
    fn start(self: &Arc<Self>, agent: Agent, project: &Path, session: Option<&str>, prompt: &str) -> Result<u64, String> {
        let program = program(agent).ok_or(match agent {
            Agent::Claude => "Claude Code isn't installed as a command (claude)",
            Agent::Codex => "Codex isn't installed (its command, or the ChatGPT app)",
        })?;
        let out = std::env::temp_dir().join(format!("yoru-codex-{}-{}.txt", std::process::id(), crate::assistant::inbox::now_ms()));
        let mut command = Command::new(&program);
        command.args(arguments(agent, session, prompt, &out)).current_dir(project).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Some(path) = login_path() {
            // The app started from the Finder has a bare PATH; the agent's
            // commands (cargo, npm, git) need the user's.
            command.env("PATH", path);
        }
        let child = command.spawn().map_err(|e| format!("couldn't start {}: {e}", agent.name()))?;
        let id = {
            let mut next = self.next.lock().unwrap();
            *next += 1;
            *next
        };
        let run = Run {
            id,
            agent,
            project: project.display().to_string(),
            session: session.map(str::to_string),
            prompt: prompt.to_string(),
            state: "running",
            result: String::new(),
            started: crate::assistant::inbox::now_ms(),
        };
        {
            let mut runs = self.runs.lock().unwrap();
            runs.push(run);
            let drop_n = runs.len().saturating_sub(KEPT_RUNS);
            runs.drain(..drop_n);
        }
        let child = Arc::new(Mutex::new(child));
        self.children.lock().unwrap().insert(id, child.clone());
        crate::hostlog::line("code", &format!("run {id}: {} in {} {}", agent.name(), project.display(), session.map_or("(new conversation)".into(), |s| format!("(continuing {s})"))));
        let me = self.clone();
        thread::Builder::new()
            .name("yoru-code".into())
            .spawn(move || me.wait(id, agent, child, &out))
            .map_err(|e| e.to_string())?;
        Ok(id)
    }

    /// Wait for run `id` to end, then say how it went.
    fn wait(&self, id: u64, agent: Agent, child: Arc<Mutex<Child>>, out: &Path) {
        let (stdout, stderr) = {
            let mut c = child.lock().unwrap();
            (c.stdout.take(), c.stderr.take())
        };
        // Read on their own threads: a full pipe would stop the agent.
        let errors = thread::spawn(move || {
            let mut s = String::new();
            if let Some(mut e) = stderr {
                let _ = e.read_to_string(&mut s);
            }
            s
        });
        let watch = {
            let child = child.clone();
            let started = Instant::now();
            thread::spawn(move || {
                loop {
                    thread::sleep(Duration::from_secs(2));
                    let mut c = child.lock().unwrap();
                    if c.try_wait().ok().flatten().is_some() {
                        return;
                    }
                    if started.elapsed() > RUN_LIMIT {
                        let _ = c.kill();
                        return;
                    }
                }
            })
        };
        let mut lines = String::new();
        if let Some(mut o) = stdout {
            let _ = o.read_to_string(&mut lines);
        }
        let _ = watch.join();
        let status = child.lock().unwrap().wait().ok();
        let errors = errors.join().unwrap_or_default();
        self.children.lock().unwrap().remove(&id);
        if self.runs.lock().unwrap().iter().any(|r| r.id == id && r.state == "stopped") {
            return;
        }
        let last = fs::read_to_string(out).ok();
        let _ = fs::remove_file(out);
        let outcome = match agent {
            Agent::Claude => claude_outcome(&lines),
            Agent::Codex => codex_outcome(&lines, last.as_deref()),
        };
        let (state, result, session) = match outcome {
            Some(o) if o.ok => ("done", o.answer, o.session),
            Some(o) => ("failed", login_hint(agent, &o.answer), o.session),
            None => {
                let why = errors.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("it stopped without an answer").to_string();
                let code = status.and_then(|s| s.code()).map_or(String::new(), |c| format!(" (exit {c})"));
                ("failed", login_hint(agent, &format!("{why}{code}")), None)
            }
        };
        self.finish(id, state, result, session);
    }

    /// Mark run `id` ended, and tell the user in the inbox.
    fn finish(&self, id: u64, state: &'static str, result: String, session: Option<String>) {
        let run = {
            let mut runs = self.runs.lock().unwrap();
            let Some(run) = runs.iter_mut().find(|r| r.id == id) else { return };
            if run.state != "running" {
                return;
            }
            run.state = state;
            run.result = result;
            if run.session.is_none() {
                run.session = session;
            }
            run.clone()
        };
        crate::hostlog::line("code", &format!("run {id}: {state}"));
        let place = Path::new(&run.project).file_name().map_or(run.project.clone(), |n| n.to_string_lossy().into_owned());
        let title = match state {
            "done" => format!("{place}: done"),
            "stopped" => format!("{place}: stopped"),
            _ => format!("{place}: it didn't work"),
        };
        let text = format!("{}\n\nAsked: {}", clip(&run.result, ANSWER_CHARS), clip(&run.prompt, 200));
        self.inbox.post(run.agent.name(), &title, &text, Urgency::Normal, None, Some("code".into()));
    }
}

/// How an agent's run ended, read from what it printed.
struct Outcome {
    ok: bool,
    answer: String,
    session: Option<String>,
}

/// `claude -p --output-format json`: one result object.
fn claude_outcome(printed: &str) -> Option<Outcome> {
    let result: Value = printed.lines().rev().find_map(|l| serde_json::from_str::<Value>(l.trim()).ok().filter(|v| v["type"] == "result"))?;
    let mut answer = result["result"].as_str().unwrap_or_default().trim().to_string();
    let denied: Vec<&str> = result["permission_denials"].as_array().into_iter().flatten().filter_map(|d| d["tool_name"].as_str()).collect();
    if !denied.is_empty() {
        answer.push_str(&format!("\n\nNot allowed to run: {}.", denied.join(", ")));
    }
    Some(Outcome { ok: result["is_error"] != true, answer, session: result["session_id"].as_str().map(str::to_string) })
}

/// `codex exec --json`: events a line, the last message in `last` (`-o`).
fn codex_outcome(printed: &str, last: Option<&str>) -> Option<Outcome> {
    let events: Vec<Value> = printed.lines().filter_map(|l| serde_json::from_str(l.trim()).ok()).collect();
    let session = events.iter().find(|e| e["type"] == "thread.started").and_then(|e| e["thread_id"].as_str()).map(str::to_string);
    let said = events.iter().rev().find(|e| e["item"]["type"] == "agent_message").and_then(|e| e["item"]["text"].as_str());
    let failed = events.iter().rev().find(|e| e["type"] == "turn.failed" || e["type"] == "error");
    let answer = last.map(str::trim).filter(|t| !t.is_empty()).or(said);
    match (answer, failed) {
        (Some(a), None) => Some(Outcome { ok: true, answer: a.to_string(), session }),
        (_, Some(f)) => {
            let why = f["error"]["message"].as_str().or(f["message"].as_str()).unwrap_or("the turn failed");
            Some(Outcome { ok: false, answer: why.to_string(), session })
        }
        (None, None) => None,
    }
}

/// What went wrong, with how to fix a login that ran out.
fn login_hint(agent: Agent, why: &str) -> String {
    let lower = why.to_lowercase();
    if lower.contains("authenticat") || lower.contains("login") || lower.contains("logged in") || lower.contains("oauth") {
        let how = match agent {
            Agent::Claude => "run claude in the Terminal and log in (/login)",
            Agent::Codex => "run codex login in the Terminal",
        };
        format!("{why}\n\n{} needs a login: {how}.", agent.name())
    } else {
        why.to_string()
    }
}

/// The command line for `agent`.
fn arguments(agent: Agent, session: Option<&str>, prompt: &str, out: &Path) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    match agent {
        Agent::Claude => {
            args.extend(["-p", "--output-format", "json", "--permission-mode", "auto"].map(String::from));
            if let Some(s) = session {
                args.extend(["--resume".into(), s.into()]);
            }
            args.extend(["--".into(), prompt.into()]);
        }
        Agent::Codex => {
            args.push("exec".into());
            if session.is_some() {
                args.push("resume".into());
            }
            args.extend(["--skip-git-repo-check", "--json", "-c", "sandbox_mode=\"workspace-write\"", "-o"].map(String::from));
            args.push(out.display().to_string());
            if let Some(s) = session {
                args.push(s.into());
            }
            args.extend(["--".into(), prompt.into()]);
        }
    }
    args
}

/// Where `agent`'s command is: on the user's PATH, in the usual places, or
/// (Codex) inside the ChatGPT app.
fn program(agent: Agent) -> Option<PathBuf> {
    let home = crate::paths::home_dir().unwrap_or_default();
    let name = match agent {
        Agent::Claude => "claude",
        Agent::Codex => "codex",
    };
    let mut places: Vec<PathBuf> = login_path().unwrap_or_default().split(':').map(|d| Path::new(d).join(name)).collect();
    places.extend([home.join(".local/bin").join(name), PathBuf::from("/opt/homebrew/bin").join(name), PathBuf::from("/usr/local/bin").join(name)]);
    match agent {
        Agent::Claude => places.push(home.join(".claude/local/claude")),
        Agent::Codex => places.extend([
            PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex"),
            PathBuf::from("/Applications/Codex.app/Contents/Resources/codex-cli/bin/codex"),
        ]),
    }
    places.into_iter().find(|p| p.is_file())
}

/// The PATH the user's shell has, asked once.
fn login_path() -> Option<String> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        let out = Command::new(shell).args(["-ilc", "printf '<<%s>>' \"$PATH\""]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let path = text.rsplit_once("<<")?.1.split_once(">>")?.0.to_string();
        (!path.is_empty()).then_some(path)
    })
    .clone()
}

/// Every conversation of `agent` (both when `None`), newest first.
pub fn sessions(home: &Path, agent: Option<Agent>) -> Vec<Session> {
    let mut all = Vec::new();
    if agent.is_none_or(|a| a == Agent::Claude) {
        all.extend(claude_sessions(home));
    }
    if agent.is_none_or(|a| a == Agent::Codex) {
        all.extend(codex_sessions(home));
    }
    // A conversation in a scratch or temporary folder isn't the user's work.
    all.retain(|s| !scratch(&s.project));
    all.sort_by(|a, b| b.updated.cmp(&a.updated));
    all
}

fn scratch(path: &str) -> bool {
    ["/Application Support/Claude/", "/private/tmp/"].iter().any(|m| path.contains(m)) || path.starts_with("/tmp/")
}

/// Those of `all` whose title, folder or id have the words of `query`,
/// most words first, then newest.
fn matching<'a>(all: &'a [Session], query: &str) -> Vec<&'a Session> {
    let words: Vec<String> = query.to_lowercase().split_whitespace().filter(|w| w.chars().count() > 1).map(str::to_string).collect();
    if words.is_empty() {
        return all.iter().take(LISTED).collect();
    }
    let mut scored: Vec<(usize, &Session)> = all
        .iter()
        .map(|s| {
            let hay = format!("{} {} {}", s.title, s.project, s.id).to_lowercase();
            (words.iter().filter(|w| hay.contains(w.as_str())).count(), s)
        })
        .filter(|(n, _)| *n > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.updated.cmp(&a.1.updated)));
    scored.into_iter().take(LISTED).map(|(_, s)| s).collect()
}

/// The newest `.jsonl` files under `dir` (recursively), with their times.
fn newest_files(dir: &Path, n: usize) -> Vec<(PathBuf, u64)> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "jsonl") {
                let at = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as u64);
                files.push((p, at));
            }
        }
    }
    files.sort_by(|a, b| b.1.cmp(&a.1));
    files.truncate(n);
    files
}

/// A first message worth showing: not the app's own context blocks.
fn first_words(text: &str) -> Option<String> {
    let t = text.trim();
    (!t.is_empty() && !t.starts_with('<') && !t.starts_with("Caveat:")).then(|| clip(&t.replace('\n', " "), 120))
}

/// Claude Code's conversations: `~/.claude/projects/<folder>/<id>.jsonl`.
fn claude_sessions(home: &Path) -> Vec<Session> {
    newest_files(&home.join(".claude/projects"), NEWEST)
        .into_iter()
        // A subagent's work is kept beside its conversation, not one of its own.
        .filter(|(path, _)| !path.components().any(|c| c.as_os_str() == "subagents") && !path.file_stem().is_some_and(|s| s.to_string_lossy().starts_with("agent-")))
        .filter_map(|(path, updated)| {
            let id = path.file_stem()?.to_string_lossy().into_owned();
            let (mut title, mut first, mut cwd) = (None, None, None);
            for line in BufReader::new(fs::File::open(&path).ok()?).lines().map_while(Result::ok) {
                // Parsed only when it may hold what is looked for: the files are long.
                let titled = line.contains("\"custom-title\"") || line.contains("\"ai-title\"");
                let wanted = titled || (cwd.is_none() && line.contains("\"cwd\"")) || (first.is_none() && line.contains("\"type\":\"user\""));
                if !wanted {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                if titled {
                    if let Some(t) = v["customTitle"].as_str().or(v["aiTitle"].as_str()).or(v["title"].as_str()) {
                        title = Some(t.to_string());
                    }
                }
                if cwd.is_none() {
                    cwd = v["cwd"].as_str().map(str::to_string);
                }
                if first.is_none() && v["type"] == "user" {
                    first = v["message"]["content"].as_str().and_then(first_words);
                }
            }
            let project = cwd?;
            Some(Session { agent: Agent::Claude, id, title: title.or(first).unwrap_or_else(|| "(untitled)".into()), project, updated })
        })
        .collect()
}

/// Codex's conversations: `~/.codex/sessions/<y>/<m>/<d>/rollout-<time>-<id>.jsonl`,
/// titled by `~/.codex/session_index.jsonl`.
fn codex_sessions(home: &Path) -> Vec<Session> {
    let names: HashMap<String, String> = fs::read_to_string(home.join(".codex/session_index.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| Some((v["id"].as_str()?.to_string(), v["thread_name"].as_str()?.to_string())))
        .collect();
    newest_files(&home.join(".codex/sessions"), NEWEST)
        .into_iter()
        .filter_map(|(path, updated)| {
            let (mut id, mut cwd, mut first) = (None, None, None);
            for line in BufReader::new(fs::File::open(&path).ok()?).lines().map_while(Result::ok) {
                let meta = id.is_none() && line.contains("\"session_meta\"");
                // The app writes the user's words as a user_message event, `codex exec` only as a message.
                let said = first.is_none() && (line.contains("\"user_message\"") || line.contains("\"role\":\"user\""));
                if !meta && !said {
                    if id.is_some() && (first.is_some() || names.contains_key(id.as_deref().unwrap_or_default())) {
                        break;
                    }
                    continue;
                }
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                if meta {
                    id = v["payload"]["id"].as_str().map(str::to_string);
                    cwd = v["payload"]["cwd"].as_str().map(str::to_string);
                }
                if said && v["payload"]["type"] == "user_message" {
                    first = v["payload"]["message"].as_str().and_then(first_words);
                } else if said && v["payload"]["role"] == "user" {
                    first = v["payload"]["content"].as_array().into_iter().flatten().filter_map(|c| c["text"].as_str()).find_map(first_words);
                }
            }
            let id = id?;
            let title = names.get(&id).cloned().or(first).unwrap_or_else(|| "(untitled)".into());
            Some(Session { agent: Agent::Codex, id, title, project: cwd?, updated })
        })
        .collect()
}

/// The folder of project `name`: a folder itself, or the one of the user's
/// conversations whose last part is (or has) `name`.
fn project_folder(home: &Path, name: &str) -> Result<PathBuf, String> {
    let given = if let Some(rest) = name.strip_prefix("~/") { home.join(rest) } else { PathBuf::from(name) };
    if given.is_absolute() && given.is_dir() {
        return Ok(given);
    }
    let mut known: Vec<String> = sessions(home, None).into_iter().map(|s| s.project).filter(|p| !temporary(p)).collect();
    // Newest first, each once.
    let mut seen = std::collections::HashSet::new();
    known.retain(|p| seen.insert(p.clone()));
    let simple = |s: &str| s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect::<String>();
    let want = simple(name);
    if want.is_empty() {
        return Err("project is empty".into());
    }
    let last = |p: &str| simple(&Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    known
        .iter()
        .find(|p| last(p) == want)
        .or_else(|| known.iter().find(|p| last(p).contains(&want)))
        .or_else(|| known.iter().find(|p| simple(p).contains(&want)))
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .ok_or_else(|| {
            let names: Vec<String> = known.iter().take(15).filter_map(|p| Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned())).collect();
            format!("no project called {name}; the user's projects: {}", names.join(", "))
        })
}

/// A folder made for one conversation, not a project: a worktree, a
/// scratch space, a temporary folder.
fn temporary(path: &str) -> bool {
    scratch(path) || ["/.claude/worktrees/", "/Yoru Code/"].iter().any(|m| path.contains(m))
}

/// A new folder for a conversation outside any project:
/// `~/Documents/Yoru Code/<date>/<a few words of the prompt>`.
fn fresh_folder(home: &Path, prompt: &str) -> Result<PathBuf, String> {
    let slug: String = prompt
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(5)
        .collect::<Vec<_>>()
        .join("-");
    let base = home.join("Documents/Yoru Code").join(chrono::Local::now().format("%Y-%m-%d").to_string());
    let slug = if slug.is_empty() { "chat".to_string() } else { slug };
    let mut dir = base.join(&slug);
    for n in 2.. {
        if !dir.exists() {
            break;
        }
        dir = base.join(format!("{slug}-{n}"));
    }
    fs::create_dir_all(&dir).map_err(|e| format!("couldn't make {}: {e}", dir.display()))?;
    Ok(dir)
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { format!("{}…", s.chars().take(n).collect::<String>()) }
}

fn local(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64).map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yoru-code-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, lines: &[Value]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, lines.iter().map(|l| l.to_string() + "\n").collect::<String>()).unwrap();
    }

    #[test]
    fn reads_both_agents_conversations() {
        let h = home("read");
        let maps = h.join("Code/RomeMaps");
        fs::create_dir_all(&maps).unwrap();
        write(
            &h.join(".claude/projects/-x-RomeMaps/aaaa-1.jsonl"),
            &[
                json!({"type": "queue-operation"}),
                json!({"type": "user", "cwd": maps, "message": {"role": "user", "content": "<command-name>/clear</command-name>"}}),
                json!({"type": "user", "cwd": maps, "message": {"role": "user", "content": "fix the zoom on the map"}}),
                json!({"type": "custom-title", "customTitle": "Map zoom"}),
            ],
        );
        write(
            &h.join(".claude/projects/-x-RomeMaps/bbbb-2.jsonl"),
            &[json!({"type": "user", "cwd": maps, "message": {"role": "user", "content": "add a dark theme"}})],
        );
        write(
            &h.join(".codex/sessions/2026/11/12/rollout-2026-11-12T10-00-00-cccc-3.jsonl"),
            &[
                json!({"type": "session_meta", "payload": {"id": "cccc-3", "cwd": "/Users/x/Sites/pub"}}),
                json!({"type": "event_msg", "payload": {"type": "user_message", "message": "make the menu bigger"}}),
            ],
        );
        fs::write(h.join(".codex/session_index.jsonl"), "{\"id\":\"cccc-3\",\"thread_name\":\"Bigger menu\"}\n").unwrap();

        let all = sessions(&h, None);
        let titles: Vec<&str> = all.iter().map(|s| s.title.as_str()).collect();
        assert!(titles.contains(&"Map zoom") && titles.contains(&"add a dark theme") && titles.contains(&"Bigger menu"), "{titles:?}");
        let codex = all.iter().find(|s| s.agent == Agent::Codex).unwrap();
        assert_eq!((codex.id.as_str(), codex.project.as_str()), ("cccc-3", "/Users/x/Sites/pub"));
        assert_eq!(matching(&all, "zoom")[0].id, "aaaa-1");
        assert!(matching(&all, "spaceship rocket").is_empty());

        // A project by its name, whatever the case and the spaces.
        assert_eq!(project_folder(&h, "rome maps").unwrap(), maps);
        assert!(project_folder(&h, "Elsewhere").unwrap_err().contains("RomeMaps"));
        fs::remove_dir_all(&h).unwrap();
    }

    #[test]
    fn a_fresh_folder_for_each_new_conversation() {
        let h = home("fresh");
        let a = fresh_folder(&h, "Scrivi uno script che rinomina le foto!").unwrap();
        let b = fresh_folder(&h, "Scrivi uno script che rinomina le foto!").unwrap();
        assert!(a.ends_with("scrivi-uno-script-che-rinomina") && a.is_dir(), "{a:?}");
        assert!(b.to_string_lossy().ends_with("-2") && a != b);
        assert!(temporary(&a.to_string_lossy()));
        fs::remove_dir_all(&h).unwrap();
    }

    #[test]
    fn command_lines() {
        let out = Path::new("/tmp/o.txt");
        assert_eq!(arguments(Agent::Claude, Some("s1"), "go on", out), ["-p", "--output-format", "json", "--permission-mode", "auto", "--resume", "s1", "--", "go on"]);
        let codex = arguments(Agent::Codex, Some("t1"), "go on", out);
        assert_eq!(&codex[..2], ["exec", "resume"]);
        assert_eq!(&codex[codex.len() - 3..], ["t1", "--", "go on"]);
        assert_eq!(arguments(Agent::Codex, None, "hi", out)[1], "--skip-git-repo-check");
    }

    #[test]
    fn reads_how_a_run_ended() {
        let ok = claude_outcome(r#"{"type":"result","is_error":false,"result":"Fixed the zoom.","session_id":"s9","permission_denials":[{"tool_name":"Bash"}]}"#).unwrap();
        assert!(ok.ok && ok.answer.starts_with("Fixed the zoom.") && ok.answer.contains("Bash") && ok.session.as_deref() == Some("s9"));
        let login = claude_outcome(r#"{"type":"result","is_error":true,"result":"Failed to authenticate: OAuth session expired"}"#).unwrap();
        assert!(!login.ok && login_hint(Agent::Claude, &login.answer).contains("/login"));
        assert!(claude_outcome("not json").is_none());

        let printed = "{\"type\":\"thread.started\",\"thread_id\":\"t7\"}\n{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"OK\"}}\n";
        let c = codex_outcome(printed, None).unwrap();
        assert!(c.ok && c.answer == "OK" && c.session.as_deref() == Some("t7"));
        assert_eq!(codex_outcome(printed, Some("Done, all tests pass.\n")).unwrap().answer, "Done, all tests pass.");
        let failed = codex_outcome("{\"type\":\"turn.failed\",\"error\":{\"message\":\"quota\"}}", None).unwrap();
        assert!(!failed.ok && failed.answer == "quota");
    }

    /// This Mac's conversations, and a real prompt to Codex in a fresh folder
    /// (`CODE_SEND=1`): `cargo test -- --ignored this_macs --nocapture`.
    #[test]
    #[ignore]
    fn this_macs_conversations() {
        let home = crate::paths::home_dir().unwrap();
        let all = sessions(&home, None);
        for s in matching(&all, "") {
            println!("{:?} {} | {} | {}", s.agent, s.id, s.title, s.project);
        }
        println!("projects: {:?}", project_folder(&home, "dualeye"));
        if std::env::var("CODE_SEND").is_ok() {
            let inbox = Arc::new(Inbox::open(None));
            let coding = Coding::new(inbox.clone());
            println!("{}", coding.call_tool(&json!({"action": "send", "agent": "codex", "prompt": "Create hello.txt containing the word ciao, then reply: done"})).unwrap());
            for _ in 0..90 {
                thread::sleep(Duration::from_secs(2));
                if coding.runs.lock().unwrap()[0].state != "running" {
                    break;
                }
            }
            println!("{:#?}", coding.runs.lock().unwrap()[0]);
            println!("{:?}", inbox.list().first().map(|n| (&n.title, &n.text)));
        }
    }
}
