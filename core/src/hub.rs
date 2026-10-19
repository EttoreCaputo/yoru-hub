//! Share the board between processes. Only one process can hold the serial
//! port, and while the app or `yoru` runs, the bridge holds it. Its
//! [`Hub`] lets others (`yoru mcp`, `yoru call`) use the board through
//! the bridge instead of opening the port a second time. [`Board`] is the
//! other side: it goes through a running hub, and opens the port for the call
//! itself when there is none.
//!
//! The hub listens on 127.0.0.1, on a port the OS picks, and leaves the port
//! and a random token in `hub.json` in the data folder, readable by this
//! user only. A client's first request is `hello` with that token. Then, one
//! JSON-RPC 2.0 message per line, a request at a time:
//!
//! | Method | Result |
//! |--------|--------|
//! | `hello` `{"token","client"}` | `{"bridge", "boards": [BoardInfo], "board": Hello or null, "port"}` (`board` and `port` for older clients: the board a call without `board` would reach) |
//! | `host/boards` | `[BoardInfo]`: the boards attached |
//! | `host/hello` | The `hello` of the board the request reaches |
//! | `tools/list`, `tools/call`, `media/...`, `wifi/...` | Passed to the board as they are |
//! | `host/snapshot` | `{"snapshot": Snapshot or null, "age_ms"}`: the bridge's latest sample |
//! | `host/say` `{"text","language"?}` | How it was spoken, once played |
//! | `host/claude_hook` (a Claude Code hook's JSON) | `{"taken"}`: whether a bridge takes alerts |
//! | `host/claude_permission` (a `PermissionRequest` hook's JSON) | `{"decision": "allow" \| "deny" \| null}`, once answered in the app or on the board; `null` at once without a bridge or with approvals off |
//! | `host/timers` `{"name","arguments","language"?}` | `{"text","is_error"}`: a [`Timers::tools`] tool, run on the bridge's timers |
//! | `host/notify` `{"title","text"?,"urgency"?,"source"?,"url"?}` | `{"id"}`: a notification in the assistant's inbox, delivered to the board |
//!
//! With several boards attached, a request for a board (`tools/...`,
//! `media/...`, `wifi/...`, `host/say`) takes `"board"` in its params: a
//! board's id, or its kind (`yoru`) when only one of that kind is
//! there. Without it the request goes to the only board, else to the only
//! board with the tool called, else to the primary board; otherwise it fails
//! with [`SEVERAL_BOARDS`]. `tools/list` without `board` lists every board's
//! tools, each name once.

use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::claude::agents::Decision;
use crate::claude::alerts::MAX_APPROVAL_WAIT_S;
use crate::link::{CallError, Hello, Link, Tool, ToolResult};
use crate::profile::BoardKind;
use crate::serial;
use crate::snapshot::Snapshot;
use crate::timers::{self, Timers};

/// Reply for a request the hub can't pass on because no board is attached.
pub const NO_BOARD: i64 = -32000;
const UNAUTHORIZED: i64 = -32001;
/// Reply for a request that could go to more than one board: pass `board`.
pub const SEVERAL_BOARDS: i64 = -32002;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_REQUEST: i64 = -32600;
const INVALID_PARAMS: i64 = -32602;
/// `host/say` answers once the board has spoken.
const SAY_TIMEOUT: Duration = Duration::from_secs(90);

/// How long the board gets for one tool call.
pub const TOOL_TIMEOUT: Duration = Duration::from_secs(3);
/// A WiFi scan (`wifi/scan`) goes over every channel.
pub const SCAN_TIMEOUT: Duration = Duration::from_secs(12);
/// A client waits this long for the hub, which waits [`TOOL_TIMEOUT`] for the board.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(100);
/// Tools whose effect the bridge mirrors in its settings.
const SETTINGS_TOOLS: [&str; 2] = ["set_face", "set_rotation"];

/// Where a running hub says how to reach it.
pub fn hub_file() -> Option<PathBuf> {
    crate::claude::data_dir().map(|d| d.join("hub.json"))
}

#[derive(Serialize, Deserialize)]
struct HubFile {
    port: u16,
    token: String,
    pid: u32,
}

/// What a frontend shows about the processes using the board through the hub.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HubStatus {
    /// Clients connected now (each MCP server keeps one open).
    pub clients: usize,
    /// Tool calls passed to the board since the hub started.
    pub calls: u64,
    pub last_tool: Option<String>,
    pub last_call_age_ms: Option<u64>,
}

/// What a client is told about an attached board.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardInfo {
    /// From its `hello`; `None` before firmware that says it.
    pub id: Option<String>,
    pub kind: BoardKind,
    /// The serial port, or where it is on WiFi.
    pub port: String,
    pub firmware: String,
    /// The board calls without `board` go to, when several could.
    #[serde(default)]
    pub primary: bool,
}

impl BoardInfo {
    /// Whether `wanted` (an id, or a kind) names this board.
    pub fn is(&self, wanted: &str) -> bool {
        self.id.as_deref() == Some(wanted) || BoardKind::from_slug(wanted) == Some(self.kind)
    }
}

/// A board clients reach through its session's link.
struct Attached {
    link: Arc<Link>,
    hello: Hello,
    kind: BoardKind,
    port: String,
    /// Speaks through the board, when the bridge has text-to-speech.
    say: SayFn,
    /// Set when a client changed a face or a rotation: the session reads them back.
    settings_changed: Arc<AtomicBool>,
    /// One request at a time: the board drops input it has no room for.
    one_at_a_time: Mutex<()>,
    /// Its tools, once asked: where a call without `board` goes.
    tools: Mutex<Option<Vec<Tool>>>,
}

impl Attached {
    fn info(&self, primary: Option<&str>) -> BoardInfo {
        let id = self.hello.id.clone();
        let primary = primary.is_some() && id.as_deref() == primary;
        BoardInfo { id, kind: self.kind, port: self.port.clone(), firmware: self.hello.firmware.clone(), primary }
    }

