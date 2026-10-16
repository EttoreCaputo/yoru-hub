//! MCP server over stdio, for Claude Code, Claude Desktop and other MCP
//! clients (`companion mcp`, or `companion-hub --mcp`).
//!
//! The board's tools are passed through as the firmware describes them in
//! `tools/list`, so a new firmware tool shows up without a host change. The
//! board is reached through [`Board`]: the running app or `companion` bridge,
//! or the port opened for each call. Two host tools read this computer,
//! `get_metrics` and `get_claude_usage`, and `speak` talks through the
//! board's speaker with the bridge's text-to-speech. What the client is told
//! about the board comes from its kind's [`BoardProfile`](crate::BoardProfile).
//!
//! With several boards attached to the bridge, the tools are every board's,
//! each with an optional `board` argument (an id, or a kind), and
//! `list_boards` says which boards there are.
//!
//! The last tool list of each kind of board is kept in
//! `board-tools-<kind>.json`, so the board's tools are listed even when the
//! client starts while the board is away; calling one then says why it can't
//! run. If the board had never been seen, the server tells the client once
//! its tools become available.

use std::borrow::Cow;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, ListToolsResult, PaginatedRequestParams, ServerCapabilities,
    ServerConfig, Tool as McpTool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, Peer, RoleServer, ServerHandler, ServiceExt};
use serde_json::{Value, json};

use crate::claude::ClaudeUsage;
use crate::claude::alerts;
use crate::music::Music;
use crate::timers::Timers;
use crate::hub::{Board, BoardInfo};
use crate::link::{self, CallError, Tool, ToolResult};
use crate::profile::BoardKind;
use crate::sensors::Collector;
use crate::snapshot::{Snapshot, short_floats};

/// The argument that turns the app binary into the MCP server.
pub const FLAG: &str = "--mcp";

/// What the client is told before it knows which board this is.
const ABOUT_ANY_BOARD: &str = "The board is a small desk companion connected to this computer: \
a DualEye (two round screens with watch faces) or a Yoru (a desk pet on one round screen). Its tools say what it can do.";

/// What the client is told about the host's own tools, whatever the board.
const HOST_INSTRUCTIONS: &str = "get_metrics and get_claude_usage read this computer, and work without the board. \
speak says a short sentence out loud through the board's speaker, in Italian or English. \
set_timer, set_reminder, pomodoro, control_timer, update_timer and get_timers run timers on the host: the board rings when one is up, \
and a DualEye counts the first one down on a screen (they need the Companion Hub app or `companion` running to ring). \
media_control and now_playing play, pause or skip the music on this computer and say what's playing.";

/// What the client is told when several boards are connected.
const SEVERAL_BOARDS: &str = "Several boards are connected: list_boards names them. \
Board tools and speak take `board`, a board's id or kind; without it a call goes to the only board with that tool, or the primary one.";

/// A bridge sample older than this is not "current"; sample here instead.
const FRESH: Duration = Duration::from_secs(5);
/// While the board's tools are unknown, look for them this often.
const WATCH: Duration = Duration::from_secs(10);

/// Serve MCP on stdin/stdout until the client closes it. `port` is used only
/// when no bridge is running (`None` auto-detects the board).
pub fn serve_stdio(port: Option<String>) -> io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(async {
        let server = CompanionMcp::new(Board::new(port, &format!("companion-mcp/{}", env!("CARGO_PKG_VERSION"))), crate::claude::data_dir());
        // The instructions name the board: learn its kind before the client asks.
        let probe = server.clone();
        let _ = tokio::task::spawn_blocking(move || probe.refresh_kind()).await;
        let running = server.clone().serve(rmcp::transport::stdio()).await.map_err(io::Error::other)?;
        let watcher = tokio::spawn(watch_tools(server, running.peer().clone()));
        let _ = running.waiting().await;
        watcher.abort();
        Ok(())
    })
}

#[derive(Clone)]
pub struct CompanionMcp {
    inner: Arc<Inner>,
}

struct Inner {
    board: Board,
    /// The folder with the tool lists kept per kind (`board-tools-<kind>.json`).
    cache_dir: Option<PathBuf>,
    /// The kind of board last reached: whose tools are kept, and what the
    /// instructions describe.
    kind: Mutex<Option<BoardKind>>,
    /// Board tools as last listed to the client, and whether they came from the board itself.
    listed: Mutex<Option<(Vec<Tool>, bool)>>,
    /// The boards attached to the bridge, as last asked.
    boards: Mutex<Vec<BoardInfo>>,
    collector: Mutex<Option<Collector>>,
    claude: Mutex<Option<ClaudeUsage>>,
}

