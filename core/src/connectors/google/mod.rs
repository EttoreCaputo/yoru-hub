//! Gmail and Google Calendar through their REST APIs, as connectors.
//!
//! Google's own MCP servers want an OAuth client of Google's liking, and a
//! verified app for everyone else's account. The REST APIs
//! (`gmail.googleapis.com`, `www.googleapis.com/calendar/v3`) take any
//! account, personal ones too, with a client the user makes in their own
//! Google Cloud project ([`oauth`]). Yoru serves their tools itself: an MCP
//! server in-process, on a pipe, so the manager sees a connector like any
//! other (sessions, tool policies, `needs_login`).
//!
//! - [`oauth`]: the sign-in and the tokens.
//! - [`gmail`], [`calendar`]: the tools, each a REST call or a few.

mod calendar;
mod gmail;
pub(crate) mod oauth;

use std::sync::Arc;
use std::time::Duration;

use reqwest::Method;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, ListToolsResult, PaginatedRequestParams, ServerCapabilities,
    ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::{Map, Value, json};

use super::config::{ConnectorConfig, ConnectorsFile, GoogleService, OAuthConfig, TransportConfig};
use oauth::{TokenError, TokenSource};

const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// Room on the pipe between the manager's client and the server.
const PIPE_BYTES: usize = 256 * 1024;

impl GoogleService {
    /// What the consent screen asks for: the least that lets the tools work.
    pub fn scopes(self) -> &'static [&'static str] {
        match self {
            // Read, label, archive, trash and draft; sending is its own scope.
            Self::Gmail => &["https://www.googleapis.com/auth/gmail.modify", "https://www.googleapis.com/auth/gmail.send"],
            Self::Calendar => &["https://www.googleapis.com/auth/calendar.events", "https://www.googleapis.com/auth/calendar.readonly"],
        }
    }

    /// The API to enable in the Google Cloud project.
    pub fn api_name(self) -> &'static str {
        match self {
            Self::Gmail => "Gmail API",
            Self::Calendar => "Google Calendar API",
        }
    }

    fn tools(self) -> Vec<ToolDef> {
        match self {
            Self::Gmail => gmail::tools(),
            Self::Calendar => calendar::tools(),
        }
    }
}

/// One tool, before it becomes an MCP [`Tool`].
pub(crate) struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub properties: Value,
    pub required: &'static [&'static str],
    pub read_only: bool,
    /// Only meaningful when it acts: it can lose something.
    pub destructive: bool,
}

impl ToolDef {
    fn to_mcp(&self) -> Tool {
        serde_json::from_value(json!({
            "name": self.name,
            "description": self.description,
            "inputSchema": {"type": "object", "properties": self.properties, "required": self.required},
            "annotations": {"readOnlyHint": self.read_only, "destructiveHint": self.destructive, "openWorldHint": true},
        }))
        .expect("a valid tool")
    }
}

/// What a tool says back: text for the model, or a failure it should see.
pub(crate) type ToolResult = Result<String, String>;

/// Why a REST call failed.
#[derive(Debug)]
pub(crate) enum ApiError {
    /// The tokens are gone: the connector needs a sign-in.
    SignIn(String),
    /// Google answered with an error (its message, which names the API to
    /// enable when it's off).
    Google { status: u16, message: String },
    Other(String),
}

impl ApiError {
    pub fn describe(self) -> String {
        match self {
            Self::SignIn(why) => format!("Google needs you to sign in again in Yoru Hub, Connectors ({why})"),
            Self::Google { status, message } => format!("Google answered {status}: {message}"),
            Self::Other(why) => why,
        }
    }
}

/// The REST calls of one connector, with its tokens.
pub(crate) struct Api {
    tokens: TokenSource,
    http: reqwest::Client,
    /// Called when Google refuses the refresh token.
    sign_in_needed: Box<dyn Fn() + Send + Sync>,
}

impl Api {
    /// `GET` with query parameters.
    pub async fn get(&self, url: &str, query: &[(&str, String)]) -> Result<Value, ApiError> {
        self.send(Method::GET, url, query, None).await
    }

