//! [`Connectors`] against real MCP servers: one over Streamable HTTP in this
//! process, and one over stdio that is this test binary run again
//! ([`stdio_server_helper`]). Secrets stay in memory; OAuth can't be tested
//! offline beyond a server that asks for it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager};
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::{Map, Value, json};

use super::*;

const STDIO_ENV: &str = "YORU_CONNECTORS_TEST_STDIO";
const TOKEN_ENV: &str = "YORU_TEST_TOKEN";

/// A server with a tool that reads, one that acts, one that fails, and one
/// that adds a tool and says so.
#[derive(Clone, Default)]
struct TestServer {
    grown: Arc<AtomicBool>,
}

fn schema() -> Map<String, Value> {
    json!({"type": "object", "properties": {"text": {"type": "string"}}}).as_object().unwrap().clone()
}

impl TestServer {
    fn tools(&self) -> Vec<Tool> {
        let mut tools = vec![
            Tool::new("echo", "Say the text back", schema()).with_annotations(ToolAnnotations::new().read_only(true)),
            Tool::new("add_note", "Add a note", schema()),
            Tool::new("fail", "Always fails", schema()).with_annotations(ToolAnnotations::new().read_only(true)),
            Tool::new("grow", "Add a tool", schema()),
            Tool::new("whoami", "The token from the environment", schema()).with_annotations(ToolAnnotations::new().read_only(true)),
        ];
        if self.grown.load(Ordering::SeqCst) {
            tools.push(Tool::new("extra.tool", "Added later", schema()));
        }
        tools
    }
}

impl ServerHandler for TestServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().enable_tool_list_changed().build())
    }

    async fn list_tools(&self, _: Option<PaginatedRequestParams>, _: RequestContext<RoleServer>) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tools()))
    }

    async fn call_tool(&self, request: CallToolRequestParams, context: RequestContext<RoleServer>) -> Result<CallToolResponse, ErrorData> {
        let text = request.arguments.as_ref().and_then(|a| a.get("text")).and_then(Value::as_str).unwrap_or_default().to_string();
        let result = match request.name.as_ref() {
            "echo" | "add_note" | "extra.tool" => CallToolResult::success(vec![ContentBlock::text(text)]),
            "fail" => CallToolResult::error(vec![ContentBlock::text("it failed")]),
            "whoami" => CallToolResult::success(vec![ContentBlock::text(std::env::var(TOKEN_ENV).unwrap_or_default())]),
            "grow" => {
                self.grown.store(true, Ordering::SeqCst);
                let _ = context.peer.notify_tool_list_changed().await;
                CallToolResult::success(vec![ContentBlock::text("grown")])
            }
            other => return Err(ErrorData::invalid_params(format!("no tool {other}"), None)),
        };
        Ok(result.into())
    }
}

/// Not a test: the stdio MCP server, when run by [`stdio_round_trip`].
/// libtest's own lines on stdout aren't JSON, and rmcp skips them
/// (`--nocapture` lets the helper's output through).
#[test]
fn stdio_server_helper() {
    if std::env::var_os(STDIO_ENV).is_none() {
        return;
    }
    // libtest wrote "test … " without a newline: end that line, so our
    // first message starts on a line of its own.
    println!();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let running = TestServer::default().serve(rmcp::transport::stdio()).await.unwrap();
        let _ = running.waiting().await;
    });
}

/// An HTTP MCP server on a free port; the runtime must outlive the test.
fn http_server(rt: &tokio::runtime::Runtime) -> String {
    rt.block_on(async {
        let server = TestServer::default();
        let service: StreamableHttpService<TestServer, LocalSessionManager> =
            StreamableHttpService::new(move || Ok(server.clone()), Default::default(), StreamableHttpServerConfig::default());
        let router = axum::Router::new().nest_service("/mcp", service);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await });
        format!("http://{addr}/mcp")
    })
}

/// A server that wants a sign-in for everything.
fn auth_server(rt: &tokio::runtime::Runtime) -> String {
    rt.block_on(async {
        let deny = || async {
            (
                axum::http::StatusCode::UNAUTHORIZED,
                [("www-authenticate", "Bearer resource_metadata=\"http://127.0.0.1/.well-known/oauth-protected-resource\"")],
                "",
            )
        };
        let router = axum::Router::new().route("/mcp", axum::routing::any(deny));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await });
        format!("http://{addr}/mcp")
    })
}

fn open(dir: &std::path::Path) -> (Connectors, Arc<Mutex<Vec<ConnectorEvent>>>, Arc<dyn SecretStore>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecrets::default());
    let c = Connectors::open_with_secrets(dir.to_path_buf(), secrets.clone(), move |e| sink.lock().unwrap().push(e)).unwrap();
    (c, events, secrets)
}