impl CompanionMcp {
    pub fn new(board: Board, cache_dir: Option<PathBuf>) -> Self {
        Self {
            inner: Arc::new(Inner {
                board,
                cache_dir,
                kind: Mutex::default(),
                listed: Mutex::default(),
                boards: Mutex::default(),
                collector: Mutex::default(),
                claude: Mutex::default(),
            }),
        }
    }

    /// Ask the board which kind it is; when it can't be reached, keep the
    /// last kind known.
    fn refresh_kind(&self) -> Option<BoardKind> {
        *self.inner.boards.lock().unwrap() = self.inner.board.attached().unwrap_or_default();
        let mut kind = self.inner.kind.lock().unwrap();
        if let Ok(now) = self.inner.board.kind() {
            *kind = Some(now);
        }
        *kind
    }

    /// More than one board attached, as last asked.
    fn several(&self) -> bool {
        self.inner.boards.lock().unwrap().len() > 1
    }

    /// The kind of board to describe: the last one reached, else the one
    /// whose tools were kept most recently.
    fn kind(&self) -> Option<BoardKind> {
        let known = *self.inner.kind.lock().unwrap();
        known.or_else(|| {
            let dir = self.inner.cache_dir.as_ref()?;
            BoardKind::ALL
                .into_iter()
                .filter_map(|k| Some((fs::metadata(link::tools_cache_in(dir, k)).ok()?.modified().ok()?, k)))
                .max_by_key(|(modified, _)| *modified)
                .map(|(_, k)| k)
        })
    }

    fn cache_file(&self, kind: BoardKind) -> Option<PathBuf> {
        self.inner.cache_dir.as_ref().map(|d| link::tools_cache_in(d, kind))
    }

    /// The board's tools: live, else the last ones seen. The flag says which.
    fn board_tools(&self) -> (Vec<Tool>, bool) {
        let inner = &self.inner;
        let boards = inner.board.attached().unwrap_or_default();
        *inner.boards.lock().unwrap() = boards.clone();
        if boards.len() > 1 {
            return (self.every_boards_tools(&boards), true);
        }
        match inner.board.list_tools() {
            Ok(tools) => {
                // Another board may sit there now: keep these under its kind.
                if let Some(path) = self.refresh_kind().and_then(|k| self.cache_file(k))
                    && let Ok(json) = serde_json::to_vec_pretty(&tools)
                {
                    let _ = path.parent().map(fs::create_dir_all);
                    let _ = fs::write(path, json);
                }
                (tools, true)
            }
            Err(_) => {
                let cached = self.kind().and_then(|k| self.cache_file(k)).and_then(|p| fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok());
                (cached.unwrap_or_default(), false)
            }
        }
    }

    /// Each board's tools, kept under its kind, each name once.
    fn every_boards_tools(&self, boards: &[BoardInfo]) -> Vec<Tool> {
        let mut all: Vec<Tool> = Vec::new();
        for b in boards {
            let Ok(tools) = self.inner.board.list_tools_on(Some(&selector(b))) else { continue };
            if let Some(path) = self.cache_file(b.kind)
                && let Ok(json) = serde_json::to_vec_pretty(&tools)
            {
                let _ = path.parent().map(fs::create_dir_all);
                let _ = fs::write(path, json);
            }
            for tool in tools {
                if !all.iter().any(|t| t.name == tool.name) {
                    all.push(tool);
                }
            }
        }
        all
    }

    /// The instructions for the client: the boards' kinds, then the host's tools.
    fn instructions(&self) -> String {
        let mut kinds: Vec<BoardKind> = self.inner.boards.lock().unwrap().iter().map(|b| b.kind).collect();
        kinds.dedup();
        if self.several() {
            let about: Vec<&str> = BoardKind::ALL.into_iter().filter(|k| kinds.contains(k)).map(|k| k.profile().mcp_about).collect();
            return format!("{} {SEVERAL_BOARDS} {HOST_INSTRUCTIONS}", about.join(" "));
        }
        let about = self.kind().map_or(ABOUT_ANY_BOARD, |k| k.profile().mcp_about);
        format!("{about} {HOST_INSTRUCTIONS}")
    }

    /// The board's name in messages: its kind's, or just "the board".
    fn board_name(&self) -> String {
        self.kind().map_or_else(|| "board".to_string(), |k| format!("{k} board"))
    }

