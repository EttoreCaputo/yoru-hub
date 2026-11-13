//! The user's coding agents on this computer, Claude Code, Codex and
//! OpenCode, as four of Yoru's tools:
//!
//! - `code_send` starts work: a new conversation in one of the user's
//!   projects or in a fresh folder, or the next message of a conversation
//!   (the user's, or a run of Yoru's that asked something).
//! - `code_status` says what is going on: what is being worked on now, and
//!   inside one conversation or run its last steps and what it last said
//!   ([`peek`]).
//! - `code_sessions` lists the user's past conversations, to find one.
//! - `code_control` stops a run, approves what a run stopped for, and
//!   watches a conversation of the user's own to say when it finishes or
//!   seems to wait for them.
//!
//! Yoru's runs are followed as they go (their steps, read from what the
//! agent prints), and when one ends Yoru says so out loud ([`Announcer`]):
//! what it answered, or what it needs from the user (an approval, an answer
//! to its question, a login). The words and the answer also go to the inbox.
//! The tools were one (`code`, with an action) until Gemini Live called its
//! listing instead of sending a prompt, and then said it had sent it.
//!
//! Nothing here talks to the agents' apps: their command lines are run as
//! the user would run them (`claude -p`, `codex exec`, `opencode run`), and
//! the conversations are read from the files they keep (`~/.claude/projects`,
//! `~/.codex/sessions`, OpenCode's database). Claude Code works in its `auto`
//! permission mode, where risky actions are checked and refused; Codex in its
//! `workspace-write` sandbox; OpenCode with `--auto`, which approves what its
//! configuration doesn't deny. What was refused is what the user is asked to
//! approve, and an approved run continues the conversation allowed to do it.

mod peek;
pub use peek::Peek;

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

use super::inbox::{Inbox, Urgency, now_ms};

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
/// What Yoru says of an answer out loud, at most: the rest is in the inbox.
const SPOKEN_CHARS: usize = 240;
/// A conversation written to this recently is being worked on.
pub const ACTIVE_MS: u64 = 3 * 60 * 1000;
/// A run of Yoru's stays in the active list this long after it ends, to be seen.
const ENDED_SHOWN_MS: u64 = 10 * 60 * 1000;
/// The active list is looked for again after this long, however often it's asked.
const ACTIVE_CACHE: Duration = Duration::from_secs(4);
/// Conversations `code_status` looks inside when asked about all of them.
const PEEKED: usize = 6;
/// A watched conversation is looked at this often…
const WATCH_EVERY: Duration = Duration::from_secs(5);
/// …and no longer after this long.
const WATCH_LIMIT: Duration = Duration::from_secs(4 * 60 * 60);
/// What approving a Codex run gives it: no sandbox, for that turn.
const FULL_ACCESS: &str = "full-access";

/// The tools' names.
pub const TOOLS: [&str; 4] = ["code_send", "code_status", "code_sessions", "code_control"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Claude,
    Codex,
    #[serde(rename = "opencode")]
    OpenCode,
}

impl Agent {
    fn parse(s: &str) -> Option<Agent> {
        match s.trim().to_lowercase().as_str() {
            "claude" | "claude code" | "claude-code" => Some(Agent::Claude),
            "codex" | "chatgpt" | "openai" | "gpt" => Some(Agent::Codex),
            "opencode" | "open code" | "open-code" => Some(Agent::OpenCode),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
            Agent::OpenCode => "OpenCode",
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
    /// `running`; `done`; `needs_you`: it ended waiting for the user (see
    /// `needs`); `failed`; `stopped`.
    pub state: &'static str,
    /// The agent's last answer, or what went wrong.
    pub result: String,
    /// Its last steps, oldest first, as they happen.
    pub steps: Vec<String>,
    /// What it needs from the user, when `needs_you`.
    pub needs: Option<String>,
    /// What approving lets it do: Claude Code's tools (`Bash(npm install)`),
    /// or Codex's [`FULL_ACCESS`].
    pub allow: Vec<String>,
    /// The language the user asked in (`it`, `en`): Yoru answers in it.
    pub language: &'static str,
    pub started: u64,
    /// Unix ms it ended, once it has.
    pub ended: Option<u64>,
}

/// A conversation being worked on now, or a run of Yoru's that just ended:
/// what the app and the notch show.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Active {
    pub agent: Agent,
    pub title: String,
    pub project: String,
    pub session: Option<String>,
    /// Unix ms of its last change.
    pub updated: u64,
    /// Yoru's run, when Yoru sent it.
    pub run: Option<u64>,
    /// `working`, or how Yoru's run ended (`done`, `needs_you`, `failed`, `stopped`).
    pub state: &'static str,
    /// Its last step, for Yoru's runs.
    pub now: Option<String>,
}

/// Says something to the user out loud, in a language (`it`, `en`): set by
/// the bridge, which has the boards.
pub type Announcer = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// A conversation of the user's that Yoru keeps an eye on.
#[derive(Debug, Clone)]
struct Watch {
    agent: Agent,
    session: String,
    title: String,
    /// The state last told, not to say it twice.
    told: Option<&'static str>,
    since: Instant,
}

/// The runs, and where their answers go.
pub struct Coding {
    inbox: Arc<Inbox>,
    runs: Mutex<Vec<Run>>,
    /// The running agents' processes, to stop them.
    children: Mutex<HashMap<u64, Arc<Mutex<Child>>>>,
    next: Mutex<u64>,
    /// The last active list, and when it was made.
    active: Mutex<Option<(Instant, Vec<Active>)>>,
    announcer: Mutex<Option<Announcer>>,
    watches: Mutex<Vec<Watch>>,
    /// The thread that looks at the watched conversations is running.
    watching: Mutex<bool>,
}

impl std::fmt::Debug for Coding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Coding").finish_non_exhaustive()
    }
}

impl Coding {
    pub fn new(inbox: Arc<Inbox>) -> Arc<Self> {
        Arc::new(Self {
            inbox,
            runs: Mutex::default(),
            children: Mutex::default(),
            next: Mutex::new(0),
            active: Mutex::default(),
            announcer: Mutex::default(),
            watches: Mutex::default(),
            watching: Mutex::new(false),
        })
    }

    /// Who says the runs' ends out loud; none, and they only reach the inbox.
    pub fn set_announcer(&self, announcer: Option<Announcer>) {
        *self.announcer.lock().unwrap() = announcer;
    }