    pub async fn send(&self, method: Method, url: &str, query: &[(&str, String)], body: Option<Value>) -> Result<Value, ApiError> {
        let mut url = url::Url::parse(url).map_err(|e| ApiError::Other(e.to_string()))?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
        }
        // A 401 with a token we thought valid: refresh once and try again.
        for attempt in 0..2 {
            let token = self.token().await?;
            let mut request = self.http.request(method.clone(), url.clone()).bearer_auth(token).timeout(CALL_TIMEOUT);
            if let Some(body) = &body {
                request = request.header(reqwest::header::CONTENT_TYPE, "application/json").body(body.to_string());
            }
            let response = request.send().await.map_err(|e| ApiError::Other(format!("reaching Google: {e}")))?;
            let status = response.status();
            let bytes = response.bytes().await.map_err(|e| ApiError::Other(e.to_string()))?;
            if status == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
                self.tokens.invalidate().await;
                continue;
            }
            if status.is_success() {
                return Ok(if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).map_err(|e| ApiError::Other(e.to_string()))? });
            }
            let error: Value = serde_json::from_slice(&bytes).unwrap_or_default();
            let message = error["error"]["message"].as_str().map(str::to_string).unwrap_or_else(|| String::from_utf8_lossy(&bytes).chars().take(300).collect());
            if status == reqwest::StatusCode::UNAUTHORIZED {
                (self.sign_in_needed)();
                return Err(ApiError::SignIn(message));
            }
            return Err(ApiError::Google { status: status.as_u16(), message });
        }
        unreachable!("the loop returns on its second pass")
    }

    async fn token(&self) -> Result<String, ApiError> {
        match self.tokens.access_token().await {
            Ok(token) => Ok(token),
            Err(TokenError::SignInAgain(why)) => {
                (self.sign_in_needed)();
                Err(ApiError::SignIn(why))
            }
            Err(TokenError::Other(why)) => Err(ApiError::Other(why)),
        }
    }
}

/// The in-process MCP server of one Google connector.
#[derive(Clone)]
struct Server {
    service: GoogleService,
    api: Arc<Api>,
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        let info = Implementation::new("yoru-google", env!("CARGO_PKG_VERSION"));
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(info)
    }

    async fn list_tools(&self, _request: Option<PaginatedRequestParams>, _context: RequestContext<RoleServer>) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.service.tools().iter().map(ToolDef::to_mcp).collect()))
    }

    async fn call_tool(&self, request: CallToolRequestParams, _context: RequestContext<RoleServer>) -> Result<CallToolResponse, ErrorData> {
        let args = Args(request.arguments.unwrap_or_default());
        let result = match self.service {
            GoogleService::Gmail => gmail::call(&self.api, &request.name, &args).await,
            GoogleService::Calendar => calendar::call(&self.api, &request.name, &args).await,
        };
        Ok(CallToolResponse::from(match result {
            Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
            Err(why) => CallToolResult::error(vec![ContentBlock::text(why)]),
        }))
    }
}

/// Start the server of a Google connector on one end of a pipe; the other
/// end is for the manager's client. The server ends when the client does.
pub(crate) fn spawn_server(
    service: GoogleService,
    tokens: TokenSource,
    sign_in_needed: Box<dyn Fn() + Send + Sync>,
) -> tokio::io::DuplexStream {
    let (client_end, server_end) = tokio::io::duplex(PIPE_BYTES);
    let api = Arc::new(Api { tokens, http: reqwest::Client::new(), sign_in_needed });
    tokio::spawn(async move {
        if let Ok(running) = (Server { service, api }).serve(server_end).await {
            let _ = running.waiting().await;
        }
    });
    client_end
}

/// A tool's arguments, read leniently: models send numbers as strings.
pub(crate) struct Args(pub Map<String, Value>);

impl Args {
    pub fn str(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
    }

    pub fn need(&self, key: &str) -> Result<&str, String> {
        self.str(key).ok_or_else(|| format!("`{key}` is missing"))
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        match self.0.get(key)? {
            Value::Bool(b) => Some(*b),
            Value::String(s) => s.parse().ok(),
            _ => None,
        }
    }