    fn all_tools(&self) -> Vec<McpTool> {
        let (board, live) = self.board_tools();
        let boards = self.inner.boards.lock().unwrap().clone();
        let mut host = host_tools();
        if boards.len() > 1
            && let Some(speak) = host.iter_mut().find(|t| t["name"] == "speak")
        {
            add_board_argument(&mut speak["inputSchema"], &boards);
        }
        let mut tools: Vec<McpTool> = host.into_iter().filter_map(|t| serde_json::from_value(t).ok()).collect();
        let names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
        for tool in &board {
            let mut tool = tool.clone();
            if boards.len() > 1 {
                add_board_argument(&mut tool.input_schema, &boards);
            }
            // Host tools win a name clash.
            if !names.contains(&tool.name)
                && let Ok(t) = serde_json::to_value(&tool).and_then(serde_json::from_value)
            {
                tools.push(t);
            }
        }
        *self.inner.listed.lock().unwrap() = Some((board, live));
        tools
    }

    fn call(&self, name: &str, mut arguments: Value) -> Result<CallToolResult, ErrorData> {
        // Which board is the hub's business, not the board's.
        let on = arguments.as_object_mut().and_then(|a| a.remove("board")).and_then(|b| b.as_str().map(str::to_string));
        let on = on.as_deref();
        match name {
            "get_metrics" => Ok(json_result(self.metrics().metrics_json())),
            "list_boards" => Ok(match self.inner.board.boards() {
                Ok(boards) => json_result(json!({"boards": boards})),
                Err(e) => text_result(&format!("No board can be reached: {e}."), true),
            }),
            "get_claude_usage" => Ok(match self.claude_usage() {
                Some(usage) => json_result(usage),
                None => text_result("No Claude Code usage on this computer: Claude Code has not run here, or keeps its data elsewhere (CLAUDE_CONFIG_DIR).", false),
            }),
            "speak" => {
                let text = arguments.get("text").and_then(Value::as_str).unwrap_or("");
                if text.trim().is_empty() {
                    return Err(ErrorData::invalid_params("text must be a non-empty string", None));
                }
                Ok(match self.inner.board.say_on(on, text, arguments.get("language").and_then(Value::as_str)) {
                    Ok(spoken) => json_result(spoken),
                    Err(e) => text_result(&format!("The {} can't speak: {e}.", self.board_name()), true),
                })
            }
            name if Music::tools().iter().any(|t| t.name == name) => Ok(match Music::default().call_tool(name, &arguments) {
                Some(Ok(text)) if name == "now_playing" => json_result(serde_json::from_str(&text).unwrap_or(Value::String(text))),
                Some(Ok(text)) => text_result(&text, false),
                Some(Err(e)) => text_result(&e, true),
                None => text_result(&format!("unknown tool {name}"), true),
            }),
            name if Timers::tools().iter().any(|t| t.name == name) => {
                let language = arguments.get("language").and_then(Value::as_str).unwrap_or(alerts::system_language()).to_string();
                Ok(match self.inner.board.timer_tool(name, &arguments, &language) {
                    Ok(text) if name == "get_timers" => json_result(serde_json::from_str(&text).unwrap_or(Value::String(text))),
                    Ok(text) => text_result(&text, false),
                    Err(e) => text_result(&e, true),
                })
            }
            _ => match self.inner.board.call_tool_on(on, name, arguments) {
                Ok(result) => to_mcp(result),
                Err(CallError::Rpc { code: -32602, message }) => Err(ErrorData::invalid_params(message, None)),
                Err(e) => Ok(text_result(&format!("The {} can't run {name}: {e}.", self.board_name()), true)),
            },
        }
    }

    /// The bridge's sample while it is fresh, else one taken here.
    fn metrics(&self) -> Snapshot {
        if let Some((snapshot, _)) = self.inner.board.bridge_snapshot().filter(|(_, age)| *age < FRESH) {
            return snapshot;
        }
        let mut slot = self.inner.collector.lock().unwrap();
        let collector = match slot.as_mut() {
            Some(c) => c,
            None => {
                // CPU load is the change between two samples.
                let c = slot.insert(Collector::new());
                c.sample();
                std::thread::sleep(Duration::from_millis(500));
                c
            }
        };
        collector.sample()
    }

    fn claude_usage(&self) -> Option<Value> {
        let from_bridge = self.inner.board.bridge_snapshot().filter(|(_, age)| *age < FRESH).and_then(|(s, _)| s.claude);
        let usage = from_bridge.or_else(|| self.inner.claude.lock().unwrap().get_or_insert_with(ClaudeUsage::new).sample())?;
        Some(short_floats(&usage))
    }
}