    fn call(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, CallError> {
        let _one = self.one_at_a_time.lock().unwrap();
        self.link.call(method, params, timeout)
    }

    /// Its tools, asked once per attach.
    fn tools(&self) -> Vec<Tool> {
        if let Some(tools) = &*self.tools.lock().unwrap() {
            return tools.clone();
        }
        let listed = {
            let _one = self.one_at_a_time.lock().unwrap();
            self.link.list_tools(TOOL_TIMEOUT)
        };
        match listed {
            Ok(tools) => self.tools.lock().unwrap().insert(tools).clone(),
            Err(_) => Vec::new(),
        }
    }
}

struct Shared {
    token: String,
    stop: AtomicBool,
    boards: Mutex<Vec<Arc<Attached>>>,
    /// Why there is no board, while there is none.
    unavailable: Mutex<String>,
    /// The id of the primary board, from the bridge's config.
    primary: Mutex<Option<Arc<Mutex<Option<String>>>>>,
    snapshot: Mutex<Option<(Snapshot, Instant)>>,
    /// Takes Claude Code's hook events, while a bridge runs.
    claude_hook: Mutex<Option<HookFn>>,
    /// Answers Claude Code's permission requests, while a bridge runs.
    claude_permission: Mutex<Option<PermissionFn>>,
    /// The bridge's timers, while it runs.
    timers: Mutex<Option<Arc<Timers>>>,
    /// The assistant, whose inbox `host/notify` posts to, while a bridge runs.
    assistant: Mutex<Option<Arc<crate::Assistant>>>,
    clients: AtomicUsize,
    calls: AtomicU64,
    last_call: Mutex<Option<(String, Instant)>>,
}

/// The listening end, owned by the frontend and handed to its bridge through
/// [`crate::BridgeConfig::hub`]. It outlives bridge restarts, so clients keep
/// being told the board is busy while esptool holds the port, rather than
/// opening it themselves. Dropping it stops listening and removes `hub.json`.
pub struct Hub {
    shared: Arc<Shared>,
    file: Option<PathBuf>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Hub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hub").field("file", &self.file).finish_non_exhaustive()
    }
}

impl Hub {
    pub fn start() -> io::Result<Arc<Hub>> {
        Self::start_at(hub_file())
    }

    fn start_at(file: Option<PathBuf>) -> io::Result<Arc<Hub>> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let mut token = [0u8; 16];
        getrandom::fill(&mut token).map_err(|e| io::Error::other(e.to_string()))?;
        let token: String = token.iter().map(|b| format!("{b:02x}")).collect();
        if let Some(path) = &file {
            let info = HubFile { port, token: token.clone(), pid: std::process::id() };
            write_private(path, &serde_json::to_vec(&info).expect("hub file serializes"))?;
        }
        let shared = Arc::new(Shared {
            token,
            stop: AtomicBool::new(false),
            boards: Mutex::default(),
            unavailable: Mutex::new("the bridge has not found the board yet".into()),
            primary: Mutex::default(),
            snapshot: Mutex::default(),
            claude_hook: Mutex::default(),
            claude_permission: Mutex::default(),
            timers: Mutex::default(),
            assistant: Mutex::default(),
            clients: AtomicUsize::new(0),
            calls: AtomicU64::new(0),
            last_call: Mutex::default(),
        });
        let thread = {
            let shared = shared.clone();
            thread::Builder::new().name("yoru-hub".into()).spawn(move || accept_loop(&listener, &shared))?
        };
        Ok(Arc::new(Hub { shared, file, thread: Some(thread) }))
    }

    pub fn status(&self) -> HubStatus {
        let last = self.shared.last_call.lock().unwrap().clone();
        HubStatus {
            clients: self.shared.clients.load(Ordering::Relaxed),
            calls: self.shared.calls.load(Ordering::Relaxed),
            last_tool: last.as_ref().map(|(tool, _)| tool.clone()),
            last_call_age_ms: last.map(|(_, at)| at.elapsed().as_millis() as u64),
        }
    }

    /// Every board goes: clients get `reason` as why their board calls fail
    /// until a board is attached again.
    pub fn set_unavailable(&self, reason: &str) {
        self.shared.boards.lock().unwrap().clear();
        *self.shared.unavailable.lock().unwrap() = reason.to_string();
    }

    /// Clients reach the board on `port` through `link`, and speak with
    /// `say`. A client changing its faces or rotation sets `settings_changed`.
    /// It replaces what was attached on that port, or with that id.
    pub(crate) fn attach(&self, link: Arc<Link>, hello: Hello, port: &str, say: SayFn, settings_changed: Arc<AtomicBool>) {
        let kind = BoardKind::from_hello(&hello);
        let mut boards = self.shared.boards.lock().unwrap();
        boards.retain(|a| a.port != port && (hello.id.is_none() || a.hello.id != hello.id));
        boards.push(Arc::new(Attached {
            link,
            hello,
            kind,
            port: port.to_string(),
            say,
            settings_changed,
            one_at_a_time: Mutex::default(),
            tools: Mutex::default(),
        }));
    }

    /// The board on `port` is gone (`reason`): once none is left, clients
    /// are told so until another is attached.
    pub(crate) fn detach(&self, port: &str, reason: &str) {
        let mut boards = self.shared.boards.lock().unwrap();
        boards.retain(|a| a.port != port);
        if boards.is_empty() {
            *self.shared.unavailable.lock().unwrap() = reason.to_string();
        }
    }

    /// Where the id of the primary board is kept; `None` without a bridge.
    pub(crate) fn set_primary(&self, primary: Option<Arc<Mutex<Option<String>>>>) {
        *self.shared.primary.lock().unwrap() = primary;
    }

    /// What `host/claude_hook` hands its event to; `None` without a bridge.
    pub(crate) fn set_claude_hook(&self, hook: Option<HookFn>) {
        *self.shared.claude_hook.lock().unwrap() = hook;
    }

    /// What `host/claude_permission` asks; `None` without a bridge.
    pub(crate) fn set_claude_permission(&self, ask: Option<PermissionFn>) {
        *self.shared.claude_permission.lock().unwrap() = ask;
    }

    /// The assistant `host/notify` posts to; `None` without a bridge.
    pub(crate) fn set_assistant(&self, assistant: Option<Arc<crate::Assistant>>) {
        *self.shared.assistant.lock().unwrap() = assistant;
    }

    /// The timers `host/timers` works on; `None` without a bridge.
    pub(crate) fn set_timers(&self, timers: Option<Arc<Timers>>) {
        *self.shared.timers.lock().unwrap() = timers;
    }

    pub(crate) fn set_snapshot(&self, snapshot: &Snapshot) {
        *self.shared.snapshot.lock().unwrap() = Some((snapshot.clone(), Instant::now()));
    }

    #[cfg(test)]
    fn addr(&self) -> SocketAddr {
        let info: HubFile = serde_json::from_slice(&fs::read(self.file.as_ref().unwrap()).unwrap()).unwrap();
        SocketAddr::from((Ipv4Addr::LOCALHOST, info.port))
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        // Leave a newer hub's file alone.
        if let Some(path) = &self.file
            && read_hub_file(path).is_some_and(|f| f.token == self.shared.token)
        {
            let _ = fs::remove_file(path);
        }
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)
}