    pub fn int(&self, key: &str) -> Option<u64> {
        match self.0.get(key)? {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    /// A list of strings, given as an array or one comma-separated string.
    pub fn list(&self, key: &str) -> Vec<String> {
        match self.0.get(key) {
            Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            Some(Value::String(s)) => s.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            _ => Vec::new(),
        }
    }
}

/// Pretty JSON for the model.
pub(crate) fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

/// A path segment, escaped (ids are safe, but calendar ids are emails).
pub(crate) fn segment(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>().replace('+', "%20")
}

/// Connectors made before Yoru spoke to Google itself pointed at Google's
/// MCP servers: point them at the REST APIs, keeping the user's client.
/// Their tokens and tool policies were for those servers, so they go: the
/// user signs in again. Returns the ids changed, whose keychain entries the
/// caller clears.
pub(crate) fn migrate(file: &mut ConnectorsFile) -> Vec<String> {
    let mut changed = Vec::new();
    for c in &mut file.connectors {
        let TransportConfig::Http { url } = &c.transport else { continue };
        let service = if url.contains("gmailmcp.googleapis.com") {
            GoogleService::Gmail
        } else if url.contains("calendarmcp.googleapis.com") {
            GoogleService::Calendar
        } else {
            continue;
        };
        migrate_one(c, service);
        changed.push(c.id.clone());
    }
    changed
}

fn migrate_one(c: &mut ConnectorConfig, service: GoogleService) {
    c.transport = TransportConfig::Google { service };
    let old = c.oauth.take().unwrap_or_default();
    // A "Web application" client only takes the port it was registered with.
    c.oauth = Some(OAuthConfig { client_id: old.client_id, redirect_port: old.redirect_port, ..Default::default() });
    c.tools.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_are_valid_and_say_what_they_do() {
        for service in [GoogleService::Gmail, GoogleService::Calendar] {
            let tools = service.tools();
            assert!(!tools.is_empty());
            for t in &tools {
                let mcp = t.to_mcp();
                assert!(super::super::names::qualify("google-calendar", &mcp.name).len() <= 64, "{}", t.name);
                for r in t.required {
                    assert!(t.properties.get(r).is_some(), "{}: {r}", t.name);
                }
                assert!(!(t.read_only && t.destructive), "{}", t.name);
            }
        }
    }

    #[test]
    fn old_mcp_connectors_move_to_the_rest_apis() {
        let mut file: ConnectorsFile = serde_json::from_value(json!({"connectors": [
            {"id": "gmail", "name": "Gmail", "transport": {"type": "http", "url": "https://gmailmcp.googleapis.com/mcp/v1"},
             "oauth": {"client_id": "cid", "auth_params": {"access_type": "offline"}, "redirect_port": 8765},
             "tools": {"trash_thread": {"enabled": false, "acts": true}}},
            {"id": "notion", "name": "Notion", "transport": {"type": "http", "url": "https://mcp.notion.com/mcp"}}
        ]}))
        .unwrap();
        assert_eq!(migrate(&mut file), vec!["gmail".to_string()]);
        let gmail = &file.connectors[0];
        assert_eq!(gmail.transport, TransportConfig::Google { service: GoogleService::Gmail });
        let oauth = gmail.oauth.as_ref().unwrap();
        assert_eq!((oauth.client_id.as_deref(), oauth.redirect_port), (Some("cid"), Some(8765)));
        assert!(oauth.auth_params.is_empty() && gmail.tools.is_empty());
        gmail.validate().unwrap();
        assert!(migrate(&mut file).is_empty());
    }

    #[test]
    fn arguments_are_read_leniently() {
        let args = Args(json!({"n": "5", "b": "true", "to": "a@x.it, b@y.it", "cc": ["c@z.it"], "blank": "  "}).as_object().unwrap().clone());
        assert_eq!(args.int("n"), Some(5));
        assert_eq!(args.bool("b"), Some(true));
        assert_eq!(args.list("to"), vec!["a@x.it", "b@y.it"]);
        assert_eq!(args.list("cc"), vec!["c@z.it"]);
        assert_eq!(args.str("blank"), None);
        assert!(args.need("blank").is_err());
    }

    #[test]
    fn calendar_ids_are_escaped() {
        assert_eq!(segment("it.italian#holiday@group.v.calendar.google.com"), "it.italian%23holiday%40group.v.calendar.google.com");
        assert_eq!(segment("a b"), "a%20b");
    }
}