    /// The model's tools.
    pub fn tools() -> Vec<Value> {
        let agent = json!({"type": "string", "enum": ["claude", "codex", "opencode"], "description": "Claude Code (the default), Codex or OpenCode"});
        vec![
            json!({
                "name": "code_send",
                "description": "Have one of the user's coding agents on this computer do something: Claude Code (claude, the default), Codex \
or OpenCode. This is the only way to start or continue an agent's work: call it whenever the user asks an agent to do, write, fix, check, \
look up or answer anything. Saying you sent it without this call sends nothing. \
New conversation: pass project (a name or folder) to work in one of the user's projects; with no project it gets a fresh folder of its own. \
Continue a conversation: pass session (an id from code_status or code_sessions), or run to answer or continue one of your runs (one that asked \
the user something). prompt is the user's request in their own words, whole, in their language: don't summarize or add to it. \
It works in the background, from seconds to many minutes. Say it started only once this answers \"sent to\". \
When it finishes or needs the user you will say so out loud yourself, unasked: don't promise anything else.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "prompt": {"type": "string", "description": "What to tell the agent: the user's request, in their words"},
                        "agent": agent,
                        "project": {"type": "string", "description": "A new conversation in this project: its name or folder"},
                        "session": {"type": "string", "description": "The id of a conversation to continue"},
                        "run": {"type": "integer", "description": "One of your runs to answer or continue"},
                    },
                    "required": ["prompt"],
                },
            }),
            json!({
                "name": "code_status",
                "description": "What the coding agents (Claude Code, Codex, OpenCode) are doing. With nothing: what is being worked on now, \
the user's own conversations and your runs, with what each is doing. With run or session: what is going on inside it now: its state, what it \
was asked, its last steps (commands run, files edited) and what it last said; for your runs, its answer and what it needs from the user. \
Call it when the user asks which sessions are active, how one is going, what an agent is doing or did, or answers something you told them \
about one. States: working; waiting (a step started a while ago and nothing since: maybe an approval in the agent's window, maybe a long \
command); your_turn (it finished and waits for the user's next message); and for your runs done, needs_you (see needs), failed, stopped.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "run": {"type": "integer", "description": "One of your runs"},
                        "session": {"type": "string", "description": "A conversation's id"},
                        "agent": agent,
                    },
                },
            }),
            json!({
                "name": "code_sessions",
                "description": "The user's past conversations with their coding agents (Claude Code, Codex, OpenCode), newest first, each with \
its id, title and project folder: to find one to continue or look into. Says nothing of whether they're active: code_status does.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "Words of the title, project or first message"},
                        "agent": {"type": "string", "enum": ["claude", "codex", "opencode"], "description": "All of them when left out"},
                    },
                },
            }),
            json!({
                "name": "code_control",
                "description": "Act on a coding agent's work. approve: one of your runs stopped because it wasn't allowed to do something \
(code_status says what, under needs): once the user said yes, let it do that and carry on. stop: stop one of your runs that is still working. \
watch: keep an eye on a conversation of the user's own (its session id, from code_status) and tell them out loud when it finishes or seems \
to wait for them; unwatch: stop watching it.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "action": {"type": "string", "enum": ["approve", "stop", "watch", "unwatch"]},
                        "run": {"type": "integer", "description": "approve, stop: the run"},
                        "session": {"type": "string", "description": "watch, unwatch: the conversation's id"},
                        "agent": agent,
                    },
                    "required": ["action"],
                },
            }),
        ]
    }

    /// Run tool `name` (one of [`TOOLS`]).
    pub fn call_tool(self: &Arc<Self>, name: &str, input: &Value) -> Result<String, String> {
        match name {
            "code_send" => self.send(input),
            "code_status" => self.status(input),
            "code_sessions" => {
                let home = crate::paths::home_dir().ok_or("no home folder")?;
                let all = sessions(&home, text(input, "agent").and_then(Agent::parse));
                let found = matching(&all, text(input, "query").unwrap_or_default());
                if found.is_empty() {
                    return Ok(if all.is_empty() { "no conversations found".into() } else { "no conversation matches; try fewer words".into() });
                }
                Ok(json!(found.iter().map(|s| json!({"agent": s.agent, "id": s.id, "title": s.title, "project": s.project, "updated": local(s.updated)})).collect::<Vec<_>>()).to_string())
            }
            "code_control" => match input["action"].as_str().unwrap_or_default() {
                "stop" => self.stop(input["run"].as_u64().ok_or("run is needed")?),
                "approve" => self.approve(input["run"].as_u64().ok_or("run is needed")?),
                "watch" => self.watch(input),
                "unwatch" => {
                    let id = text(input, "session").ok_or("session is needed")?;
                    let mut watches = self.watches.lock().unwrap();
                    let before = watches.len();
                    watches.retain(|w| w.session != id);
                    Ok(if watches.len() < before { format!("stopped watching {id}") } else { format!("{id} wasn't watched") })
                }
                other => Err(format!("unknown action {other}")),
            },
            other => Err(format!("unknown tool {other}")),
        }
    }

    /// `code_send`.
    fn send(self: &Arc<Self>, input: &Value) -> Result<String, String> {
        let prompt = text(input, "prompt").ok_or("prompt is needed")?;
        let agent = text(input, "agent").and_then(Agent::parse);
        let home = crate::paths::home_dir().ok_or("no home folder")?;
        let (agent, project, session) = if let Some(id) = input["run"].as_u64() {
            let run = self.run(id).ok_or(format!("there is no run {id}"))?;
            let session = run.session.clone().ok_or(format!("run {id} has no conversation to continue yet: send a new one"))?;
            (run.agent, PathBuf::from(&run.project), Some(session))
        } else if let Some(id) = text(input, "session") {
            let all = sessions(&home, None);
            let s = all.iter().find(|s| s.id == id && agent.is_none_or(|a| a == s.agent)).ok_or(format!("no conversation {id}: find it with code_sessions"))?;
            (s.agent, PathBuf::from(&s.project), Some(s.id.clone()))
        } else if let Some(name) = text(input, "project") {
            (agent.unwrap_or(Agent::Claude), project_folder(&home, name)?, None)
        } else {
            (agent.unwrap_or(Agent::Claude), fresh_folder(&home, prompt)?, None)
        };
        if let Some(busy) = self.runs.lock().unwrap().iter().find(|r| r.state == "running" && session.is_some() && r.session == session) {
            return Err(format!("run {} is still working in that conversation: wait for it, or stop it first", busy.id));
        }
        let run = self.start(agent, &project, session.as_deref(), prompt, &[])?;
        Ok(format!("sent to {} (run {run}) in {}: it works on its own; you'll tell the user when it's done or needs them", agent.name(), project.display()))
    }

    /// `code_status`.
    fn status(&self, input: &Value) -> Result<String, String> {
        let home = crate::paths::home_dir().ok_or("no home folder")?;
        let now = now_ms();
        if let Some(id) = input["run"].as_u64() {
            let run = self.run(id).ok_or(format!("there is no run {id}"))?;
            let mut out = json!({
                "run": run.id, "agent": run.agent, "project": run.project, "session": run.session, "asked": run.prompt,
                "state": run.state, "steps": run.steps, "started": local(run.started),
            });
            if run.state != "running" {
                out["answer"] = json!(clip(&run.result, ANSWER_CHARS));
            }
            if let Some(needs) = &run.needs {
                out["needs"] = json!(needs);
            }
            return Ok(out.to_string());
        }
        if let Some(id) = text(input, "session") {
            if let Some(run) = self.runs.lock().unwrap().iter().rev().find(|r| r.session.as_deref() == Some(id) && r.state == "running") {
                return self.status(&json!({"run": run.id}));
            }
            let agent = text(input, "agent").and_then(Agent::parse);
            let agent = match agent {
                Some(a) => a,
                None => sessions(&home, None).iter().find(|s| s.id == id).map(|s| s.agent).ok_or(format!("no conversation {id}: find it with code_sessions"))?,
            };
            let p = peek::peek(&home, agent, id, now).ok_or(format!("couldn't read conversation {id}"))?;
            return Ok(json!({"agent": agent, "session": id, "state": p.state, "asked": p.asked, "steps": p.steps, "said": p.said, "updated": local(p.updated)}).to_string());
        }
        let active = self.active();
        if active.is_empty() {
            return Ok("nothing is being worked on now".into());
        }
        let list: Vec<Value> = active
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let mut item = json!({"agent": a.agent, "title": a.title, "project": a.project, "session": a.session, "state": a.state, "updated": local(a.updated)});
                if let Some(run) = a.run {
                    item["run"] = json!(run);
                    item["doing"] = json!(a.now);
                } else if let Some(p) = a.session.as_deref().filter(|_| i < PEEKED).and_then(|id| peek::peek(&home, a.agent, id, now)) {
                    item["state"] = json!(p.state);
                    item["doing"] = json!(p.steps.last());
                    item["said"] = json!(p.said.map(|s| clip(&s, 200)));
                }
                item
            })
            .collect();
        Ok(json!(list).to_string())
    }

    /// Stop run `id`.
    pub fn stop(&self, id: u64) -> Result<String, String> {
        let child = self.children.lock().unwrap().get(&id).cloned().ok_or(format!("run {id} isn't running"))?;
        let _ = child.lock().unwrap().kill();
        self.finish(id, Ending { state: "stopped", result: "stopped by the user".into(), session: None, needs: None, allow: Vec::new() });
        Ok(format!("stopped run {id}"))
    }

    /// Let run `id` do what it stopped for, continuing its conversation.
    fn approve(self: &Arc<Self>, id: u64) -> Result<String, String> {
        let run = self.run(id).ok_or(format!("there is no run {id}"))?;
        if run.state != "needs_you" || run.allow.is_empty() {
            return Err(match run.needs {
                Some(needs) => format!("run {id} doesn't need an approval but {needs}: answer it with code_send and run {id}"),
                None => format!("run {id} isn't waiting for an approval ({})", run.state),
            });
        }
        let session = run.session.clone().ok_or(format!("run {id} has no conversation to continue"))?;
        let what = run.needs.clone().unwrap_or_default();
        let prompt = format!("The user approved it ({what}). Go on with what you were doing.");
        let new = self.start(run.agent, Path::new(&run.project), Some(&session), &prompt, &run.allow)?;
        if let Some(r) = self.runs.lock().unwrap().iter_mut().find(|r| r.id == id) {
            r.state = "done";
            r.needs = None;
        }
        // The new run speaks the user's language, not the prompt's.
        if let Some(r) = self.runs.lock().unwrap().iter_mut().find(|r| r.id == new) {
            r.language = run.language;
        }
        Ok(format!("approved: {} goes on as run {new}", run.agent.name()))
    }

    /// Watch a conversation of the user's.
    fn watch(self: &Arc<Self>, input: &Value) -> Result<String, String> {
        let id = text(input, "session").ok_or("session is needed")?;
        let home = crate::paths::home_dir().ok_or("no home folder")?;
        let all = sessions(&home, text(input, "agent").and_then(Agent::parse));
        let s = all.iter().find(|s| s.id == id).ok_or(format!("no conversation {id}: find it with code_status or code_sessions"))?;
        {
            let mut watches = self.watches.lock().unwrap();
            watches.retain(|w| w.session != id);
            watches.push(Watch { agent: s.agent, session: s.id.clone(), title: s.title.clone(), told: None, since: Instant::now() });
        }
        let mut watching = self.watching.lock().unwrap();
        if !*watching {
            *watching = true;
            let me = self.clone();
            thread::Builder::new().name("yoru-code-watch".into()).spawn(move || me.keep_watching()).map_err(|e| e.to_string())?;
        }
        Ok(format!("watching \"{}\": you'll tell the user when it finishes or seems to wait for them", s.title))
    }

    /// Look at the watched conversations until none is left.
    fn keep_watching(self: Arc<Self>) {
        let home = crate::paths::home_dir().unwrap_or_default();
        loop {
            thread::sleep(WATCH_EVERY);
            let watches = self.watches.lock().unwrap().clone();
            if watches.is_empty() {
                let mut watching = self.watching.lock().unwrap();
                // A watch added meanwhile keeps the thread.
                if self.watches.lock().unwrap().is_empty() {
                    *watching = false;
                    return;
                }
                continue;
            }
            for w in watches {
                let peeked = peek::peek(&home, w.agent, &w.session, now_ms());
                let state = peeked.as_ref().map_or("gone", |p| p.state);
                let over = matches!(state, "your_turn" | "interrupted" | "gone") || w.since.elapsed() > WATCH_LIMIT;
                let news = peeked.filter(|_| w.told != Some(state)).and_then(|p| watched_news(&w, &p).map(|n| (n, p.asked)));
                if let Some(((said, urgency), asked)) = news {
                    let language = crate::intents::guess_language(asked.as_deref().unwrap_or(&said));
                    self.inbox.post(w.agent.name(), &clip(&w.title, 80), &said, urgency, None, Some("code".into()));
                    self.announce(&said, language);
                }
                let mut watches = self.watches.lock().unwrap();
                if over {
                    watches.retain(|x| x.session != w.session);
                } else if let Some(x) = watches.iter_mut().find(|x| x.session == w.session) {
                    x.told = Some(state);
                }
            }
        }
    }

    fn announce(&self, text: &str, language: &str) {
        let announcer = self.announcer.lock().unwrap().clone();
        if let Some(say) = announcer {
            say(text, language);
        }
    }

    fn run(&self, id: u64) -> Option<Run> {
        self.runs.lock().unwrap().iter().find(|r| r.id == id).cloned()
    }

    pub fn active(&self) -> Vec<Active> {
        if let Some((at, list)) = self.active.lock().unwrap().as_ref() {
            if at.elapsed() < ACTIVE_CACHE {
                return list.clone();
            }
        }
        let home = crate::paths::home_dir().unwrap_or_default();
        let now = now_ms();
        let runs = self.runs.lock().unwrap().clone();
        let list = active_list(&runs, sessions_since(&home, None, now.saturating_sub(ACTIVE_MS)), now);
        *self.active.lock().unwrap() = Some((Instant::now(), list.clone()));
        list
    }

    /// Send `prompt` to `agent` in `project` (continuing `session`, allowed
    /// to do `allow`), in the background. The run's id.
    fn start(self: &Arc<Self>, agent: Agent, project: &Path, session: Option<&str>, prompt: &str, allow: &[String]) -> Result<u64, String> {
        let program = program(agent).ok_or(match agent {
            Agent::Claude => "Claude Code isn't installed as a command (claude)",
            Agent::Codex => "Codex isn't installed (its command, or the ChatGPT app)",
            Agent::OpenCode => "OpenCode isn't installed as a command (opencode)",
        })?;
        let home = crate::paths::home_dir().unwrap_or_default();
        let model = (agent == Agent::OpenCode).then(|| opencode_model(&home, session, project)).flatten();
        let out = std::env::temp_dir().join(format!("yoru-codex-{}-{}.txt", std::process::id(), now_ms()));
        let mut command = Command::new(&program);
        command.args(arguments(agent, session, model.as_deref(), prompt, &out, allow)).current_dir(project).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        // OpenCode works where PWD says, not in the process's folder.
        command.env("PWD", project);
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
            steps: Vec::new(),
            needs: None,
            allow: Vec::new(),
            language: crate::intents::guess_language(prompt),
            started: now_ms(),
            ended: None,
        };
        {
            let mut runs = self.runs.lock().unwrap();
            runs.push(run);
            let drop_n = runs.len().saturating_sub(KEPT_RUNS);
            runs.drain(..drop_n);
        }
        let child = Arc::new(Mutex::new(child));
        self.children.lock().unwrap().insert(id, child.clone());
        *self.active.lock().unwrap() = None;
        crate::hostlog::line("code", &format!("run {id}: {} in {} {}", agent.name(), project.display(), session.map_or("(new conversation)".into(), |s| format!("(continuing {s})"))));
        let me = self.clone();
        thread::Builder::new()
            .name("yoru-code".into())
            .spawn(move || me.wait(id, agent, child, &out))
            .map_err(|e| e.to_string())?;
        Ok(id)
    }

    /// Follow run `id` until it ends, then say how it went.
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
        let mut blocked = None;
        if let Some(o) = stdout {
            for line in BufReader::new(o).lines().map_while(Result::ok) {
                if let Ok(event) = serde_json::from_str::<Value>(line.trim()) {
                    let progress = progress(agent, &event);
                    blocked = blocked.or(progress.blocked);
                    self.update(id, |r| {
                        if r.session.is_none() {
                            r.session = progress.session;
                        }
                        for s in progress.steps {
                            push_step(&mut r.steps, s);
                        }
                    });
                }
                lines.push_str(&line);
                lines.push('\n');
            }
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
            Agent::OpenCode => opencode_outcome(&lines),
        };
        let ending = match outcome {
            Some(o) if o.ok => {
                let (needs, allow) = needs_of(&o, blocked.as_deref());
                Ending { state: if needs.is_some() { "needs_you" } else { "done" }, result: o.answer, session: o.session, needs, allow }
            }
            Some(o) => Ending { state: "failed", result: login_hint(agent, &o.answer), session: o.session, needs: None, allow: Vec::new() },
            None => {
                let why = errors.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("it stopped without an answer").to_string();
                let code = status.and_then(|s| s.code()).map_or(String::new(), |c| format!(" (exit {c})"));
                Ending { state: "failed", result: login_hint(agent, &format!("{why}{code}")), session: None, needs: None, allow: Vec::new() }
            }
        };
        self.finish(id, ending);
    }

    fn update(&self, id: u64, change: impl FnOnce(&mut Run)) {
        if let Some(r) = self.runs.lock().unwrap().iter_mut().find(|r| r.id == id) {
            change(r);
        }
    }

    /// Mark run `id` ended, and tell the user: out loud, and in the inbox.
    fn finish(&self, id: u64, ending: Ending) {
        let run = {
            let mut runs = self.runs.lock().unwrap();
            let Some(run) = runs.iter_mut().find(|r| r.id == id) else { return };
            if run.state != "running" {
                return;
            }
            run.state = ending.state;
            run.result = ending.result;
            run.needs = ending.needs;
            run.allow = ending.allow;
            run.ended = Some(now_ms());
            if run.session.is_none() {
                run.session = ending.session;
            }
            run.clone()
        };
        *self.active.lock().unwrap() = None;
        crate::hostlog::line("code", &format!("run {id}: {}{}", run.state, run.needs.as_ref().map_or(String::new(), |n| format!(", needs {n}"))));
        let place = folder_name(&run.project);
        let title = match run.state {
            "done" => format!("{place}: done"),
            "needs_you" => format!("{place}: needs you"),
            "stopped" => format!("{place}: stopped"),
            _ => format!("{place}: it didn't work"),
        };
        let spoken = announcement(&run);
        let text = format!("{}\n\nAsked: {}", clip(&run.result, ANSWER_CHARS), clip(&run.prompt, 200));
        let urgency = if run.state == "needs_you" { Urgency::High } else { Urgency::Normal };
        self.inbox.post(run.agent.name(), &title, &text, urgency, None, Some("code".into()));
        if let Some(spoken) = spoken {
            self.announce(&spoken, run.language);
        }
    }
}

