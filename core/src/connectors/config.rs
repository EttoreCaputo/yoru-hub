//! The connectors as kept in `connectors.json` in the data folder.
//!
//! The file holds what the user chose, never a secret: OAuth tokens, a
//! client secret and the tokens a local server needs live in the keychain
//! ([`super::store`]). A client secret can still be passed in with a config
//! (from the app's form); [`super::Connectors`] moves it to the keychain and
//! it is never written here.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::ConnectorError;

/// The file's name in the data folder.
pub const FILE_NAME: &str = "connectors.json";

/// The longest id: it leaves room for the tool's name in a qualified name
/// (64 characters at most, see [`super::names`]).
pub const MAX_ID_LEN: usize = 24;

/// One service Yoru can reach.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectorConfig {
    /// A slug: lowercase letters, digits and `-`, starting with a letter or a
    /// digit. It names the connector's tools (`gmail__search`) and its
    /// keychain entries, so it never changes.
    pub id: String,
    /// What the user sees.
    pub name: String,
    pub transport: TransportConfig,
    /// How to sign in, when the service needs our own OAuth client or
    /// endpoints it doesn't publish. Without it, a server that asks for a
    /// sign-in is handled with discovery and dynamic client registration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<OAuthConfig>,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    /// The tools whose policy the user changed; the others take the default
    /// from the server's annotations ([`ToolPolicy::default_for`]).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tools: BTreeMap<String, ToolPolicy>,
}

fn enabled_by_default() -> bool {
    true
}

/// How Yoru talks to the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TransportConfig {
    /// A remote server over Streamable HTTP.
    Http { url: String },
    /// A local server, run as a child process speaking MCP on stdin/stdout.
    Stdio {
        command: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, EnvValue>,
    },
}

impl TransportConfig {
    pub fn kind(&self) -> TransportKind {
        match self {
            Self::Http { .. } => TransportKind::Http,
            Self::Stdio { .. } => TransportKind::Stdio,
        }
    }

    /// The named secrets the child process needs.
    pub fn secret_names(&self) -> Vec<&str> {
        match self {
            Self::Http { .. } => Vec::new(),
            Self::Stdio { env, .. } => env
                .values()
                .filter_map(|v| match v {
                    EnvValue::Secret { secret } => Some(secret.as_str()),
                    EnvValue::Plain(_) => None,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    Http,
    Stdio,
}

/// An environment variable of a local server: a plain value, or the name of
/// a secret in the keychain (`{"secret": "GITHUB_TOKEN"}`) so tokens stay out
/// of the file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EnvValue {
    Plain(String),
    Secret { secret: String },
}

/// Our own OAuth client for a service, or endpoints it doesn't publish.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OAuthConfig {
    /// A client registered in the service's console. Without it, the client
    /// is registered dynamically (RFC 7591) at the first sign-in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// Accepted from the caller and moved to the keychain; never written to
    /// the file (hence never serialized).
    #[serde(default, skip_serializing)]
    pub client_secret: Option<String>,
    /// The scopes to ask for; empty lets the server's metadata decide.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    /// For a service without OAuth metadata discovery: both must be given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_url: Option<String>,
    /// Extra query parameters for the authorization URL: Google returns a
    /// refresh token only with `access_type=offline` (and `prompt=consent`
    /// to get one again).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub auth_params: BTreeMap<String, String>,
    /// A fixed loopback port for the redirect, for clients whose redirect URI
    /// must match exactly (a Google "Web application" client registered with
    /// `http://127.0.0.1:<port>/callback`). Without it any free port is used,
    /// which desktop/native clients accept (RFC 8252 §7.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect_port: Option<u16>,
}

/// Whether a tool is offered to the agent, and whether calling it changes
/// something (and so asks the user first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPolicy {
    pub enabled: bool,
    pub acts: bool,
}

impl ToolPolicy {
    /// From the MCP tool annotations: only a tool that says it is read-only
    /// (`readOnlyHint: true`) reads; anything else may act, as the spec's
    /// defaults assume.
    pub fn default_for(read_only_hint: Option<bool>) -> Self {
        Self { enabled: true, acts: read_only_hint != Some(true) }
    }
}

/// The file as a whole.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConnectorsFile {
    #[serde(default)]
    pub connectors: Vec<ConnectorConfig>,
}