fn read_hub_file(path: &Path) -> Option<HubFile> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn accept_loop(listener: &TcpListener, shared: &Arc<Shared>) {
    while !shared.stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let shared = shared.clone();
                let _ = thread::Builder::new().name("yoru-hub-client".into()).spawn(move || {
                    shared.clients.fetch_add(1, Ordering::Relaxed);
                    let _ = serve_client(stream, &shared);
                    shared.clients.fetch_sub(1, Ordering::Relaxed);
                });
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::sleep(POLL),
            Err(_) => thread::sleep(POLL),
        }
    }
}

fn serve_client(stream: TcpStream, shared: &Shared) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(POLL * 2))?;
    let mut out = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let mut line = Vec::new();
    let mut authorized = false;
    while !shared.stop.load(Ordering::Relaxed) {
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => return Ok(()),
            Ok(_) if line.ends_with(b"\n") => {}
            Ok(_) => return Ok(()),
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => continue,
            Err(e) => return Err(e),
        }
        let request: Option<Value> = serde_json::from_slice(&line).ok();
        line.clear();
        let Some(request) = request else { continue };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        let reply = if method == "hello" {
            authorized = params.get("token").and_then(Value::as_str) == Some(shared.token.as_str());
            if authorized { Ok(hello(shared)) } else { Err((UNAUTHORIZED, "wrong hub token".to_string())) }
        } else if !authorized {
            Err((UNAUTHORIZED, "send hello first".to_string()))
        } else {
            handle(shared, method, params)
        };
        let message = match reply {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err((code, message)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
        };
        let mut bytes = message.to_string().into_bytes();
        bytes.push(b'\n');
        out.write_all(&bytes)?;
        if !authorized {
            return Ok(());
        }
    }
    Ok(())
}

fn hello(shared: &Shared) -> Value {
    // Older clients know one board: the one a call without `board` reaches.
    let (hello, port) = match shared.pick(None, None) {
        Ok(a) => (serde_json::to_value(&a.hello).unwrap_or(Value::Null), Some(a.port.clone())),
        Err(_) => (Value::Null, None),
    };
    json!({"bridge": env!("CARGO_PKG_VERSION"), "boards": shared.infos(), "board": hello, "port": port})
}

impl Shared {
    fn primary(&self) -> Option<String> {
        self.primary.lock().unwrap().as_ref().and_then(|p| p.lock().unwrap().clone())
    }

    fn infos(&self) -> Vec<BoardInfo> {
        let primary = self.primary();
        self.boards.lock().unwrap().iter().map(|a| a.info(primary.as_deref())).collect()
    }

    /// The board a request goes to: the one `wanted` names, or as [`pick`] says.
    fn pick(&self, wanted: Option<&str>, tool: Option<&str>) -> Result<Arc<Attached>, (i64, String)> {
        let boards = self.boards.lock().unwrap().clone();
        if boards.is_empty() {
            return Err((NO_BOARD, self.unavailable.lock().unwrap().clone()));
        }
        let infos: Vec<BoardInfo> = boards.iter().map(|a| a.info(None)).collect();
        let has_tool = |i: usize| tool.is_some_and(|t| boards[i].tools().iter().any(|x| x.name == t));
        pick(&infos, wanted, has_tool, self.primary().as_deref()).map(|i| boards[i].clone())
    }
}

/// Which of `boards` (not empty) a request goes to: the one `wanted` names
/// (an id, or the only board of a kind); without `wanted`, the only board,
/// else the only one with the tool, else the `primary` one.
fn pick(boards: &[BoardInfo], wanted: Option<&str>, has_tool: impl Fn(usize) -> bool, primary: Option<&str>) -> Result<usize, (i64, String)> {
    let names = || boards.iter().map(|b| b.id.clone().unwrap_or_else(|| b.kind.profile().slug.to_string())).collect::<Vec<_>>().join(", ");
    if let Some(wanted) = wanted {
        let by_id: Vec<usize> = (0..boards.len()).filter(|&i| boards[i].id.as_deref() == Some(wanted)).collect();
        let found: Vec<usize> = if by_id.is_empty() { (0..boards.len()).filter(|&i| boards[i].is(wanted)).collect() } else { by_id };
        return match found[..] {
            [i] => Ok(i),
            [] => Err((NO_BOARD, format!("no board `{wanted}` is connected (there is {})", names()))),
            _ => Err((SEVERAL_BOARDS, format!("several {wanted} boards: pass `board` with one's id ({})", names()))),
        };
    }
    if boards.len() == 1 {
        return Ok(0);
    }
    let with_tool: Vec<usize> = (0..boards.len()).filter(|&i| has_tool(i)).collect();
    if let [i] = with_tool[..] {
        return Ok(i);
    }
    if let Some(i) = primary.and_then(|p| boards.iter().position(|b| b.id.as_deref() == Some(p))) {
        return Ok(i);
    }
    Err((SEVERAL_BOARDS, format!("several boards: pass `board` ({})", names())))
}

