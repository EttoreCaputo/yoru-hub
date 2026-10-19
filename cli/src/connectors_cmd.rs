//! `connectors`: Yoru's MCP connectors to your services, from the command line.
//!
//!   companion connectors presets                      # services offered with one click
//!   companion connectors add --preset linear          # ...add one
//!   companion connectors add --id notion --name Notion --url https://mcp.notion.com/mcp
//!   companion connectors add --id gh --name GitHub --command npx --args -y,@modelcontextprotocol/server-github --env GITHUB_PERSONAL_ACCESS_TOKEN=secret:GITHUB_TOKEN
//!   companion connectors secret GITHUB_TOKEN          # its token, read from stdin, into the keychain
//!   companion connectors login linear                 # sign in in the browser
//!   companion connectors list
//!   companion connectors tools [linear]
//!   companion connectors call linear__list_issues '{"limit":5}'
//!   companion connectors logout linear
//!   companion connectors remove linear

use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::Subcommand;
use yoru_core::connectors::catalog::{self, Registration};
use yoru_core::connectors::{ConnectionState, ConnectorConfig, Connectors, EnvValue, OAuthConfig, TransportConfig};
use serde_json::Value;

/// How long to wait for the connectors to connect before listing them.
const READY: Duration = Duration::from_secs(20);
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Subcommand)]
pub enum Action {
    /// The connectors and their state
    List {
        #[arg(long)]
        json: bool,
    },
    /// The services that can be added with --preset
    Presets,
    /// Add a connector: a preset, a remote server (--url) or a local one (--command)
    Add {
        /// A service from `connectors presets`
        #[arg(long, conflicts_with_all = ["url", "command"])]
        preset: Option<String>,
        /// A slug: lowercase letters, digits and '-' (default: the preset's)
        #[arg(long, required_unless_present = "preset")]
        id: Option<String>,
        #[arg(long, required_unless_present = "preset")]
        name: Option<String>,
        /// A remote MCP server (Streamable HTTP)
        #[arg(long, conflicts_with = "command")]
        url: Option<String>,
        /// A local MCP server to run
        #[arg(long)]
        command: Option<String>,
        /// Its arguments, comma separated
        #[arg(long, value_delimiter = ',', allow_hyphen_values = true, requires = "command")]
        args: Vec<String>,
        /// KEY=VALUE, or KEY=secret:NAME for a secret set with `connectors secret NAME`
        #[arg(long, requires = "command")]
        env: Vec<String>,
        /// Your own OAuth client's id, for a service without dynamic registration
        #[arg(long)]
        client_id: Option<String>,
        /// Read the OAuth client's secret from stdin (it goes to the keychain)
        #[arg(long, requires = "client_id")]
        client_secret_stdin: bool,
        /// OAuth scopes to ask for (repeat)
        #[arg(long)]
        scope: Vec<String>,
    },
    /// Remove a connector and its tokens
    Remove { id: String },
    /// Sign in to a remote connector in the browser
    Login { id: String },
    /// Forget a connector's tokens
    Logout { id: String },
    /// Store a secret for local servers, read from stdin (--forget deletes it)
    Secret {
        name: String,
        #[arg(long)]
        forget: bool,
    },
    /// The tools offered to the agent, of every connector or one
    Tools {
        id: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Call a tool by its qualified name, with its arguments as one JSON object
    Call { tool: String, arguments: Option<String> },
}

pub fn run(action: Action) -> ExitCode {
    match execute(action) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn execute(action: Action) -> Result<(), String> {
    if let Action::Presets = action {
        presets();
        return Ok(());
    }
    let connectors = open()?;
    match action {
        Action::Presets => unreachable!(),
        Action::List { json } => list(&connectors, json),
        Action::Add { preset, id, name, url, command, args, env, client_id, client_secret_stdin, scope } => {
            let secret = if client_secret_stdin { Some(read_stdin("client secret")?) } else { None };
            let config = match preset {
                Some(p) => from_preset(&p, id, name, client_id, secret)?,
                None => {
                    let transport = match (url, command) {
                        (Some(url), _) => TransportConfig::Http { url },
                        (None, Some(command)) => TransportConfig::Stdio { command, args, env: parse_env(&env)? },
                        (None, None) => return Err("give --url, --command or --preset".into()),
                    };
                    let oauth = (client_id.is_some() || !scope.is_empty())
                        .then(|| OAuthConfig { client_id, client_secret: secret, scopes: scope, ..Default::default() });
                    ConnectorConfig { id: id.unwrap_or_default(), name: name.unwrap_or_default(), transport, oauth, enabled: true, tools: Default::default() }
                }
            };
            let id = config.id.clone();
            connectors.add(config).map_err(|e| e.to_string())?;
            report(&connectors, &id);
            Ok(())
        }
        Action::Remove { id } => connectors.remove(&id).map_err(|e| e.to_string()),
        Action::Login { id } => {
            connectors.login(&id, &open_browser).map_err(|e| e.to_string())?;
            println!("Signed in.");
            report(&connectors, &id);
            Ok(())
        }
        Action::Logout { id } => connectors.logout(&id).map_err(|e| e.to_string()),
        Action::Secret { name, forget } if forget => connectors.delete_secret(&name).map_err(|e| e.to_string()),
        Action::Secret { name, .. } => {
            let value = read_stdin(&name)?;
            connectors.set_secret(&name, &value).map_err(|e| e.to_string())
        }
        Action::Tools { id, json } => tools(&connectors, id.as_deref(), json),
        Action::Call { tool, arguments } => {
            let arguments: Value = match arguments {
                Some(text) => serde_json::from_str(&text).map_err(|e| format!("arguments: {e}"))?,
                None => Value::Object(Default::default()),
            };
            wait_all(&connectors);
            let out = connectors.call(&tool, arguments, CALL_TIMEOUT).map_err(|e| e.to_string())?;
            println!("{}", out.text);
            if out.is_error {
                return Err("the tool reported an error".into());
            }
            Ok(())
        }
    }
}

/// The connectors in the data folder, with secrets in the keychain.
fn open() -> Result<Connectors, String> {
    let dir = data_dir().ok_or("no data folder (HOME is not set)")?;
    Connectors::open(dir, |_| {}).map_err(|e| e.to_string())
}

/// The folder `connectors.json` lives in: the host's data folder.
fn data_dir() -> Option<PathBuf> {
    yoru_core::assistant::default_dir()
}

fn wait_all(connectors: &Connectors) {
    for c in connectors.list() {
        connectors.wait_ready(&c.id, READY);
    }
}

fn list(connectors: &Connectors, json: bool) -> Result<(), String> {
    wait_all(connectors);
    let all = connectors.list();
    if json {
        println!("{}", serde_json::to_string_pretty(&all).unwrap_or_default());
        return Ok(());
    }
    if all.is_empty() {
        println!("No connectors: add one with `connectors add` (see `connectors presets`).");
    }
    for c in all {
        let offered = c.tools.iter().filter(|t| t.enabled).count();
        println!("{:<20} {:<12} {}  ({} tools, {offered} offered)  {}", c.id, state(&c.state), c.name, c.tools.len(), c.target);
    }
    Ok(())
}

fn state(state: &ConnectionState) -> String {
    match state {
        ConnectionState::Disabled => "off".into(),
        ConnectionState::NeedsLogin => "needs login".into(),
        ConnectionState::Connecting => "connecting".into(),
        ConnectionState::Connected => "connected".into(),
        ConnectionState::Error { message } => format!("error: {message}"),
    }
}

fn report(connectors: &Connectors, id: &str) {
    let ready = connectors.wait_ready(id, READY);
    let tools = connectors.get(id).map(|c| c.tools.len()).unwrap_or(0);
    match ready {
        ConnectionState::Connected => println!("{id}: connected, {tools} tools."),
        ConnectionState::NeedsLogin => println!("{id}: sign in with `connectors login {id}`."),
        other => println!("{id}: {}", state(&other)),
    }
}

fn presets() {
    for p in catalog::presets() {
        let how = match p.registration {
            Registration::Dynamic => "sign in",
            Registration::OwnClient if p.one_click() => "sign in (Yoru's client)",
            Registration::OwnClient => "your own OAuth client (--client-id)",
            Registration::None => "no sign-in",
        };
        println!("{:<16} {:<20} {how}\n                 {}\n                 {}", p.id, p.name, p.notes, p.docs);
    }
}

fn from_preset(id: &str, own_id: Option<String>, name: Option<String>, client_id: Option<String>, secret: Option<String>) -> Result<ConnectorConfig, String> {
    let preset = catalog::preset(id).ok_or_else(|| format!("no preset {id:?}: see `connectors presets`"))?;
    if client_id.is_none() && !preset.one_click() {
        return Err(format!(
            "this build has no {} client, so it needs your own: {}\n{}\nThen add it with --client-id (and --client-secret-stdin).",
            preset.name, preset.docs, preset.setup
        ));
    }
    let mut config = preset.config(client_id, secret);
    config.id = own_id.unwrap_or(config.id);
    config.name = name.unwrap_or(config.name);
    Ok(config)
}

fn parse_env(pairs: &[String]) -> Result<BTreeMap<String, EnvValue>, String> {
    pairs
        .iter()
        .map(|pair| {
            let (key, value) = pair.split_once('=').ok_or_else(|| format!("--env {pair:?}: expected KEY=VALUE"))?;
            let value = match value.strip_prefix("secret:") {
                Some(secret) => EnvValue::Secret { secret: secret.into() },
                None => EnvValue::Plain(value.into()),
            };
            Ok((key.to_string(), value))
        })
        .collect()
}

/// One line from stdin, so a secret stays out of the shell's history.
fn read_stdin(what: &str) -> Result<String, String> {
    eprintln!("Paste the {what} and press Enter:");
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).map_err(|e| e.to_string())?;
    let value = line.trim().to_string();
    if value.is_empty() {
        return Err(format!("no {what} given"));
    }
    Ok(value)
}

fn open_browser(url: &str) {
    eprintln!("Opening the browser to sign in. If it doesn't open, go to:\n{url}");
    let status = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).status()
    } else if cfg!(windows) {
        // `start`'s first quoted argument is the window title.
        std::process::Command::new("cmd").args(["/C", "start", "", url]).status()
    } else {
        std::process::Command::new("xdg-open").arg(url).status()
    };
    if let Err(e) = status {
        eprintln!("Couldn't open the browser: {e}");
    }
}

fn tools(connectors: &Connectors, id: Option<&str>, json: bool) -> Result<(), String> {
    wait_all(connectors);
    if let Some(id) = id {
        let status = connectors.get(id).map_err(|e| e.to_string())?;
        if status.state != ConnectionState::Connected {
            return Err(format!("{id}: {}", state(&status.state)));
        }
    }
    let tools: Vec<_> = connectors.tools().into_iter().filter(|t| id.is_none_or(|id| t.connector_id == id)).collect();
    if json {
        println!("{}", serde_json::to_string_pretty(&tools).unwrap_or_default());
        return Ok(());
    }
    for t in tools {
        let kind = if t.tool.acts { "acts" } else { "reads" };
        let first_line = t.tool.description.as_deref().and_then(|d| d.lines().next()).unwrap_or_default();
        println!("{:<40} {kind:<5}  {first_line}", t.tool.qualified_name);
    }
    Ok(())
}