/// How a run ended.
struct Ending {
    state: &'static str,
    result: String,
    session: Option<String>,
    needs: Option<String>,
    allow: Vec<String>,
}

/// A tool input's text field, trimmed, when not empty.
fn text<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input[key].as_str().map(str::trim).filter(|s| !s.is_empty())
}

fn push_step(steps: &mut Vec<String>, step: String) {
    if steps.last() != Some(&step) {
        steps.push(step);
    }
    let excess = steps.len().saturating_sub(peek::STEPS);
    steps.drain(..excess);
}

/// A folder's last part.
fn folder_name(path: &str) -> String {
    Path::new(path).file_name().map_or(path.to_string(), |n| n.to_string_lossy().into_owned())
}

/// What one line an agent printed while working says.
#[derive(Default)]
struct Progress {
    session: Option<String>,
    steps: Vec<String>,
    /// A command the sandbox stopped (Codex).
    blocked: Option<String>,
}

/// Read one event an agent printed: Claude Code's `stream-json`, Codex's
/// and OpenCode's `--json`.
fn progress(agent: Agent, e: &Value) -> Progress {
    let mut p = Progress::default();
    match agent {
        Agent::Claude => {
            if e["type"] == "system" && e["subtype"] == "init" {
                p.session = e["session_id"].as_str().map(str::to_string);
            }
            if e["type"] == "assistant" && e["parent_tool_use_id"].is_null() {
                for c in e["message"]["content"].as_array().into_iter().flatten().filter(|c| c["type"] == "tool_use") {
                    p.steps.push(peek::step(c["name"].as_str().unwrap_or("a tool"), &c["input"]));
                }
            }
        }
        Agent::Codex => {
            if e["type"] == "thread.started" {
                p.session = e["thread_id"].as_str().map(str::to_string);
            }
            let item = &e["item"];
            let started = e["type"] == "item.started";
            let completed = e["type"] == "item.completed";
            match item["type"].as_str() {
                Some("command_execution") if started => p.steps.push(peek::step("command_execution", item)),
                Some("command_execution") if completed => {
                    let output = item["aggregated_output"].as_str().unwrap_or_default();
                    let failed = item["exit_code"].as_i64().is_some_and(|c| c != 0);
                    if failed && (output.contains("Operation not permitted") || output.contains("sandbox")) {
                        p.blocked = item["command"].as_str().map(|c| clip(c, 80));
                    }
                }
                Some("file_change") if completed => {
                    let path = item["changes"][0]["path"].as_str().unwrap_or_default();
                    p.steps.push(peek::step("edit", &json!({"path": path})));
                }
                Some("mcp_tool_call") if started => p.steps.push(format!("used {} {}", item["server"].as_str().unwrap_or_default(), item["tool"].as_str().unwrap_or_default())),
                Some("web_search") if completed => p.steps.push(peek::step("web_search", item)),
                _ => {}
            }
        }
        Agent::OpenCode => {
            p.session = e["sessionID"].as_str().map(str::to_string);
            let part = &e["part"];
            if part["type"] == "tool" {
                p.steps.push(peek::step(part["tool"].as_str().unwrap_or("a tool"), &part["state"]["input"]));
            }
        }
    }
    p
}