impl ConnectorsFile {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join(FILE_NAME)
    }

    /// No file yet is an empty list; a broken one is an error, so it isn't
    /// overwritten by the next save.
    pub fn load(dir: &Path) -> io::Result<Self> {
        match fs::read(Self::path(dir)) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    /// Written to a temporary file and renamed, so a crash never leaves half a file.
    pub fn save(&self, dir: &Path) -> io::Result<()> {
        fs::create_dir_all(dir)?;
        let json = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        let tmp = dir.join(format!("{FILE_NAME}.tmp"));
        fs::write(&tmp, json)?;
        fs::rename(tmp, Self::path(dir))
    }

    pub fn get(&self, id: &str) -> Option<&ConnectorConfig> {
        self.connectors.iter().find(|c| c.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut ConnectorConfig> {
        self.connectors.iter_mut().find(|c| c.id == id)
    }
}

impl ConnectorConfig {
    /// Check what the file can't express: the id's shape, a usable transport
    /// and complete OAuth endpoints.
    pub fn validate(&self) -> Result<(), ConnectorError> {
        let bad = |why: String| Err(ConnectorError::InvalidConfig(why));
        if !is_valid_id(&self.id) {
            return bad(format!(
                "id {:?} must be 1 to {MAX_ID_LEN} lowercase letters, digits or '-', starting with a letter or digit",
                self.id
            ));
        }
        if self.name.trim().is_empty() {
            return bad("the name is empty".into());
        }
        match &self.transport {
            TransportConfig::Http { url } => {
                let parsed = url::Url::parse(url).map_err(|e| ConnectorError::InvalidConfig(format!("url {url:?}: {e}")))?;
                if !matches!(parsed.scheme(), "https" | "http") {
                    return bad(format!("url {url:?} must be http(s)"));
                }
            }
            TransportConfig::Stdio { command, .. } if command.trim().is_empty() => return bad("the command is empty".into()),
            TransportConfig::Stdio { .. } => {
                if self.oauth.is_some() {
                    return bad("a local server takes its tokens from the environment, not OAuth".into());
                }
            }
        }
        if let Some(oauth) = &self.oauth
            && oauth.authorization_url.is_some() != oauth.token_url.is_some()
        {
            return bad("authorization_url and token_url go together".into());
        }
        Ok(())
    }
}

/// `^[a-z0-9][a-z0-9-]{0,23}$`. No `_`, so `id__tool` splits unambiguously
/// and the id is a safe keychain account name.
pub fn is_valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit())
        && id.len() <= MAX_ID_LEN
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> ConnectorsFile {
        let mut env = BTreeMap::new();
        env.insert("GITHUB_TOKEN".into(), EnvValue::Secret { secret: "GITHUB_TOKEN".into() });
        env.insert("LOG".into(), EnvValue::Plain("warn".into()));
        let mut tools = BTreeMap::new();
        tools.insert("send_email".into(), ToolPolicy { enabled: false, acts: true });
        ConnectorsFile {
            connectors: vec![
                ConnectorConfig {
                    id: "linear".into(),
                    name: "Linear".into(),
                    transport: TransportConfig::Http { url: "https://mcp.linear.app/mcp".into() },
                    oauth: None,
                    enabled: true,
                    tools,
                },
                ConnectorConfig {
                    id: "github-local".into(),
                    name: "GitHub".into(),
                    transport: TransportConfig::Stdio { command: "npx".into(), args: vec!["-y".into(), "server".into()], env },
                    oauth: None,
                    enabled: false,
                    tools: BTreeMap::new(),
                },
            ],
        }
    }

    #[test]
    fn round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(ConnectorsFile::load(dir.path()).unwrap(), ConnectorsFile::default());
        let file = sample();
        file.save(dir.path()).unwrap();
        assert_eq!(ConnectorsFile::load(dir.path()).unwrap(), file);
    }

    #[test]
    fn the_json_is_as_documented() {
        let v = serde_json::to_value(sample()).unwrap();
        assert_eq!(v["connectors"][0]["transport"], json!({"type": "http", "url": "https://mcp.linear.app/mcp"}));
        assert_eq!(v["connectors"][1]["transport"]["env"]["GITHUB_TOKEN"], json!({"secret": "GITHUB_TOKEN"}));
        assert_eq!(v["connectors"][1]["transport"]["env"]["LOG"], json!("warn"));
        assert_eq!(v["connectors"][0]["tools"]["send_email"], json!({"enabled": false, "acts": true}));
        // `enabled` defaults to true when missing.
        let c: ConnectorConfig = serde_json::from_value(json!({"id": "x", "name": "X", "transport": {"type": "http", "url": "https://x.example/mcp"}})).unwrap();
        assert!(c.enabled);
    }

    #[test]
    fn a_client_secret_is_never_written() {
        let mut c = sample().connectors.remove(0);
        c.oauth = Some(OAuthConfig { client_id: Some("abc".into()), client_secret: Some("s3cret".into()), ..Default::default() });
        let text = serde_json::to_string(&c).unwrap();
        assert!(!text.contains("s3cret"), "{text}");
        // ...but it is read from a caller.
        let back: ConnectorConfig = serde_json::from_value(json!({
            "id": "g", "name": "G", "transport": {"type": "http", "url": "https://g.example/mcp"},
            "oauth": {"client_id": "abc", "client_secret": "s3cret"}
        }))
        .unwrap();
        assert_eq!(back.oauth.unwrap().client_secret.as_deref(), Some("s3cret"));
    }

    #[test]
    fn a_broken_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(ConnectorsFile::path(dir.path()), "{not json").unwrap();
        assert!(ConnectorsFile::load(dir.path()).is_err());
    }

    #[test]
    fn ids() {
        for ok in ["gmail", "google-calendar", "a", "x1", "0day"] {
            assert!(is_valid_id(ok), "{ok}");
        }
        for bad in ["", "Gmail", "-x", "a_b", "a.b", "a b", "averyveryveryverylongidentifier"] {
            assert!(!is_valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn validation() {
        let mut c = sample().connectors.remove(0);
        assert!(c.validate().is_ok());
        c.transport = TransportConfig::Http { url: "ftp://x".into() };
        assert!(c.validate().is_err());
        c.transport = TransportConfig::Http { url: "https://x.example/mcp".into() };
        c.oauth = Some(OAuthConfig { authorization_url: Some("https://x.example/auth".into()), ..Default::default() });
        assert!(c.validate().is_err());
    }

    #[test]
    fn policy_defaults_follow_the_annotations() {
        assert_eq!(ToolPolicy::default_for(Some(true)), ToolPolicy { enabled: true, acts: false });
        assert_eq!(ToolPolicy::default_for(Some(false)), ToolPolicy { enabled: true, acts: true });
        assert_eq!(ToolPolicy::default_for(None), ToolPolicy { enabled: true, acts: true });
    }

    #[test]
    fn secret_names() {
        let file = sample();
        assert_eq!(file.connectors[1].transport.secret_names(), vec!["GITHUB_TOKEN"]);
        assert!(file.connectors[0].transport.secret_names().is_empty());
    }
}