/// Speak `text` (in `language`, or the one it looks like) and return how it went.
pub(crate) type SayFn = Arc<dyn Fn(&str, Option<&str>) -> Result<Value, String> + Send + Sync>;

/// Take a Claude Code hook's JSON; it must not wait on the board.
pub(crate) type HookFn = Arc<dyn Fn(Value) + Send + Sync>;

/// Hold a `PermissionRequest` hook's JSON until it is answered, or give up.
pub(crate) type PermissionFn = Arc<dyn Fn(Value) -> Option<Decision> + Send + Sync>;

fn handle(shared: &Shared, method: &str, mut params: Value) -> Result<Value, (i64, String)> {
    // Which board, for the hub only: the board itself isn't told.
    let wanted = params.as_object_mut().and_then(|p| p.remove("board")).and_then(|b| b.as_str().map(str::to_string));
    let wanted = wanted.as_deref();
    match method {
        "tools/list" => return list_tools(shared, wanted),
        "tools/call" => {}
        // Image uploads (firmware 1.1), writes taking a moment when the flash
        // erases; the WiFi set-up (firmware 1.4.3), a scan taking seconds.
        m if m.starts_with("media/") || m.starts_with("wifi/") => {}
        "host/say" => {
            let text = params.get("text").and_then(Value::as_str).filter(|t| !t.trim().is_empty());
            let Some(text) = text else { return Err((INVALID_PARAMS, "expected {\"text\": string, \"language\"?: \"it\" | \"en\"}".into())) };
            let board = shared.pick(wanted, None)?;
            return (board.say)(text, params.get("language").and_then(Value::as_str)).map_err(|e| (NO_BOARD, e));
        }
        "host/boards" => return Ok(json!(shared.infos())),
        "host/hello" => return Ok(serde_json::to_value(&shared.pick(wanted, None)?.hello).unwrap_or(Value::Null)),
        "host/claude_hook" => {
            let hook = shared.claude_hook.lock().unwrap().clone();
            let taken = hook.is_some();
            if let Some(hook) = hook {
                hook(params);
            }
            return Ok(json!({"taken": taken}));
        }
        "host/claude_permission" => {
            // Each client has its own thread: this one may wait minutes.
            let ask = shared.claude_permission.lock().unwrap().clone();
            let decision = ask.and_then(|ask| ask(params));
            return Ok(json!({"decision": decision}));
        }
        "host/timers" => {
            let timers = shared.timers.lock().unwrap().clone().ok_or((NO_BOARD, "the bridge isn't running".to_string()))?;
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let language = params.get("language").and_then(Value::as_str).unwrap_or("en");
            return match timers.call_tool(name, &arguments, language) {
                Some(Ok(text)) => Ok(json!({"text": text, "is_error": false})),
                Some(Err(text)) => Ok(json!({"text": text, "is_error": true})),
                None => Err((INVALID_PARAMS, format!("unknown timer tool `{name}`"))),
            };
        }
        "host/notify" => {
            let assistant = shared.assistant.lock().unwrap().clone().ok_or((NO_BOARD, "the Yoru app isn't running".to_string()))?;
            let title = params.get("title").and_then(Value::as_str).filter(|t| !t.trim().is_empty());
            let Some(title) = title else { return Err((INVALID_PARAMS, "expected {\"title\": string, ...}".into())) };
            let text = params.get("text").and_then(Value::as_str).unwrap_or_default();
            let source = params.get("source").and_then(Value::as_str).unwrap_or("Claude Code");
            let urgency = params.get("urgency").and_then(Value::as_str).and_then(crate::assistant::Urgency::parse).unwrap_or_default();
            let url = params.get("url").and_then(Value::as_str).map(str::to_string);
            let id = assistant.inbox.post(source, title, text, urgency, url, None);
            return Ok(json!({"id": id}));
        }
        "host/snapshot" => {
            let snapshot = shared.snapshot.lock().unwrap();
            let (snap, age) = match &*snapshot {
                Some((s, at)) => (serde_json::to_value(s).unwrap_or(Value::Null), Some(at.elapsed().as_millis() as u64)),
                None => (Value::Null, None),
            };
            return Ok(json!({"snapshot": snap, "age_ms": age}));
        }
        "" => return Err((INVALID_REQUEST, "no method".into())),
        other => return Err((METHOD_NOT_FOUND, format!("unknown method `{other}`"))),
    }
    let tool = (method == "tools/call").then(|| params.get("name").and_then(Value::as_str).unwrap_or("").to_string());
    let board = shared.pick(wanted, tool.as_deref())?;
    let timeout = if method == "wifi/scan" { SCAN_TIMEOUT } else { TOOL_TIMEOUT };
    let reply = board.call(method, params, timeout);
    if let Some(tool) = tool {
        shared.calls.fetch_add(1, Ordering::Relaxed);
        let ok = reply.as_ref().is_ok_and(|r| r.get("isError") != Some(&Value::Bool(true)));
        if ok && SETTINGS_TOOLS.contains(&tool.as_str()) {
            board.settings_changed.store(true, Ordering::Relaxed);
        }
        *shared.last_call.lock().unwrap() = Some((tool, Instant::now()));
    }
    reply.map_err(rpc_error)
}

/// `tools/list`: the board's, asked again; without `board` and with several
/// boards, every board's, each name once.
fn list_tools(shared: &Shared, wanted: Option<&str>) -> Result<Value, (i64, String)> {
    let several = shared.boards.lock().unwrap().len() > 1;
    if wanted.is_none() && several {
        let mut tools: Vec<Tool> = Vec::new();
        for board in shared.boards.lock().unwrap().clone() {
            for tool in board.tools() {
                if !tools.iter().any(|t| t.name == tool.name) {
                    tools.push(tool);
                }
            }
        }
        return Ok(json!({"tools": tools}));
    }
    let board = shared.pick(wanted, None)?;
    let reply = board.call("tools/list", json!({}), TOOL_TIMEOUT).map_err(rpc_error)?;
    if let Ok(tools) = serde_json::from_value::<Vec<Tool>>(reply["tools"].clone()) {
        *board.tools.lock().unwrap() = Some(tools);
    }
    Ok(reply)
}