/// What a run that ended well still needs from the user, and what
/// approving it allows: the actions it was refused, the sandbox that
/// stopped a command, or an answer to the question it ended with.
fn needs_of(o: &Outcome, blocked: Option<&str>) -> (Option<String>, Vec<String>) {
    if !o.denied.is_empty() {
        let what: Vec<String> = o.denied.iter().map(|(tool, input)| peek::step(tool, input)).collect();
        let allow = o
            .denied
            .iter()
            .map(|(tool, input)| match input["command"].as_str() {
                Some(c) if tool == "Bash" => format!("Bash({c})"),
                _ => tool.clone(),
            })
            .collect();
        return (Some(format!("approval for what it wasn't allowed to do: {}", what.join("; "))), allow);
    }
    if let Some(command) = blocked {
        return (Some(format!("approval for full access outside its sandbox, which stopped: {command}")), vec![FULL_ACCESS.into()]);
    }
    if question_in(&o.answer).is_some() {
        return (Some("an answer to its question".into()), Vec::new());
    }
    (None, Vec::new())
}

/// The question an answer ends with.
fn question_in(answer: &str) -> Option<String> {
    let plain = plain_words(answer);
    let end = plain.trim_end_matches(|c: char| c.is_whitespace() || c == ')' || c == '"');
    if !end.ends_with('?') {
        return None;
    }
    let body = &end[..end.len() - 1];
    let start = body.rfind(['.', '!', '?', ':', '\n']).map_or(0, |i| i + 1);
    Some(clip(end[start..].trim(), SPOKEN_CHARS))
}