/// What names a board to the hub: its id, else its kind.
fn selector(board: &BoardInfo) -> String {
    board.id.clone().unwrap_or_else(|| board.kind.profile().slug.to_string())
}

/// Give a tool's input schema an optional `board`: one of the boards' ids, or a kind.
fn add_board_argument(schema: &mut Value, boards: &[BoardInfo]) {
    let mut names: Vec<String> = boards.iter().map(selector).collect();
    for b in boards {
        let slug = b.kind.profile().slug.to_string();
        if !names.contains(&slug) {
            names.push(slug);
        }
    }
    if !schema.is_object() {
        *schema = json!({"type": "object"});
    }
    let properties = schema.as_object_mut().expect("an object").entry("properties").or_insert_with(|| json!({}));
    if let Some(p) = properties.as_object_mut() {
        p.insert(
            "board".into(),
            json!({"type": "string", "enum": names, "description": "Which board: its id, or its kind when only one of that kind is connected (see list_boards)"}),
        );
    }
}

fn host_tools() -> Vec<Value> {
    let no_args = json!({"type": "object", "properties": {}, "additionalProperties": false});
    let timers = Timers::tools().into_iter().map(|t| {
        let read_only = t.name == "get_timers";
        json!({
            "name": t.name,
            "description": t.description,
            "inputSchema": t.input_schema,
            "annotations": {"readOnlyHint": read_only, "destructiveHint": false, "openWorldHint": false},
        })
    });
    let music = Music::tools().into_iter().map(|t| {
        let read_only = t.name == "now_playing";
        json!({
            "name": t.name,
            "description": t.description,
            "inputSchema": t.input_schema,
            "annotations": {"readOnlyHint": read_only, "destructiveHint": false, "openWorldHint": false},
        })
    });
    let mut tools = vec![
        json!({
            "name": "get_metrics",
            "title": "Computer metrics",
            "description": "Current CPU and GPU temperature, load, clock, power and memory of this computer, and its fan speeds.",
            "inputSchema": no_args,
            "annotations": {"readOnlyHint": true, "openWorldHint": false},
        }),
        json!({
            "name": "get_claude_usage",
            "title": "Claude Code usage",
            "description": "Claude Code usage on this computer: tokens in the current 5-hour window (tok) and today, \
                            share of the plan's 5-hour (s_pct) and weekly (w_pct) limits used, minutes until the window resets (left_min), \
                            whether Claude is working, idle or asleep, and the latest model.",
            "inputSchema": no_args,
            "annotations": {"readOnlyHint": true, "openWorldHint": false},
        }),
        json!({
            "name": "list_boards",
            "title": "Connected boards",
            "description": "The boards connected to this computer: each one's id, kind (dualeye or yoru), port, firmware, and whether it is the primary one.",
            "inputSchema": no_args,
            "annotations": {"readOnlyHint": true, "openWorldHint": false},
        }),
        json!({
            "name": "speak",
            "title": "Speak through the board",
            "description": "Say a short text out loud through the board's speaker. Returns once it has been played. \
                            Needs the Companion Hub app with spoken replies on (or `companion --tts`) running.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": {"type": "string", "maxLength": 500, "description": "Plain sentences, no markdown"},
                    "language": {"type": "string", "enum": ["it", "en"], "description": "Default: what the text looks like"},
                },
                "required": ["text"],
                "additionalProperties": false,
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": false},
        }),
    ];
    tools.extend(timers);
    tools.extend(music);
    tools
}

fn to_mcp(result: ToolResult) -> Result<CallToolResult, ErrorData> {
    serde_json::to_value(result).and_then(serde_json::from_value).map_err(|e| ErrorData::internal_error(format!("board result: {e}"), None))
}

fn json_result(value: Value) -> CallToolResult {
    let text = serde_json::to_string_pretty(&value).unwrap_or_default();
    let mut result = CallToolResult::success(vec![ContentBlock::text(text)]);
    result.structured_content = Some(value);
    result
}

fn text_result(text: &str, is_error: bool) -> CallToolResult {
    if is_error { CallToolResult::error(vec![ContentBlock::text(text)]) } else { CallToolResult::success(vec![ContentBlock::text(text)]) }
}

impl ServerHandler for CompanionMcp {
    fn get_info(&self) -> ServerConfig {
        let mut info = Implementation::new("companion-hub", env!("CARGO_PKG_VERSION"));
        info.title = Some("Companion Hub".into());
        ServerConfig::new(ServerCapabilities::builder().enable_tools().enable_tool_list_changed().build())
            .with_server_info(info)
            .with_instructions(self.instructions())
    }