fn rpc_error(e: CallError) -> (i64, String) {
    match e {
        CallError::Rpc { code, message } => (code, message),
        other => (NO_BOARD, other.to_string()),
    }
}

/// What `hello` told a client about the hub.
#[derive(Debug, Clone, Deserialize)]
pub struct HubHello {
    pub bridge: String,
    pub board: Option<Hello>,
    pub port: Option<String>,
    /// Every board attached; empty from a hub before several boards.
    #[serde(default)]
    pub boards: Vec<BoardInfo>,
}

/// One connection to a running hub.
struct HubClient {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    next_id: u64,
}

impl HubClient {
    /// `Ok(None)` when no hub is running.
    fn connect(file: &Path, client: &str) -> io::Result<Option<(HubClient, HubHello)>> {
        let Some(info) = read_hub_file(file) else { return Ok(None) };
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, info.port));
        // A hub that went away without cleaning up: its port refuses.
        let Ok(stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(300)) else { return Ok(None) };
        Self::handshake(stream, &info.token, client).map(Some)
    }

    fn handshake(stream: TcpStream, token: &str, client: &str) -> io::Result<(HubClient, HubHello)> {
        // The hub answers at once; a stale hub.json may point at someone else's port.
        stream.set_read_timeout(Some(Duration::from_secs(1)))?;
        stream.set_nodelay(true)?;
        let writer = stream.try_clone()?;
        let mut hub = HubClient { reader: BufReader::new(stream), writer, next_id: 1 };
        let hello = hub.request("hello", json!({"token": token, "client": client})).map_err(|e| io::Error::other(e.to_string()))?;
        let hello = serde_json::from_value(hello).map_err(io::Error::other)?;
        hub.writer.set_read_timeout(Some(CLIENT_TIMEOUT))?;
        Ok((hub, hello))
    }

    fn request_waiting(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value, CallError> {
        self.writer.set_read_timeout(Some(timeout))?;
        let reply = self.request(method, params);
        self.writer.set_read_timeout(Some(CLIENT_TIMEOUT))?;
        reply
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, CallError> {
        let id = self.next_id;
        self.next_id += 1;
        let mut bytes = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string().into_bytes();
        bytes.push(b'\n');
        self.writer.write_all(&bytes)?;
        loop {
            let mut line = Vec::new();
            match self.reader.read_until(b'\n', &mut line) {
                Ok(0) => return Err(CallError::Closed),
                Ok(_) => {}
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => return Err(CallError::Timeout),
                Err(e) => return Err(e.into()),
            }
            let msg: Value = serde_json::from_slice(&line).map_err(|e| CallError::Invalid(e.to_string()))?;
            // A reply to an earlier request that timed out.
            if msg.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(result) = msg.get("result") {
                return Ok(result.clone());
            }
            let err = msg.get("error").cloned().unwrap_or_default();
            let code = err.get("code").and_then(Value::as_i64).unwrap_or(0);
            let message = err.get("message").and_then(Value::as_str).unwrap_or("error").to_string();
            return Err(if code == NO_BOARD { CallError::Unavailable(message) } else { CallError::Rpc { code, message } });
        }
    }
}

/// How [`Board`] reached the board for its last call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    /// Through the app's or `yoru`'s bridge.
    Hub,
    /// The port, opened for that call only.
    Direct,
}

/// The board, for processes that don't stream to it: through the running
/// bridge when there is one, otherwise over the port, opened for each call
/// and closed right after so it stays free for the app.
pub struct Board {
    port: Option<String>,
    /// Which board, when several are attached: an id or a kind ([`Board::on`]).
    selector: Option<String>,
    client: String,
    file: Option<PathBuf>,
    hub: Mutex<Option<HubClient>>,
    route: Mutex<Option<Route>>,
}

impl Board {
    /// `port` is only used when no bridge runs; `None` auto-detects the board.
    /// `client` names this process to the hub (`yoru-mcp/0.1.0`).
    pub fn new(port: Option<String>, client: &str) -> Self {
        Self::with_hub_file(port, client, hub_file())
    }

    pub(crate) fn with_hub_file(port: Option<String>, client: &str, file: Option<PathBuf>) -> Self {
        Self { port, selector: None, client: client.to_string(), file, hub: Mutex::default(), route: Mutex::default() }
    }

    /// Reach the board `board` names (an id, or a kind) when several are
    /// attached; `None` leaves the choice to the hub.
    pub fn on(mut self, board: Option<String>) -> Self {
        self.selector = board;
        self
    }

    /// `params` with the board to reach, for the hub.
    fn aimed(&self, params: Value, on: Option<&str>) -> Value {
        match (params, on.or(self.selector.as_deref())) {
            (Value::Object(mut map), Some(board)) => {
                map.insert("board".into(), board.into());
                Value::Object(map)
            }
            (params, _) => params,
        }
    }

    pub fn last_route(&self) -> Option<Route> {
        *self.route.lock().unwrap()
    }

    /// The board's `hello`: as the running bridge last heard it, or asked
    /// over the port. It says which kind of board this is.
    pub fn hello(&self) -> Result<Hello, CallError> {
        if let Some(file) = &self.file
            && let Ok(Some((hub, hello))) = HubClient::connect(file, &self.client)
        {
            *self.hub.lock().unwrap() = Some(hub);
            *self.route.lock().unwrap() = Some(Route::Hub);
            if let Some(wanted) = &self.selector {
                // The hub's `board` is its own pick: ask for the one named.
                let found = hello.boards.iter().any(|b| b.is(wanted));
                if !found {
                    return Err(CallError::Unavailable(format!("no board `{wanted}` is connected")));
                }
                let mut slot = self.hub.lock().unwrap();
                let hub = self.hub_client(&mut slot).ok_or(CallError::Closed)?;
                let reply = hub.request("host/hello", self.aimed(json!({}), None));
                return reply.and_then(|v| serde_json::from_value(v).map_err(|e| CallError::Invalid(e.to_string())));
            }
            return hello.board.ok_or_else(|| CallError::Unavailable("the bridge has no board right now".into()));
        }
        *self.route.lock().unwrap() = Some(Route::Direct);
        self.open_direct().map(|(_, hello)| hello)
    }

