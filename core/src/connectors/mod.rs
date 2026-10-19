//! Connectors: Yoru as an MCP **client** of the user's services.
//!
//! A connector is one MCP server: a remote one over Streamable HTTP, usually
//! behind OAuth (Notion, Linear, GitHub…), or a local one run as a child
//! process (`npx …`, `uvx …`) with its token passed in the environment.
//! [`Connectors`] keeps a session open with each enabled connector, lists
//! their tools for the agent under qualified names (`linear__list_issues`)
//! and calls them.
//!
//! - [`config`]: the connectors as kept in `connectors.json` in the data
//!   folder the caller gives, with a policy per tool (offered or not; reads
//!   or acts, defaulting from the server's annotations).
//! - [`store`]: tokens and secrets in the system keychain, never in a file.
//! - `login`: the OAuth sign-in in the browser, with a loopback redirect
//!   (RFC 8252), discovery and dynamic client registration through rmcp.
//! - `manager`: [`Connectors`], a synchronous façade over a tokio runtime of
//!   its own: sessions, reconnection with backoff, token refresh (rmcp's
//!   `AuthClient`), and the tool list kept current on `tools/list_changed`.
//! - [`catalog`]: services the app can offer with one click.
//! - [`names`]: the qualified tool names LLM APIs accept.
//!
//! The rest of the crate is synchronous, so every method blocks; don't call
//! them from an async task (use `spawn_blocking`). The module depends on no
//! other module of the crate.
//!
//! Every public type is `Serialize` (snake_case), so an app can hand it to a
//! webview as is.

pub mod catalog;
pub mod config;
mod login;
mod manager;
pub mod names;
pub mod store;
#[cfg(test)]
mod tests;

use std::fmt;

use serde::Serialize;
use serde_json::Value;

pub use config::{ConnectorConfig, EnvValue, OAuthConfig, ToolPolicy, TransportConfig, TransportKind};
pub use manager::Connectors;
pub use store::{Keychain, MemorySecrets, SecretStore};

/// Where a connector stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ConnectionState {
    /// Turned off by the user.
    Disabled,
    /// The server wants a sign-in: call [`Connectors::login`].
    NeedsLogin,
    Connecting,
    Connected,
    /// The last attempt failed; another follows after a backoff.
    Error { message: String },
}

/// A connector as the app shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConnectorStatus {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    #[serde(flatten)]
    pub state: ConnectionState,
    pub transport: TransportKind,
    /// The URL, or the command line of a local server.
    pub target: String,
    /// Whether OAuth tokens are stored for it.
    pub signed_in: bool,
    /// Every tool the server listed last, offered or not; empty until the
    /// first connection.
    pub tools: Vec<ToolInfo>,
}

/// One tool of a connector, with the user's policy.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolInfo {
    /// The server's own name.
    pub name: String,
    /// The name the agent uses: `<connector id>__<tool>` ([`names`]).
    pub qualified_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// A JSON Schema object.
    pub input_schema: Value,
    /// Offered to the agent.
    pub enabled: bool,
    /// Calling it changes something, so the user is asked first.
    pub acts: bool,
    /// The server says it may destroy something (`destructiveHint`, which the
    /// spec assumes when it's missing); meaningful only when it acts.
    pub destructive: bool,
}

/// A tool offered to the agent: connected, enabled, with its connector.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConnectorTool {
    pub connector_id: String,
    pub connector_name: String,
    #[serde(flatten)]
    pub tool: ToolInfo,
}

/// What a tool call returned.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolOutput {
    /// The content blocks joined, images and resources summarised as text.
    pub text: String,
    /// The tool's `structuredContent`, when it gave one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured: Option<Value>,
    /// The tool ran and reported a failure (the model should see it, it's
    /// not a transport error).
    pub is_error: bool,
}

/// What changed, for the app to refresh.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ConnectorEvent {
    StateChanged {
        id: String,
        #[serde(flatten)]
        state: ConnectionState,
    },
    ToolsChanged { id: String },
    /// The tokens stopped working and refreshing them failed: the user has
    /// to sign in again.
    LoginNeeded { id: String },
}

/// What can go wrong; `kind` and `message` when serialized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum ConnectorError {
    UnknownConnector(String),
    AlreadyExists(String),
    InvalidConfig(String),
    UnknownTool(String),
    ToolDisabled(String),
    /// The connector isn't connected (its state says why).
    NotConnected(String),
    /// The connector needs a sign-in.
    NeedsLogin(String),
    /// A local server has no sign-in: its tokens are named secrets.
    NoLogin(String),
    Timeout(String),
    /// The call failed in the protocol or the transport.
    Call(String),
    Keychain(String),
    Io(String),
    /// The user said no in the browser.
    LoginDenied(String),
    /// The browser came back with another sign-in's state.
    LoginStateMismatch,
    LoginTimeout,
    LoginCancelled,
    /// The service doesn't support dynamic client registration: an OAuth
    /// client has to be created in its console and its id set.
    ClientIdRequired(String),
    OAuth(String),
}

impl fmt::Display for ConnectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownConnector(id) => write!(f, "no connector {id:?}"),
            Self::AlreadyExists(id) => write!(f, "a connector {id:?} exists already"),
            Self::InvalidConfig(why) => write!(f, "invalid connector: {why}"),
            Self::UnknownTool(name) => write!(f, "no tool {name:?}"),
            Self::ToolDisabled(name) => write!(f, "the tool {name:?} is turned off"),
            Self::NotConnected(why) => write!(f, "not connected: {why}"),
            Self::NeedsLogin(id) => write!(f, "{id} needs you to sign in again"),
            Self::NoLogin(id) => write!(f, "{id} is a local server: set its secrets instead of signing in"),
            Self::Timeout(what) => write!(f, "{what} timed out"),
            Self::Call(why) => write!(f, "the call failed: {why}"),
            Self::Keychain(why) => write!(f, "keychain: {why}"),
            Self::Io(why) => write!(f, "{why}"),
            Self::LoginDenied(why) => write!(f, "sign-in refused: {why}"),
            Self::LoginStateMismatch => write!(f, "the browser answered for another sign-in; try again"),
            Self::LoginTimeout => write!(f, "nobody signed in within 5 minutes"),
            Self::LoginCancelled => write!(f, "sign-in cancelled"),
            Self::ClientIdRequired(why) => write!(f, "{why}"),
            Self::OAuth(why) => write!(f, "sign-in failed: {why}"),
        }
    }
}

impl std::error::Error for ConnectorError {}

impl From<std::io::Error> for ConnectorError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}