fn http(id: &str, url: &str) -> ConnectorConfig {
    ConnectorConfig {
        id: id.into(),
        name: id.to_uppercase(),
        transport: TransportConfig::Http { url: url.into() },
        oauth: None,
        enabled: true,
        tools: Default::default(),
    }
}

fn eventually(what: &str, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !ok() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

const T: Duration = Duration::from_secs(10);

#[test]
fn http_list_and_call() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let url = http_server(&rt);
    let dir = tempfile::tempdir().unwrap();
    let (c, events, _) = open(dir.path());

    c.add(http("local", &url)).unwrap();
    assert_eq!(c.wait_ready("local", T), ConnectionState::Connected);
    assert!(events.lock().unwrap().contains(&ConnectorEvent::StateChanged { id: "local".into(), state: ConnectionState::Connected }));

    let tools = c.tools();
    let echo = tools.iter().find(|t| t.tool.qualified_name == "local__echo").expect("echo listed");
    assert!(!echo.tool.acts && echo.tool.enabled);
    assert_eq!(echo.connector_name, "LOCAL");
    assert_eq!(echo.tool.input_schema["type"], "object");
    let note = tools.iter().find(|t| t.tool.name == "add_note").unwrap();
    assert!(note.tool.acts && note.tool.destructive);
    assert!(tools.iter().all(|t| names::is_valid(&t.tool.qualified_name)));

    let out = c.call("local__echo", json!({"text": "ciao"}), T).unwrap();
    assert_eq!(out, ToolOutput { text: "ciao".into(), structured: None, is_error: false });
    assert!(c.call("local__fail", json!({}), T).unwrap().is_error);
    assert_eq!(c.call("local__nope", json!({}), T), Err(ConnectorError::UnknownTool("local__nope".into())));
    assert!(matches!(c.call("local__echo", json!([1]), T), Err(ConnectorError::Call(_))));

    // The policy: off, the tool is neither offered nor callable.
    c.set_tool_policy("local", "echo", false, false).unwrap();
    assert!(!c.tools().iter().any(|t| t.tool.name == "echo"));
    assert_eq!(c.call("local__echo", json!({}), T), Err(ConnectorError::ToolDisabled("local__echo".into())));
    assert!(c.list()[0].tools.iter().any(|t| t.name == "echo" && !t.enabled));

    // tools/list_changed: the new tool shows up under a valid name.
    events.lock().unwrap().clear();
    c.call("local__grow", json!({}), T).unwrap();
    eventually("the new tool", || c.tools().iter().any(|t| t.tool.name == "extra.tool"));
    assert!(events.lock().unwrap().contains(&ConnectorEvent::ToolsChanged { id: "local".into() }));
    let extra = c.tools().into_iter().find(|t| t.tool.name == "extra.tool").unwrap();
    assert!(names::is_valid(&extra.tool.qualified_name));
    assert_eq!(c.resolve(&extra.tool.qualified_name).unwrap().1.name, "extra.tool");
    assert_eq!(c.call(&extra.tool.qualified_name, json!({"text": "x"}), T).unwrap().text, "x");

    // Off and on again.
    c.set_enabled("local", false).unwrap();
    assert_eq!(c.get("local").unwrap().state, ConnectionState::Disabled);
    assert!(c.tools().is_empty());
    assert!(matches!(c.call("local__add_note", json!({}), T), Err(ConnectorError::NotConnected(_))));
    c.set_enabled("local", true).unwrap();
    assert_eq!(c.wait_ready("local", T), ConnectionState::Connected);

    // The file keeps it all; a new instance connects on its own.
    drop(c);
    let (c, _, _) = open(dir.path());
    assert_eq!(c.wait_ready("local", T), ConnectionState::Connected);
    assert!(!c.tools().iter().any(|t| t.tool.name == "echo"), "the policy survived");
    assert_eq!(c.add(http("local", &url)).unwrap_err(), ConnectorError::AlreadyExists("local".into()));
    c.remove("local").unwrap();
    assert!(c.list().is_empty());
    assert_eq!(c.remove("local"), Err(ConnectorError::UnknownConnector("local".into())));
}