    /// Which kind of board this is ([`Board::hello`]).
    pub fn kind(&self) -> Result<BoardKind, CallError> {
        self.hello().map(|h| BoardKind::from_hello(&h))
    }

    /// The boards a call can reach: those attached to the running bridge,
    /// or the one on the port.
    pub fn boards(&self) -> Result<Vec<BoardInfo>, CallError> {
        if let Some(boards) = self.attached() {
            return Ok(boards);
        }
        // A hub from before several boards, or the port.
        let hub = self.file.as_deref().and_then(|f| HubClient::connect(f, &self.client).ok().flatten());
        let (hello, port) = match hub {
            Some((_, hub)) => (hub.board.ok_or_else(|| CallError::Unavailable("the bridge has no board right now".into()))?, hub.port.unwrap_or_default()),
            None => {
                *self.route.lock().unwrap() = Some(Route::Direct);
                let (_, hello) = self.open_direct()?;
                (hello, self.direct_port().unwrap_or_default())
            }
        };
        let kind = BoardKind::from_hello(&hello);
        Ok(vec![BoardInfo { id: hello.id, kind, port, firmware: hello.firmware, primary: false }])
    }

    /// The boards attached to the running bridge; `None` without one, or
    /// with a hub from before several boards. Never opens the port.
    pub fn attached(&self) -> Option<Vec<BoardInfo>> {
        let mut slot = self.hub.lock().unwrap();
        let hub = self.hub_client(&mut slot)?;
        match hub.request("host/boards", json!({})) {
            Ok(list) => serde_json::from_value(list).ok(),
            Err(CallError::Rpc { .. } | CallError::Unavailable(_)) => None,
            Err(_) => {
                *slot = None;
                None
            }
        }
    }

    pub fn list_tools(&self) -> Result<Vec<Tool>, CallError> {
        self.list_tools_on(None)
    }

    /// The tools of board `on` (an id or a kind); `None` as [`Board::on`] said.
    pub fn list_tools_on(&self, on: Option<&str>) -> Result<Vec<Tool>, CallError> {
        #[derive(Deserialize)]
        struct List {
            tools: Vec<Tool>,
        }
        let list = self.run(|hub| hub.request("tools/list", self.aimed(json!({}), on)), |link| Ok(json!({"tools": link.list_tools(TOOL_TIMEOUT)?})))?;
        serde_json::from_value::<List>(list).map(|l| l.tools).map_err(|e| CallError::Invalid(e.to_string()))
    }

    pub fn call_tool(&self, name: &str, arguments: Value) -> Result<ToolResult, CallError> {
        self.call_tool_on(None, name, arguments)
    }

    /// Call `name` on board `on` (an id or a kind); `None` as [`Board::on`] said.
    pub fn call_tool_on(&self, on: Option<&str>, name: &str, arguments: Value) -> Result<ToolResult, CallError> {
        let params = self.aimed(json!({"name": name, "arguments": arguments}), on);
        let result = self.run(
            |hub| hub.request("tools/call", params.clone()),
            |link| Ok(serde_json::to_value(link.call_tool(name, arguments.clone(), TOOL_TIMEOUT)?).unwrap()),
        )?;
        serde_json::from_value(result).map_err(|e| CallError::Invalid(e.to_string()))
    }

    /// Speak through the board's speaker, in `language` (`it`, `en`) or the
    /// one the text looks like. Needs a bridge with text-to-speech (the app
    /// with spoken replies on, or `yoru --tts`). Returns once it's played.
    pub fn say(&self, text: &str, language: Option<&str>) -> Result<Value, CallError> {
        self.say_on(None, text, language)
    }

    /// [`Board::say`] through board `on` (an id or a kind).
    pub fn say_on(&self, on: Option<&str>, text: &str, language: Option<&str>) -> Result<Value, CallError> {
        let mut slot = self.hub.lock().unwrap();
        let hub = self
            .hub_client(&mut slot)
            .ok_or_else(|| CallError::Unavailable("speaking needs the Yoru app with spoken replies on, or yoru --tts, running".into()))?;
        *self.route.lock().unwrap() = Some(Route::Hub);
        let reply = hub.request_waiting("host/say", self.aimed(json!({"text": text, "language": language}), on), SAY_TIMEOUT);
        if matches!(reply, Err(CallError::Io(_) | CallError::Closed | CallError::Invalid(_))) {
            *slot = None;
        }
        reply
    }

    /// Run `job` with a way to send the board JSON-RPC requests (`media/...`
    /// for an image upload): through the bridge, or over the port, opened once
    /// for the whole job.
    pub fn session<T>(&self, job: impl FnOnce(&dyn Fn(&str, Value, Duration) -> Result<Value, CallError>) -> Result<T, CallError>) -> Result<T, CallError> {
        {
            let mut slot = self.hub.lock().unwrap();
            if let Some(hub) = self.hub_client(&mut slot) {
                *self.route.lock().unwrap() = Some(Route::Hub);
                let hub = std::cell::RefCell::new(hub);
                let result = job(&|method, params, timeout| hub.borrow_mut().request_waiting(method, self.aimed(params, None), timeout));
                if matches!(result, Err(CallError::Io(_) | CallError::Closed | CallError::Invalid(_))) {
                    *slot = None;
                }
                return result;
            }
        }
        *self.route.lock().unwrap() = Some(Route::Direct);
        let (link, _) = self.open_direct()?;
        job(&|method, params, timeout| link.call(method, params, timeout))
    }