/// Markdown made into words to say: no code blocks, marks, bullets or links' addresses.
fn plain_words(text: &str) -> String {
    let mut out = Vec::new();
    let mut fenced = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced || l.is_empty() || l.starts_with('|') {
            continue;
        }
        let l = l.trim_start_matches(['#', '>', ' ']).trim_start_matches("- ").trim_start_matches("* ");
        let mut words = String::new();
        let mut rest = l;
        // [text](url) → text
        while let Some(open) = rest.find('[') {
            let Some(close) = rest[open..].find("](").map(|c| open + c) else { break };
            let Some(end) = rest[close..].find(')').map(|e| close + e) else { break };
            words.push_str(&rest[..open]);
            words.push_str(&rest[open + 1..close]);
            rest = &rest[end + 1..];
        }
        words.push_str(rest);
        out.push(words.replace(['*', '`', '_'], ""));
    }
    out.join(" ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The start of an answer to say: whole sentences up to [`SPOKEN_CHARS`],
/// and whether some was left out.
fn spoken_part(answer: &str) -> (String, bool) {
    let plain = plain_words(answer);
    if plain.chars().count() <= SPOKEN_CHARS {
        return (plain, false);
    }
    let cut: String = plain.chars().take(SPOKEN_CHARS).collect();
    let end = cut.rfind(['.', '!', '?']).filter(|&i| i > SPOKEN_CHARS / 3).map_or(cut.len(), |i| i + 1);
    (cut[..end].trim().to_string(), true)
}

/// What Yoru says when run `run` ends, in its language; nothing for one
/// the user stopped.
fn announcement(run: &Run) -> Option<String> {
    let it = run.language == "it";
    let agent = run.agent.name();
    // A fresh folder's name is only the prompt's first words.
    let in_place = if temporary(&run.project) { String::new() } else { format!(" in {}", folder_name(&run.project)) };
    match run.state {
        "done" => {
            let (said, more) = spoken_part(&run.result);
            let rest = match (more, it) {
                (false, _) => "",
                (true, true) => " Il resto è nella inbox.",
                (true, false) => " The rest is in the inbox.",
            };
            Some(match (said.is_empty(), it) {
                (true, true) => format!("{agent} ha finito{in_place}."),
                (true, false) => format!("{agent} is done{in_place}."),
                (false, true) => format!("{agent} ha finito{in_place}: {said}{rest}"),
                (false, false) => format!("{agent} is done{in_place}: {said}{rest}"),
            })
        }
        "needs_you" if !run.allow.is_empty() => {
            let what: Vec<String> = run
                .allow
                .iter()
                .map(|a| match (a.as_str(), it) {
                    (FULL_ACCESS, true) => "lavorare fuori dalla sua cartella".into(),
                    (FULL_ACCESS, false) => "work outside its folder".into(),
                    (a, _) => a.strip_prefix("Bash(").and_then(|c| c.strip_suffix(')')).unwrap_or(a).to_string(),
                })
                .collect();
            let what = clip(&what.join(", "), 140);
            Some(if it {
                format!("{agent} si è fermato{in_place}: chiede il permesso per {what}. Vuoi che lo lasci fare?")
            } else {
                format!("{agent} stopped{in_place}: it asks for permission to {what}. Shall I let it?")
            })
        }
        "needs_you" => {
            let question = question_in(&run.result).unwrap_or_default();
            Some(if it { format!("{agent} ti chiede{in_place}: {question}") } else { format!("{agent} asks you{in_place}: {question}") })
        }
        "failed" => {
            let why = clip(run.result.lines().find(|l| !l.trim().is_empty()).unwrap_or_default(), 160);
            let login = run.result.contains("needs a login");
            Some(match (login, it) {
                (true, true) => format!("{agent} non è partito: va rifatto il login. {why}"),
                (true, false) => format!("{agent} didn't start: it needs a login. {why}"),
                (false, true) => format!("{agent} non ce l'ha fatta{in_place}: {why}"),
                (false, false) => format!("{agent} couldn't do it{in_place}: {why}"),
            })
        }
        _ => None,
    }
}

/// What to tell the user about a watched conversation that is now `p`,
/// and how loud; nothing while it works.
fn watched_news(w: &Watch, p: &Peek) -> Option<(String, Urgency)> {
    let it = crate::intents::guess_language(p.asked.as_deref().or(p.said.as_deref()).unwrap_or_default()) == "it";
    let agent = w.agent.name();
    let title = clip(&w.title, 60);
    match p.state {
        "your_turn" => {
            let said = p.said.as_deref().unwrap_or_default();
            if let Some(q) = question_in(said) {
                return Some((if it { format!("{agent}, in «{title}», ti chiede: {q}") } else { format!("{agent} asks you, in “{title}”: {q}") }, Urgency::High));
            }
            let (said, more) = spoken_part(said);
            let rest = match (more, it) {
                (false, _) => "",
                (true, true) => " Il resto è nella sessione.",
                (true, false) => " The rest is in the session.",
            };
            Some((
                match (said.is_empty(), it) {
                    (true, true) => format!("{agent} ha finito «{title}»."),
                    (true, false) => format!("{agent} finished “{title}”."),
                    (false, true) => format!("{agent} ha finito «{title}»: {said}{rest}"),
                    (false, false) => format!("{agent} finished “{title}”: {said}{rest}"),
                },
                Urgency::Normal,
            ))
        }
        "waiting" => {
            let step = p.steps.last().cloned().unwrap_or_default();
            Some((
                if it {
                    format!("{agent} è fermo da un po' in «{title}», su: {step}. Forse aspetta un tuo permesso.")
                } else {
                    format!("{agent} has been on one step for a while in “{title}”: {step}. It may be waiting for your approval.")
                },
                Urgency::High,
            ))
        }
        _ => None,
    }
}

/// [`Coding::active`] from Yoru's `runs` and the conversations written to
/// lately (`recent`), at `now`. A conversation of Yoru's run is shown once,
/// as the run: by its id, or (a new one, not named yet) by its folder.
fn active_list(runs: &[Run], recent: Vec<Session>, now: u64) -> Vec<Active> {
    let shown: Vec<&Run> = runs.iter().filter(|r| r.state == "running" || r.ended.is_some_and(|e| now.saturating_sub(e) < ENDED_SHOWN_MS)).collect();
    let mut list: Vec<Active> = shown
        .iter()
        .map(|r| Active {
            agent: r.agent,
            title: clip(&r.prompt.replace('\n', " "), 120),
            project: r.project.clone(),
            session: r.session.clone(),
            updated: r.ended.unwrap_or(r.started),
            run: Some(r.id),
            state: if r.state == "running" { "working" } else { r.state },
            now: r.steps.last().cloned(),
        })
        .collect();
    for s in recent {
        let yorus = shown.iter().any(|r| {
            r.agent == s.agent && (r.session.as_deref() == Some(s.id.as_str()) || (r.session.is_none() && r.project == s.project && s.updated >= r.started))
        });
        if !yorus {
            list.push(Active { agent: s.agent, title: s.title, project: s.project, session: Some(s.id), updated: s.updated, run: None, state: "working", now: None });
        }
    }
    let rank = |a: &Active| match a.state {
        "needs_you" => 0,
        "working" => 1,
        _ => 2,
    };
    list.sort_by(|a, b| rank(a).cmp(&rank(b)).then(b.updated.cmp(&a.updated)));
    list
}

/// How an agent's run ended, read from what it printed.
struct Outcome {
    ok: bool,
    answer: String,
    session: Option<String>,
    /// What it wasn't allowed to do: each tool and its input (Claude Code).
    denied: Vec<(String, Value)>,
}

/// `claude -p --output-format stream-json`: its last line is the result.
fn claude_outcome(printed: &str) -> Option<Outcome> {
    let result: Value = printed.lines().rev().find_map(|l| serde_json::from_str::<Value>(l.trim()).ok().filter(|v| v["type"] == "result"))?;
    let answer = result["result"].as_str().unwrap_or_default().trim().to_string();
    let denied = result["permission_denials"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| Some((d["tool_name"].as_str()?.to_string(), d["tool_input"].clone())))
        .collect();
    Some(Outcome { ok: result["is_error"] != true, answer, session: result["session_id"].as_str().map(str::to_string), denied })
}

/// `codex exec --json`: events a line, the last message in `last` (`-o`).
fn codex_outcome(printed: &str, last: Option<&str>) -> Option<Outcome> {
    let events: Vec<Value> = printed.lines().filter_map(|l| serde_json::from_str(l.trim()).ok()).collect();
    let session = events.iter().find(|e| e["type"] == "thread.started").and_then(|e| e["thread_id"].as_str()).map(str::to_string);
    let said = events.iter().rev().find(|e| e["item"]["type"] == "agent_message").and_then(|e| e["item"]["text"].as_str());
    let failed = events.iter().rev().find(|e| e["type"] == "turn.failed" || e["type"] == "error");
    let answer = last.map(str::trim).filter(|t| !t.is_empty()).or(said);
    match (answer, failed) {
        (Some(a), None) => Some(Outcome { ok: true, answer: a.to_string(), session, denied: Vec::new() }),
        (_, Some(f)) => {
            let why = f["error"]["message"].as_str().or(f["message"].as_str()).unwrap_or("the turn failed");
            Some(Outcome { ok: false, answer: why.to_string(), session, denied: Vec::new() })
        }
        (None, None) => None,
    }
}

/// `opencode run --format json`: events a line; the answer is the text of
/// the last message.
fn opencode_outcome(printed: &str) -> Option<Outcome> {
    let events: Vec<Value> = printed.lines().filter_map(|l| serde_json::from_str(l.trim()).ok()).collect();
    let session = events.iter().find_map(|e| e["sessionID"].as_str()).map(str::to_string);
    let texts: Vec<&Value> = events.iter().filter(|e| e["type"] == "text").collect();
    let last = texts.last().and_then(|e| e["part"]["messageID"].as_str());
    let answer: Vec<&str> = texts.iter().filter(|e| e["part"]["messageID"].as_str() == last).filter_map(|e| e["part"]["text"].as_str()).collect();
    let answer = answer.join("\n").trim().to_string();
    if let Some(f) = events.iter().rev().find(|e| e["type"] == "error") {
        let why = f["error"]["message"].as_str().or(f["error"]["data"]["message"].as_str()).unwrap_or("the run failed");
        return Some(Outcome { ok: false, answer: why.to_string(), session, denied: Vec::new() });
    }
    (!answer.is_empty()).then_some(Outcome { ok: true, answer, session, denied: Vec::new() })
}

/// What went wrong, with how to fix a login that ran out.
fn login_hint(agent: Agent, why: &str) -> String {
    let lower = why.to_lowercase();
    if lower.contains("authenticat") || lower.contains("login") || lower.contains("logged in") || lower.contains("oauth") {
        let how = match agent {
            Agent::Claude => "run claude in the Terminal and log in (/login)",
            Agent::Codex => "run codex login in the Terminal",
            Agent::OpenCode => "run opencode auth login in the Terminal",
        };
        format!("{why}\n\n{} needs a login: {how}.", agent.name())
    } else {
        why.to_string()
    }
}

/// The command line for `agent`, allowed to do `allow` besides what its
/// mode lets it.
fn arguments(agent: Agent, session: Option<&str>, model: Option<&str>, prompt: &str, out: &Path, allow: &[String]) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    match agent {
        Agent::Claude => {
            // stream-json, to follow its steps as they happen (it wants --verbose).
            args.extend(["-p", "--output-format", "stream-json", "--verbose", "--permission-mode", "auto"].map(String::from));
            if !allow.is_empty() {
                args.push("--allowedTools".into());
                args.extend(allow.iter().cloned());
            }
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
            let sandbox = if allow.iter().any(|a| a == FULL_ACCESS) { "danger-full-access" } else { "workspace-write" };
            args.extend(["--skip-git-repo-check", "--json", "-c"].map(String::from));
            args.push(format!("sandbox_mode=\"{sandbox}\""));
            args.push("-o".into());
            args.push(out.display().to_string());
            if let Some(s) = session {
                args.push(s.into());
            }
            args.extend(["--".into(), prompt.into()]);
        }
        Agent::OpenCode => {
            args.extend(["run", "--format", "json", "--auto"].map(String::from));
            if let Some(s) = session {
                args.extend(["--session".into(), s.into()]);
            }
            if let Some(m) = model {
                args.extend(["--model".into(), m.into()]);
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
        Agent::OpenCode => "opencode",
    };
    let mut places: Vec<PathBuf> = login_path().unwrap_or_default().split(':').map(|d| Path::new(d).join(name)).collect();
    places.extend([home.join(".local/bin").join(name), PathBuf::from("/opt/homebrew/bin").join(name), PathBuf::from("/usr/local/bin").join(name)]);
    match agent {
        Agent::Claude => places.push(home.join(".claude/local/claude")),
        Agent::Codex => places.extend([
            PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex"),
            PathBuf::from("/Applications/Codex.app/Contents/Resources/codex-cli/bin/codex"),
        ]),
        Agent::OpenCode => places.push(home.join(".opencode/bin/opencode")),
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

/// Every conversation of `agent` (all of them when `None`), newest first.
pub fn sessions(home: &Path, agent: Option<Agent>) -> Vec<Session> {
    sessions_since(home, agent, 0)
}

/// The conversations of `agent` changed since `since` (Unix ms), newest first.
pub fn sessions_since(home: &Path, agent: Option<Agent>, since: u64) -> Vec<Session> {
    let mut all = Vec::new();
    if agent.is_none_or(|a| a == Agent::Claude) {
        all.extend(claude_sessions(home, since));
    }
    if agent.is_none_or(|a| a == Agent::Codex) {
        all.extend(codex_sessions(home, since));
    }
    if agent.is_none_or(|a| a == Agent::OpenCode) {
        all.extend(opencode_sessions(home, since).into_iter().map(|(s, _)| s));
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

/// The newest `.jsonl` files under `dir` (recursively) changed since
/// `since` (Unix ms), with their times.
fn newest_files(dir: &Path, n: usize, since: u64) -> Vec<(PathBuf, u64)> {
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
                if at >= since {
                    files.push((p, at));
                }
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
fn claude_sessions(home: &Path, since: u64) -> Vec<Session> {
    newest_files(&home.join(".claude/projects"), NEWEST, since)
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
fn codex_sessions(home: &Path, since: u64) -> Vec<Session> {
    let names: HashMap<String, String> = fs::read_to_string(home.join(".codex/session_index.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| Some((v["id"].as_str()?.to_string(), v["thread_name"].as_str()?.to_string())))
        .collect();
    newest_files(&home.join(".codex/sessions"), NEWEST, since)
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

/// OpenCode's database, where it keeps its conversations.
fn opencode_db(home: &Path) -> PathBuf {
    home.join(".local/share/opencode/opencode.db")
}

/// OpenCode's conversations (not the subagents' or archived ones), newest
/// first, each with the model it used (`provider/model#variant`). Its
/// database has two tables, `session_v2` and the older `session`.
fn opencode_sessions(home: &Path, since: u64) -> Vec<(Session, Option<String>)> {
    let path = opencode_db(home);
    if !path.is_file() {
        return Vec::new();
    }
    let Ok(db) = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) else { return Vec::new() };
    let mut found: Vec<(Session, Option<String>)> = Vec::new();
    for table in ["session_v2", "session"] {
        let sql = format!(
            "SELECT id, title, directory, time_updated, model FROM {table} WHERE parent_id IS NULL AND time_archived IS NULL AND time_updated >= {since} ORDER BY time_updated DESC LIMIT {NEWEST}"
        );
        let Ok(mut query) = db.prepare(&sql) else { continue };
        let rows = query.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        });
        for (id, title, project, updated, model) in rows.into_iter().flatten().flatten() {
            if found.iter().any(|(s, _)| s.id == id) {
                continue;
            }
            let title = title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "(untitled)".into());
            let session = Session { agent: Agent::OpenCode, id, title, project, updated: updated.max(0) as u64 };
            found.push((session, model.as_deref().and_then(opencode_model_name)));
        }
    }
    found.sort_by(|a, b| b.0.updated.cmp(&a.0.updated));
    found
}

/// `{"id","providerID","variant"}` as OpenCode's `--model` wants it.
fn opencode_model_name(json: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json).ok()?;
    let mut name = format!("{}/{}", v["providerID"].as_str()?, v["id"].as_str()?);
    if let Some(variant) = v["variant"].as_str().filter(|s| !s.is_empty()) {
        name.push('#');
        name.push_str(variant);
    }
    Some(name)
}

/// The model for an OpenCode run: the conversation's own, else the one the
/// user last used in that folder, else anywhere. OpenCode's default may be
/// one its provider can't run, and the user's last choice is what works.
fn opencode_model(home: &Path, session: Option<&str>, project: &Path) -> Option<String> {
    let all = opencode_sessions(home, 0);
    let here = project.display().to_string();
    let pick = |f: &dyn Fn(&Session) -> bool| all.iter().find(|(s, m)| m.is_some() && f(s)).and_then(|(_, m)| m.clone());
    session
        .and_then(|id| pick(&|s| s.id == id))
        .or_else(|| pick(&|s| s.project == here))
        .or_else(|| pick(&|_| true))
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
        assert_eq!(
            arguments(Agent::Claude, Some("s1"), None, "go on", out, &[]),
            ["-p", "--output-format", "stream-json", "--verbose", "--permission-mode", "auto", "--resume", "s1", "--", "go on"]
        );
        let allowed = arguments(Agent::Claude, Some("s1"), None, "go on", out, &["Bash(npm install)".into()]);
        assert_eq!(&allowed[6..8], ["--allowedTools", "Bash(npm install)"]);
        let codex = arguments(Agent::Codex, Some("t1"), None, "go on", out, &[]);
        assert_eq!(&codex[..2], ["exec", "resume"]);
        assert_eq!(&codex[codex.len() - 3..], ["t1", "--", "go on"]);
        assert!(codex.contains(&"sandbox_mode=\"workspace-write\"".to_string()));
        assert!(arguments(Agent::Codex, None, None, "hi", out, &[FULL_ACCESS.into()]).contains(&"sandbox_mode=\"danger-full-access\"".to_string()));
        assert_eq!(arguments(Agent::Codex, None, None, "hi", out, &[])[1], "--skip-git-repo-check");
        assert_eq!(
            arguments(Agent::OpenCode, Some("ses_1"), Some("bedrock/gpt#medium"), "go on", out, &[]),
            ["run", "--format", "json", "--auto", "--session", "ses_1", "--model", "bedrock/gpt#medium", "--", "go on"]
        );
    }

    #[test]
    fn reads_how_a_run_ended() {
        let ok = claude_outcome(r#"{"type":"result","is_error":false,"result":"Fixed the zoom.","session_id":"s9","permission_denials":[{"tool_name":"Bash","tool_input":{"command":"npm install"}}]}"#).unwrap();
        assert!(ok.ok && ok.answer == "Fixed the zoom." && ok.session.as_deref() == Some("s9"));
        let (needs, allow) = needs_of(&ok, None);
        assert!(needs.unwrap().contains("ran npm install") && allow == ["Bash(npm install)"]);
        let login = claude_outcome(r#"{"type":"result","is_error":true,"result":"Failed to authenticate: OAuth session expired"}"#).unwrap();
        assert!(!login.ok && login_hint(Agent::Claude, &login.answer).contains("/login"));
        assert!(claude_outcome("not json").is_none());

        let printed = "{\"type\":\"thread.started\",\"thread_id\":\"t7\"}\n{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"OK\"}}\n";
        let c = codex_outcome(printed, None).unwrap();
        assert!(c.ok && c.answer == "OK" && c.session.as_deref() == Some("t7"));
        assert_eq!(codex_outcome(printed, Some("Done, all tests pass.\n")).unwrap().answer, "Done, all tests pass.");
        let failed = codex_outcome("{\"type\":\"turn.failed\",\"error\":{\"message\":\"quota\"}}", None).unwrap();
        assert!(!failed.ok && failed.answer == "quota");

        let printed = concat!(
            r#"{"type":"step_start","sessionID":"ses_4","part":{"messageID":"m1"}}"#, "\n",
            r#"{"type":"text","sessionID":"ses_4","part":{"messageID":"m1","text":"Let me look."}}"#, "\n",
            r#"{"type":"text","sessionID":"ses_4","part":{"messageID":"m2","text":"Done."}}"#, "\n",
        );
        let o = opencode_outcome(printed).unwrap();
        assert!(o.ok && o.answer == "Done." && o.session.as_deref() == Some("ses_4"));
        let failed = opencode_outcome(r#"{"type":"error","sessionID":"ses_5","error":{"type":"provider","message":"bad model"}}"#).unwrap();
        assert!(!failed.ok && failed.answer == "bad model");
    }

    #[test]
    fn whats_active_now() {
        let now = 10_000_000;
        let run = |id, session: Option<&str>, project: &str, state, started, ended| Run {
            id,
            agent: Agent::Claude,
            project: project.into(),
            session: session.map(str::to_string),
            prompt: format!("job {id}"),
            state,
            result: String::new(),
            steps: vec![format!("step of {id}")],
            needs: None,
            allow: Vec::new(),
            language: "en",
            started,
            ended,
        };
        let session = |agent, id: &str, project: &str, updated| Session { agent, id: id.into(), title: format!("chat {id}"), project: project.into(), updated };
        let runs = [
            run(1, None, "/p/yoru", "running", now - 5_000, None),
            run(2, Some("s2"), "/p/maps", "done", now - 200_000, Some(now - 60_000)),
            run(3, Some("s3"), "/p/old", "done", now - 9_000_000, Some(now - 8_000_000)),
        ];
        let recent = vec![
            session(Agent::Claude, "s1", "/p/yoru", now - 1_000),
            session(Agent::Claude, "s2", "/p/maps", now - 60_000),
            session(Agent::Codex, "x9", "/p/yoru", now - 2_000),
            session(Agent::OpenCode, "o4", "/p/marea", now - 30_000),
        ];
        let list = active_list(&runs, recent, now);
        let seen: Vec<(Option<u64>, Option<&str>, &str)> = list.iter().map(|a| (a.run, a.session.as_deref(), a.state)).collect();
        // Yoru's run 1 stands for the new conversation s1; run 2 for s2; run 3 ended long ago.
        assert_eq!(seen, [(None, Some("x9"), "working"), (Some(1), None, "working"), (None, Some("o4"), "working"), (Some(2), Some("s2"), "done")]);
        assert_eq!(list[1].now.as_deref(), Some("step of 1"));
    }

    fn ended(state: &'static str, result: &str, language: &'static str) -> Run {
        Run {
            id: 1,
            agent: Agent::Claude,
            project: "/Users/x/Code/RomeMaps".into(),
            session: Some("s1".into()),
            prompt: "fix it".into(),
            state,
            result: result.into(),
            steps: Vec::new(),
            needs: None,
            allow: Vec::new(),
            language,
            started: 0,
            ended: Some(1),
        }
    }

    #[test]
    fn says_how_a_run_ended() {
        let done = ended("done", "## Done\n\nFixed the **zoom** in [map.ts](src/map.ts).\n\n```ts\nzoom(2)\n```", "it");
        assert_eq!(announcement(&done).unwrap(), "Claude Code ha finito in RomeMaps: Done Fixed the zoom in map.ts.");
        let long = ended("done", &"This sentence is quite long. ".repeat(20), "en");
        let said = announcement(&long).unwrap();
        assert!(said.ends_with("The rest is in the inbox.") && said.len() < 340, "{said}");
        let mut asking = ended("needs_you", "I found two maps. Which one should I change, the old or the new?", "it");
        assert_eq!(announcement(&asking).unwrap(), "Claude Code ti chiede in RomeMaps: Which one should I change, the old or the new?");
        asking.allow = vec!["Bash(npm install)".into()];
        assert_eq!(announcement(&asking).unwrap(), "Claude Code si è fermato in RomeMaps: chiede il permesso per npm install. Vuoi che lo lasci fare?");
        assert!(announcement(&ended("stopped", "", "it")).is_none());
        let mut fresh = ended("failed", "Failed to authenticate\n\nClaude Code needs a login: run claude.", "en");
        fresh.project = "/Users/x/Documents/Yoru Code/2026-11-13/tell-a-joke".into();
        assert_eq!(announcement(&fresh).unwrap(), "Claude Code didn't start: it needs a login. Failed to authenticate");
    }

    #[test]
    fn questions_at_the_end() {
        assert_eq!(question_in("Done. Want me to commit it?").as_deref(), Some("Want me to commit it?"));
        assert_eq!(question_in("**Shall I push?**").as_deref(), Some("Shall I push?"));
        assert!(question_in("Why? Because it works.").is_none());
        let o = Outcome { ok: true, answer: "Should I go on?".into(), session: None, denied: Vec::new() };
        assert_eq!(needs_of(&o, None), (Some("an answer to its question".into()), Vec::new()));
        let plain = Outcome { ok: true, answer: "All done.".into(), session: None, denied: Vec::new() };
        assert_eq!(needs_of(&plain, Some("rm -rf ~/x")).1, [FULL_ACCESS]);
        assert_eq!(needs_of(&plain, None), (None, Vec::new()));
    }

    #[test]
    fn follows_a_run_as_it_goes() {
        let init = progress(Agent::Claude, &json!({"type": "system", "subtype": "init", "session_id": "s5"}));
        assert_eq!(init.session.as_deref(), Some("s5"));
        let step = progress(Agent::Claude, &json!({"type": "assistant", "parent_tool_use_id": null, "message": {"content": [{"type": "tool_use", "name": "Edit", "input": {"file_path": "/a/b.rs"}}]}}));
        assert_eq!(step.steps, ["edited b.rs"]);
        let codex = progress(Agent::Codex, &json!({"type": "item.started", "item": {"type": "command_execution", "command": "bash -lc 'cargo test'"}}));
        assert_eq!(codex.steps, ["ran bash -lc 'cargo test'"]);
        let blocked = progress(Agent::Codex, &json!({"type": "item.completed", "item": {"type": "command_execution", "command": "touch /etc/x", "exit_code": 1, "aggregated_output": "touch: /etc/x: Operation not permitted"}}));
        assert_eq!(blocked.blocked.as_deref(), Some("touch /etc/x"));
        let oc = progress(Agent::OpenCode, &json!({"type": "tool_use", "sessionID": "ses_1", "part": {"type": "tool", "tool": "bash", "state": {"input": {"command": "ls"}}}}));
        assert_eq!((oc.session.as_deref(), oc.steps.as_slice()), (Some("ses_1"), ["ran ls".to_string()].as_slice()));
        let mut steps = Vec::new();
        for s in ["a", "a", "b", "c", "d", "e", "f", "g"] {
            push_step(&mut steps, s.into());
        }
        assert_eq!(steps, ["b", "c", "d", "e", "f", "g"]);
    }

    #[test]
    fn the_tools_are_four() {
        let tools = Coding::tools();
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(names, TOOLS);
        let coding = Coding::new(Arc::new(Inbox::open(None)));
        assert!(coding.call_tool("code_send", &json!({})).unwrap_err().contains("prompt"));
        assert!(coding.call_tool("code_control", &json!({"action": "approve", "run": 4})).unwrap_err().contains("no run 4"));
        assert!(coding.call_tool("code_control", &json!({"action": "stop", "run": 4})).unwrap_err().contains("isn't running"));
    }

    #[test]
    fn reads_opencodes_conversations() {
        let h = home("opencode");
        let path = opencode_db(&h);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        for table in ["session_v2", "session"] {
            db.execute_batch(&format!(
                "CREATE TABLE {table} (id TEXT, parent_id TEXT, title TEXT, directory TEXT, time_updated INTEGER, time_archived INTEGER, model TEXT)"
            ))
            .unwrap();
        }
        db.execute_batch(
            r#"INSERT INTO session_v2 VALUES ('ses_a', NULL, 'Home screen times', '/Users/x/RomeMaps', 300, NULL, '{"id":"gpt-6","providerID":"bedrock","variant":"medium"}');
               INSERT INTO session_v2 VALUES ('ses_b', 'ses_a', 'a subagent', '/Users/x/RomeMaps', 400, NULL, NULL);
               INSERT INTO session_v2 VALUES ('ses_c', NULL, 'Disk space', '/Users/x', 200, NULL, '{"id":"sol","providerID":"bedrock"}');
               INSERT INTO session VALUES ('ses_d', NULL, 'Old one', '/Users/x/Marea', 100, NULL, NULL);
               INSERT INTO session VALUES ('ses_a', NULL, 'Home screen times', '/Users/x/RomeMaps', 300, NULL, NULL);"#,
        )
        .unwrap();
        drop(db);
        let all = sessions(&h, Some(Agent::OpenCode));
        let ids: Vec<&str> = all.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["ses_a", "ses_c", "ses_d"]);
        assert_eq!(opencode_model(&h, Some("ses_c"), Path::new("/Users/x/RomeMaps")).as_deref(), Some("bedrock/sol"));
        assert_eq!(opencode_model(&h, None, Path::new("/Users/x/RomeMaps")).as_deref(), Some("bedrock/gpt-6#medium"));
        assert_eq!(opencode_model(&h, None, Path::new("/Users/x/New")).as_deref(), Some("bedrock/gpt-6#medium"));
        fs::remove_dir_all(&h).unwrap();
    }

    /// This Mac's conversations, and a real prompt to an agent in a fresh folder
    /// (`CODE_SEND=codex` or `opencode`): `cargo test -- --ignored this_macs --nocapture`.
    #[test]
    #[ignore]
    fn this_macs_conversations() {
        let home = crate::paths::home_dir().unwrap();
        let all = sessions(&home, None);
        for s in matching(&all, "") {
            println!("{:?} {} | {} | {}", s.agent, s.id, s.title, s.project);
        }
        println!("projects: {:?}", project_folder(&home, "dualeye"));
        let coding = Coding::new(Arc::new(Inbox::open(None)));
        println!("status: {}", coding.call_tool("code_status", &json!({})).unwrap());
        for s in all.iter().filter(|s| s.agent != Agent::Claude).take(2) {
            println!("{:?} {}: {}", s.agent, s.id, coding.call_tool("code_status", &json!({"session": s.id})).unwrap_or_else(|e| e));
        }
        if std::env::var("CODE_SEND").is_ok() {
            let inbox = Arc::new(Inbox::open(None));
            let coding = Coding::new(inbox.clone());
            println!("{}", coding.call_tool("code_send", &json!({"prompt": "Create hello.txt containing the word ciao, then reply: done", "agent": std::env::var("CODE_SEND").unwrap_or_default()})).unwrap());
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