#[test]
fn stdio_round_trip() {
    let exe = std::env::current_exe().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let (c, _, secrets) = open(dir.path());
    let mut env = std::collections::BTreeMap::new();
    env.insert(STDIO_ENV.into(), EnvValue::Plain("1".into()));
    env.insert(TOKEN_ENV.into(), EnvValue::Secret { secret: "TEST_TOKEN".into() });
    let config = ConnectorConfig {
        id: "local-stdio".into(),
        name: "Local".into(),
        transport: TransportConfig::Stdio {
            command: exe.to_string_lossy().into_owned(),
            args: ["--exact", "connectors::tests::stdio_server_helper", "--nocapture", "--test-threads=1"].map(String::from).to_vec(),
            env,
        },
        oauth: None,
        enabled: true,
        tools: Default::default(),
    };
    c.add(config).unwrap();
    // Without its secret it can't start.
    match c.wait_ready("local-stdio", T) {
        ConnectionState::Error { message } => assert!(message.contains("TEST_TOKEN"), "{message}"),
        other => panic!("{other:?}"),
    }
    assert!(!c.has_secret("TEST_TOKEN").unwrap());
    c.set_secret("TEST_TOKEN", "t0ken").unwrap();
    assert_eq!(secrets.get("secret.TEST_TOKEN").unwrap().as_deref(), Some("t0ken"));
    assert_eq!(c.wait_ready("local-stdio", T), ConnectionState::Connected);
    assert_eq!(c.call("local-stdio__whoami", json!({}), T).unwrap().text, "t0ken");
    assert_eq!(c.call("local-stdio__echo", json!({"text": "hi"}), T).unwrap().text, "hi");
    assert_eq!(c.login("local-stdio", &|_| {}), Err(ConnectorError::NoLogin("local-stdio".into())));
    assert_eq!(c.logout("local-stdio"), Err(ConnectorError::NoLogin("local-stdio".into())));
    // Removing the connector keeps the named secret, which others may use.
    c.remove("local-stdio").unwrap();
    assert!(c.has_secret("TEST_TOKEN").unwrap());
}

#[test]
fn a_server_that_wants_a_sign_in() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let url = auth_server(&rt);
    let dir = tempfile::tempdir().unwrap();
    let (c, events, secrets) = open(dir.path());
    c.add(http("private", &url)).unwrap();
    eventually("needs_login", || c.get("private").unwrap().state == ConnectionState::NeedsLogin);
    let status = c.get("private").unwrap();
    assert!(!status.signed_in);
    // Never signed in: nothing expired, so no login_needed.
    assert!(!events.lock().unwrap().iter().any(|e| matches!(e, ConnectorEvent::LoginNeeded { .. })));
    assert!(c.tools().is_empty());
    // A client secret given with the config goes to the keychain only.
    let mut config = http("private", &url);
    config.oauth = Some(OAuthConfig { client_id: Some("cid".into()), client_secret: Some("csecret".into()), ..Default::default() });
    c.update(config).unwrap();
    assert_eq!(secrets.get("private.client-secret").unwrap().as_deref(), Some("csecret"));
    let file = std::fs::read_to_string(dir.path().join(config::FILE_NAME)).unwrap();
    assert!(file.contains("cid") && !file.contains("csecret"), "{file}");
    c.remove("private").unwrap();
    assert!(secrets.get("private.client-secret").unwrap().is_none());
}

#[test]
fn bad_configs_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (c, _, _) = open(dir.path());
    assert!(matches!(c.add(http("Bad_Id", "https://x.example/mcp")), Err(ConnectorError::InvalidConfig(_))));
    assert!(matches!(c.add(http("x", "not a url")), Err(ConnectorError::InvalidConfig(_))));
    assert_eq!(c.set_enabled("nope", true), Err(ConnectorError::UnknownConnector("nope".into())));
    assert!(matches!(c.set_secret("bad name", "x"), Err(ConnectorError::InvalidConfig(_))));
}

#[test]
fn outputs_read_as_text() {
    use rmcp::model::{Resource, ResourceContents};
    let mut result = CallToolResult::success(vec![
        ContentBlock::text("one"),
        ContentBlock::image("aGVsbG8=", "image/png"),
        ContentBlock::resource(ResourceContents::text("body", "file:///a.txt")),
        ContentBlock::resource_link(Resource::new("file:///b.pdf", "b.pdf")),
    ]);
    result.structured_content = Some(json!({"n": 1}));
    let out = manager::output(result);
    assert_eq!(out.text, "one\n[image: image/png, 0 KB]\n[resource file:///a.txt]\nbody\n[link b.pdf: file:///b.pdf]");
    assert_eq!(out.structured, Some(json!({"n": 1})));
    // Only structured content: the text carries it too.
    let out = manager::output(CallToolResult::structured(json!({"ok": true})));
    assert!(out.text.contains("\"ok\":true"), "{}", out.text);
}

#[test]
fn public_types_serialize_for_the_app() {
    let e = ConnectorEvent::StateChanged { id: "x".into(), state: ConnectionState::Error { message: "down".into() } };
    assert_eq!(serde_json::to_value(e).unwrap(), json!({"event": "state_changed", "id": "x", "state": "error", "message": "down"}));
    assert_eq!(serde_json::to_value(ConnectorError::LoginTimeout).unwrap(), json!({"kind": "login_timeout"}));
    assert_eq!(serde_json::to_value(ConnectorError::NeedsLogin("x".into())).unwrap(), json!({"kind": "needs_login", "message": "x"}));
}