    /// Hand a Claude Code hook's JSON to the running bridge, for its alerts.
    /// Only through the hub: without a bridge there is nobody to tell.
    pub fn claude_hook(&self, event: Value) -> Result<Value, CallError> {
        let mut slot = self.hub.lock().unwrap();
        let hub = self.hub_client(&mut slot).ok_or_else(|| CallError::Unavailable("the Yoru app isn't running".into()))?;
        *self.route.lock().unwrap() = Some(Route::Hub);
        hub.request("host/claude_hook", event)
    }

    /// Hand a `PermissionRequest` hook's JSON to the running bridge and wait
    /// for the answer from the app or the board: `None` leaves it to Claude
    /// Code's own dialog.
    pub fn claude_permission(&self, event: Value) -> Result<Option<Decision>, CallError> {
        let mut slot = self.hub.lock().unwrap();
        let hub = self.hub_client(&mut slot).ok_or_else(|| CallError::Unavailable("the app isn't running".into()))?;
        *self.route.lock().unwrap() = Some(Route::Hub);
        let wait = Duration::from_secs(u64::from(MAX_APPROVAL_WAIT_S) + 10);
        let reply = hub.request_waiting("host/claude_permission", event, wait)?;
        Ok(serde_json::from_value(reply["decision"].clone()).unwrap_or(None))
    }

    /// Run timer tool `name` ([`Timers::tools`]) on the running bridge's
    /// timers; without a bridge, on the ones kept on disk, which ring once
    /// the app or `yoru` runs again. `Err` is the tool's failure, as text.
    pub fn timer_tool(&self, name: &str, arguments: &Value, language: &str) -> Result<String, String> {
        {
            let mut slot = self.hub.lock().unwrap();
            if let Some(hub) = self.hub_client(&mut slot) {
                *self.route.lock().unwrap() = Some(Route::Hub);
                match hub.request("host/timers", json!({"name": name, "arguments": arguments, "language": language})) {
                    Ok(reply) => {
                        let text = reply["text"].as_str().unwrap_or_default().to_string();
                        return if reply["is_error"].as_bool() == Some(true) { Err(text) } else { Ok(text) };
                    }
                    // An older hub, or one whose bridge stopped: the file then.
                    Err(CallError::Rpc { code: METHOD_NOT_FOUND, .. } | CallError::Unavailable(_)) => {}
                    Err(CallError::Rpc { message, .. }) => return Err(message),
                    Err(_) => *slot = None,
                }
            }
        }
        *self.route.lock().unwrap() = Some(Route::Direct);
        let timers = Timers::open(timers::default_file());
        timers.call_tool(name, arguments, language).unwrap_or_else(|| Err(format!("unknown timer tool `{name}`")))
    }

    /// Put a notification in the running app's inbox (`host/notify`).
    pub fn notify(&self, params: Value) -> Result<String, CallError> {
        let mut slot = self.hub.lock().unwrap();
        let hub = self.hub_client(&mut slot).ok_or_else(|| CallError::Unavailable("the Yoru app isn't running".into()))?;
        let reply = hub.request("host/notify", params)?;
        Ok(reply["id"].as_str().unwrap_or_default().to_string())
    }

    /// The bridge's latest sample and its age; `None` without a bridge, or
    /// before it has sampled.
    pub fn bridge_snapshot(&self) -> Option<(Snapshot, Duration)> {
        let mut slot = self.hub.lock().unwrap();
        let hub = self.hub_client(&mut slot)?;
        let reply = hub.request("host/snapshot", json!({}));
        let reply = match reply {
            Ok(r) => r,
            Err(_) => {
                *slot = None;
                return None;
            }
        };
        let snapshot = serde_json::from_value(reply.get("snapshot")?.clone()).ok()?;
        Some((snapshot, Duration::from_millis(reply.get("age_ms")?.as_u64()?)))
    }