    async fn list_tools(&self, _request: Option<PaginatedRequestParams>, _context: RequestContext<RoleServer>) -> Result<ListToolsResult, ErrorData> {
        let this = self.clone();
        let tools = tokio::task::spawn_blocking(move || this.all_tools()).await.map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(&self, request: CallToolRequestParams, _context: RequestContext<RoleServer>) -> Result<CallToolResponse, ErrorData> {
        let this = self.clone();
        let name: Cow<'static, str> = request.name;
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        tokio::task::spawn_blocking(move || this.call(&name, arguments))
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
            .map(CallToolResponse::from)
    }
}

/// Tell the client when the board's tools turn up after it listed them without.
async fn watch_tools(server: CompanionMcp, peer: Peer<RoleServer>) {
    loop {
        tokio::time::sleep(WATCH).await;
        let listed = server.inner.listed.lock().unwrap().clone();
        let Some((before, false)) = listed else { continue };
        let this = server.clone();
        let Ok((now, true)) = tokio::task::spawn_blocking(move || this.board_tools()).await else { continue };
        let names = |tools: &[Tool]| tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>();
        if names(&now) != names(&before) && peer.notify_tool_list_changed().await.is_err() {
            return;
        }
        *server.inner.listed.lock().unwrap() = Some((now, true));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_tools_convert_to_mcp() {
        for tool in host_tools() {
            let t: McpTool = serde_json::from_value(tool).expect("valid MCP tool");
            // Only speak, the timers and media_control do something.
            let reads = crate::agent::is_read_only(&t.name);
            assert_eq!(t.annotations.and_then(|a| a.read_only_hint), Some(reads), "{}", t.name);
        }
    }

    #[test]
    fn several_boards_add_a_board_argument() {
        let boards = [
            BoardInfo { id: Some("aa".into()), kind: BoardKind::Yoru, port: String::new(), firmware: "1".into(), primary: false },
            BoardInfo { id: Some("bb".into()), kind: BoardKind::DualEye, port: String::new(), firmware: "1".into(), primary: false },
        ];
        let mut schema = json!({"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]});
        add_board_argument(&mut schema, &boards);
        assert_eq!(schema["properties"]["board"]["enum"], json!(["aa", "bb", "yoru", "dualeye"]));
        assert_eq!(schema["required"], json!(["name"]));
        let mut empty = Value::Null;
        add_board_argument(&mut empty, &boards);
        let _: McpTool = serde_json::from_value(json!({"name": "x", "inputSchema": empty})).expect("valid MCP tool");
    }

    #[test]
    fn board_results_pass_through() {
        let board: ToolResult = serde_json::from_value(json!({
            "content": [{"type": "text", "text": "left: rings"}],
            "isError": false,
        }))
        .unwrap();
        let mcp = to_mcp(board).unwrap();
        assert_eq!(mcp.is_error, Some(false));
        assert_eq!(serde_json::to_value(&mcp.content).unwrap(), json!([{"type": "text", "text": "left: rings"}]));
    }

    #[test]
    fn board_tools_come_from_the_cache_when_the_board_is_away() {
        let dir = tempfile::tempdir().unwrap();
        let tools = json!([{"name": "set_face", "description": "Switch the watch face", "inputSchema": {"type": "object"}}]);
        fs::write(link::tools_cache_in(dir.path(), BoardKind::DualEye), tools.to_string()).unwrap();
        // A port that isn't there, and no hub: the board can't be reached.
        let board = Board::with_hub_file(Some("/dev/does-not-exist".into()), "test", None);
        let server = CompanionMcp::new(board, Some(dir.path().to_path_buf()));
        let (listed, live) = server.board_tools();
        assert!(!live);
        assert_eq!(listed[0].name, "set_face");
        assert!(server.instructions().starts_with("DualEye is"));
    }

    #[test]
    fn the_newest_kept_tools_name_the_board() {
        let dir = tempfile::tempdir().unwrap();
        let board = Board::with_hub_file(Some("/dev/does-not-exist".into()), "test", None);
        let server = CompanionMcp::new(board, Some(dir.path().to_path_buf()));
        assert!(server.instructions().starts_with(ABOUT_ANY_BOARD));
        let react = json!([{"name": "react", "description": "React", "inputSchema": {"type": "object"}}]);
        fs::write(link::tools_cache_in(dir.path(), BoardKind::Yoru), react.to_string()).unwrap();
        assert_eq!(server.board_tools().0[0].name, "react");
        assert!(server.instructions().starts_with("Yoru is"));
    }
}