    fn hub_client<'a>(&self, slot: &'a mut Option<HubClient>) -> Option<&'a mut HubClient> {
        if slot.is_none() {
            *slot = HubClient::connect(self.file.as_deref()?, &self.client).ok().flatten().map(|(hub, _)| hub);
        }
        slot.as_mut()
    }

    fn run(
        &self,
        via_hub: impl Fn(&mut HubClient) -> Result<Value, CallError>,
        direct: impl FnOnce(&Link) -> Result<Value, CallError>,
    ) -> Result<Value, CallError> {
        {
            let mut slot = self.hub.lock().unwrap();
            // A second try for a connection the hub closed since the last call.
            for _ in 0..2 {
                let Some(hub) = self.hub_client(&mut slot) else { break };
                match via_hub(hub) {
                    Err(CallError::Io(_) | CallError::Closed | CallError::Invalid(_)) => *slot = None,
                    other => {
                        *self.route.lock().unwrap() = Some(Route::Hub);
                        return other;
                    }
                }
            }
        }
        *self.route.lock().unwrap() = Some(Route::Direct);
        direct(&self.open_direct()?.0)
    }

    /// The port a call without a bridge opens.
    fn direct_port(&self) -> Option<String> {
        self.port.clone().or_else(serial::detect_board).or_else(crate::wifi::default_target)
    }

    fn open_direct(&self) -> Result<(Link, Hello), CallError> {
        let port = self.direct_port().ok_or_else(|| CallError::Unavailable("board not found on USB".into()))?;
        let link = Link::open(&port, |_| {}).map_err(|e| CallError::Unavailable(format!("{port}: {e}")))?;
        let hello = link.handshake(Duration::from_secs(3)).map_err(|e| match e {
            CallError::Timeout => CallError::Unavailable(format!("{port}: the board does not answer (firmware 0.4 or later needed)")),
            other => other,
        })?;
        if let Some(wanted) = &self.selector {
            let kind = BoardKind::from_hello(&hello);
            let info = BoardInfo { id: hello.id.clone(), kind, port: port.clone(), firmware: hello.firmware.clone(), primary: false };
            if !info.is(wanted) {
                return Err(CallError::Unavailable(format!("{port} has a {kind} board, not `{wanted}`")));
            }
        }
        Ok((link, hello))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hub() -> (tempfile::TempDir, Arc<Hub>) {
        let dir = tempfile::tempdir().unwrap();
        let hub = Hub::start_at(Some(dir.path().join("hub.json"))).unwrap();
        (dir, hub)
    }

    fn client(dir: &tempfile::TempDir) -> HubClient {
        HubClient::connect(&dir.path().join("hub.json"), "test").unwrap().expect("hub running").0
    }

    #[test]
    fn needs_the_token() {
        let (_dir, hub) = hub();
        let stream = TcpStream::connect(hub.addr()).unwrap();
        let err = HubClient::handshake(stream, "not-the-token", "test").err().unwrap();
        assert!(err.to_string().contains("token"), "{err}");
    }

    #[test]
    fn without_a_board_calls_say_why() {
        let (dir, hub) = hub();
        let mut c = client(&dir);
        hub.set_unavailable("esptool is using the port");
        match c.request("tools/list", json!({})) {
            Err(CallError::Unavailable(why)) => assert_eq!(why, "esptool is using the port"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(c.request("nope", json!({})), Err(CallError::Rpc { code: METHOD_NOT_FOUND, .. })));
        assert_eq!(hub.status().clients, 1);
    }

    #[test]
    fn serves_the_latest_snapshot() {
        let (dir, hub) = hub();
        let mut c = client(&dir);
        assert_eq!(c.request("host/snapshot", json!({})).unwrap()["snapshot"], Value::Null);
        let snap: Snapshot = serde_json::from_value(json!({"v": 2, "ts": 5, "cpu": {"temp_c": 40.0}})).unwrap();
        hub.set_snapshot(&snap);
        let board = Board::with_hub_file(None, "test", Some(dir.path().join("hub.json")));
        let (got, age) = board.bridge_snapshot().unwrap();
        assert_eq!(got, snap);
        assert!(age < Duration::from_secs(1));
    }

    #[test]
    fn claude_hooks_reach_the_bridge() {
        let (dir, hub) = hub();
        let board = Board::with_hub_file(None, "test", Some(dir.path().join("hub.json")));
        assert_eq!(board.claude_hook(json!({"hook_event_name": "Stop"})).unwrap()["taken"], false);
        let got = Arc::new(Mutex::new(None));
        let sink = got.clone();
        hub.set_claude_hook(Some(Arc::new(move |v| *sink.lock().unwrap() = Some(v))));
        assert_eq!(board.claude_hook(json!({"hook_event_name": "Stop"})).unwrap()["taken"], true);
        assert_eq!(got.lock().unwrap().as_ref().unwrap()["hook_event_name"], "Stop");
    }

    #[test]
    fn permission_requests_wait_for_the_bridge() {
        let (dir, hub) = hub();
        let board = Board::with_hub_file(None, "test", Some(dir.path().join("hub.json")));
        let request = json!({"hook_event_name": "PermissionRequest", "tool_name": "Bash"});
        assert_eq!(board.claude_permission(request.clone()).unwrap(), None);
        hub.set_claude_permission(Some(Arc::new(|v: Value| (v["tool_name"] == "Bash").then_some(Decision::Deny))));
        assert_eq!(board.claude_permission(request).unwrap(), Some(Decision::Deny));
    }

    fn info(id: &str, kind: BoardKind) -> BoardInfo {
        BoardInfo { id: Some(id.into()), kind, port: format!("/dev/{id}"), firmware: "1.0.0".into(), primary: false }
    }

    #[test]
    fn a_call_reaches_one_board() {
        let one = [info("aa", BoardKind::Yoru)];
        assert_eq!(pick(&one, None, |_| false, None), Ok(0));
        let boards = [info("aa", BoardKind::Yoru), info("bb", BoardKind::Yoru), info("cc", BoardKind::Yoru)];
        // Named by id; a kind names a board only when it is the only one.
        assert_eq!(pick(&boards, Some("cc"), |_| false, None), Ok(2));
        assert_eq!(pick(&one, Some("yoru"), |_| false, None), Ok(0));
        assert_eq!(pick(&boards, Some("yoru"), |_| false, None).unwrap_err().0, SEVERAL_BOARDS);
        assert_eq!(pick(&boards, Some("zz"), |_| false, None).unwrap_err().0, NO_BOARD);
        // Unnamed: the only one with the tool, else the primary one.
        assert_eq!(pick(&boards, None, |i| i == 1, Some("cc")), Ok(1));
        assert_eq!(pick(&boards, None, |i| i > 0, Some("cc")), Ok(2));
        assert_eq!(pick(&boards, None, |_| true, Some("gone")).unwrap_err().0, SEVERAL_BOARDS);
    }

    #[test]
    fn clients_learn_the_boards() {
        let (dir, _hub) = hub();
        let mut c = client(&dir);
        assert_eq!(c.request("host/boards", json!({})).unwrap(), json!([]));
        let board = Board::with_hub_file(None, "test", Some(dir.path().join("hub.json")));
        assert_eq!(board.attached(), Some(vec![]));
        // Asked for one board while there's none: why there's none.
        match c.request("tools/call", json!({"name": "get_state", "board": "yoru"})) {
            Err(CallError::Unavailable(why)) => assert!(why.contains("not found the board"), "{why}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_board_asked_for_goes_with_the_request() {
        let board = Board::with_hub_file(None, "test", None).on(Some("yoru".into()));
        assert_eq!(board.aimed(json!({"name": "react"}), None), json!({"name": "react", "board": "yoru"}));
        assert_eq!(board.aimed(json!({}), Some("aa")), json!({"board": "aa"}));
        assert_eq!(Board::with_hub_file(None, "test", None).aimed(json!({}), None), json!({}));
    }

    #[test]
    fn file_goes_with_the_hub() {
        let (dir, hub) = hub();
        let path = dir.path().join("hub.json");
        assert!(path.exists());
        drop(hub);
        assert!(!path.exists());
        assert!(HubClient::connect(&path, "test").unwrap().is_none());
    }
}
