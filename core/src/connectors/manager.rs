//! [`Connectors`]: the connectors' sessions, behind a blocking API.
//!
//! A tokio runtime of its own runs one supervisor task per enabled
//! connector. The supervisor connects, lists the tools, keeps the session
//! until it closes, and reconnects with a backoff (1 s doubling to 5 min).
//! A server that answers 401 leaves the connector in `needs_login` until
//! [`Connectors::login`]; if it had tokens, that means refreshing them failed
//! (rmcp's `AuthClient` refreshes them on its own before they expire and
//! after a 401), and [`ConnectorEvent::LoginNeeded`] is sent.
//!
//! Changing a connector (add, update, enable, login, logout, a secret)
//! restarts its supervisor; the state is kept in [`Shared`] under plain
//! mutexes, never held across an await, and events are sent outside them.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rmcp::model::{CallToolRequestParams, CallToolResponse, CallToolResult, ClientCapabilities, ClientConfig, ContentBlock, Implementation, ResourceContents, Tool};
use rmcp::service::{ClientInitializeError, NotificationContext, RunningService, ServiceError};
use rmcp::transport::auth::{AuthClient, AuthError, AuthorizationManager, OAuthClientConfig};
use rmcp::transport::streamable_http_client::{AuthRequiredError, InsufficientScopeError, StreamableHttpClient, StreamableHttpClientTransportConfig};
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{ClientHandler, Peer, RoleClient, ServiceExt};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::runtime::Runtime;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use super::config::{ConnectorConfig, ConnectorsFile, EnvValue, ToolPolicy, TransportConfig};
use super::login::{self, LoginParams};
use super::store::{self, ConnectorCredentials, ConnectorStates, Keychain, SecretStore};
use super::{ConnectionState, ConnectorError, ConnectorEvent, ConnectorStatus, ConnectorTool, ToolInfo, ToolOutput, names};

const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(5 * 60);
/// A session that lasted this long was fine: the next failure starts the
/// backoff over.
const STABLE: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const LIST_TIMEOUT: Duration = Duration::from_secs(30);
/// Lines of a local server's stderr kept to explain why it failed.
const STDERR_LINES: usize = 20;

type Events = Box<dyn Fn(ConnectorEvent) + Send + Sync>;

/// The user's connectors and their sessions. Every method blocks; see the
/// [module docs](super).
pub struct Connectors {
    shared: Arc<Shared>,
    runtime: Option<Runtime>,
}

struct Shared {
    dir: PathBuf,
    secrets: Arc<dyn SecretStore>,
    events: Events,
    file: Mutex<ConnectorsFile>,
    live: Mutex<HashMap<String, Live>>,
    /// Sign-ins in progress, to cancel them.
    logins: Mutex<HashMap<String, Arc<Notify>>>,
}

/// A connector's session as it is now.
struct Live {
    state: ConnectionState,
    /// As last listed; kept while reconnecting so the app still shows them.
    tools: Vec<Tool>,
    peer: Option<Peer<RoleClient>>,
    task: Option<JoinHandle<()>>,
    /// The `WWW-Authenticate` header of the last 401, for the next sign-in.
    challenge: Option<String>,
}

impl Default for Live {
    fn default() -> Self {
        Self { state: ConnectionState::Connecting, tools: Vec::new(), peer: None, task: None, challenge: None }
    }
}

impl Connectors {
    /// Load `connectors.json` from `data_dir` and connect every enabled
    /// connector, with secrets in the system keychain. `events` is called
    /// from the runtime's threads.
    pub fn open(data_dir: PathBuf, events: impl Fn(ConnectorEvent) + Send + Sync + 'static) -> Result<Self, ConnectorError> {
        Self::open_with_secrets(data_dir, Arc::new(Keychain), events)
    }

    /// As [`Self::open`], with secrets kept somewhere else (tests use
    /// [`super::MemorySecrets`]).
    pub fn open_with_secrets(
        data_dir: PathBuf,
        secrets: Arc<dyn SecretStore>,
        events: impl Fn(ConnectorEvent) + Send + Sync + 'static,
    ) -> Result<Self, ConnectorError> {
        let file = ConnectorsFile::load(&data_dir)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("yoru-connectors")
            .enable_all()
            .build()?;
        let shared = Arc::new(Shared {
            dir: data_dir,
            secrets,
            events: Box::new(events),
            file: Mutex::new(file),
            live: Mutex::default(),
            logins: Mutex::default(),
        });
        let this = Self { shared, runtime: Some(runtime) };
        for id in this.ids() {
            this.restart(&id);
        }
        Ok(this)
    }

    fn runtime(&self) -> &Runtime {
        self.runtime.as_ref().expect("runtime lives as long as Connectors")
    }

    /// Run `fut` on the runtime and wait for it. Not `block_on`, which
    /// panics when the caller is itself inside a tokio runtime.
    fn run<T: Send + 'static>(&self, fut: impl Future<Output = T> + Send + 'static) -> T {
        let (tx, rx) = std::sync::mpsc::channel();
        self.runtime().spawn(async move {
            let _ = tx.send(fut.await);
        });
        rx.recv().expect("connector task panicked")
    }

    fn ids(&self) -> Vec<String> {
        self.shared.file.lock().unwrap().connectors.iter().map(|c| c.id.clone()).collect()
    }

    fn config(&self, id: &str) -> Result<ConnectorConfig, ConnectorError> {
        self.shared.config(id).ok_or_else(|| ConnectorError::UnknownConnector(id.into()))
    }

    /// Every connector, in the file's order.
    pub fn list(&self) -> Vec<ConnectorStatus> {
        let configs = self.shared.file.lock().unwrap().connectors.clone();
        configs.iter().map(|c| self.status(c)).collect()
    }

    /// One connector.
    pub fn get(&self, id: &str) -> Result<ConnectorStatus, ConnectorError> {
        Ok(self.status(&self.config(id)?))
    }

    fn status(&self, c: &ConnectorConfig) -> ConnectorStatus {
        let (state, tools) = {
            let live = self.shared.live.lock().unwrap();
            match live.get(&c.id) {
                Some(l) => (l.state.clone(), l.tools.iter().map(|t| tool_info(c, t)).collect()),
                None => (ConnectionState::Disabled, Vec::new()),
            }
        };
        let target = match &c.transport {
            TransportConfig::Http { url } => url.clone(),
            TransportConfig::Stdio { command, args, .. } => std::iter::once(command.as_str()).chain(args.iter().map(String::as_str)).collect::<Vec<_>>().join(" "),
        };
        let signed_in = matches!(c.transport, TransportConfig::Http { .. }) && ConnectorCredentials::new(&c.id, self.shared.secrets.clone()).has_token();
        ConnectorStatus { id: c.id.clone(), name: c.name.clone(), enabled: c.enabled, state, transport: c.transport.kind(), target, signed_in, tools }
    }

    /// Add a connector and connect it. A client secret in its OAuth config
    /// goes to the keychain.
    pub fn add(&self, mut config: ConnectorConfig) -> Result<ConnectorStatus, ConnectorError> {
        config.validate()?;
        self.keep_client_secret(&mut config)?;
        {
            let mut file = self.shared.file.lock().unwrap();
            if file.get(&config.id).is_some() {
                return Err(ConnectorError::AlreadyExists(config.id));
            }
            file.connectors.push(config.clone());
            file.save(&self.shared.dir)?;
        }
        self.restart(&config.id);
        self.get(&config.id)
    }

    /// Replace a connector's config (same id) and reconnect it. The tool
    /// policies are kept when the new config has none.
    pub fn update(&self, mut config: ConnectorConfig) -> Result<ConnectorStatus, ConnectorError> {
        config.validate()?;
        self.keep_client_secret(&mut config)?;
        {
            let mut file = self.shared.file.lock().unwrap();
            let old = file.get_mut(&config.id).ok_or_else(|| ConnectorError::UnknownConnector(config.id.clone()))?;
            if config.tools.is_empty() {
                config.tools = std::mem::take(&mut old.tools);
            }
            *old = config.clone();
            file.save(&self.shared.dir)?;
        }
        self.restart(&config.id);
        self.get(&config.id)
    }

    fn keep_client_secret(&self, config: &mut ConnectorConfig) -> Result<(), ConnectorError> {
        if let Some(secret) = config.oauth.as_mut().and_then(|o| o.client_secret.take()) {
            self.shared.secrets.set(&store::client_secret_account(&config.id), &secret)?;
        }
        Ok(())
    }

    /// Remove a connector, closing its session and deleting its keychain
    /// entries (tokens, client secret). Named secrets stay: other
    /// connectors may use them ([`Self::delete_secret`]).
    pub fn remove(&self, id: &str) -> Result<(), ConnectorError> {
        {
            let mut file = self.shared.file.lock().unwrap();
            let before = file.connectors.len();
            file.connectors.retain(|c| c.id != id);
            if file.connectors.len() == before {
                return Err(ConnectorError::UnknownConnector(id.into()));
            }
            file.save(&self.shared.dir)?;
        }
        if let Some(live) = self.shared.live.lock().unwrap().remove(id)
            && let Some(task) = live.task
        {
            task.abort();
        }
        for account in store::connector_accounts(id) {
            self.shared.secrets.delete(&account)?;
        }
        Ok(())
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), ConnectorError> {
        self.edit(id, |c| c.enabled = enabled)?;
        self.restart(id);
        Ok(())
    }

    /// Turn a tool on or off for the agent, and say whether it acts. The
    /// session isn't touched.
    pub fn set_tool_policy(&self, id: &str, tool: &str, enabled: bool, acts: bool) -> Result<(), ConnectorError> {
        self.edit(id, |c| {
            c.tools.insert(tool.to_string(), ToolPolicy { enabled, acts });
        })?;
        (self.shared.events)(ConnectorEvent::ToolsChanged { id: id.into() });
        Ok(())
    }

    fn edit(&self, id: &str, f: impl FnOnce(&mut ConnectorConfig)) -> Result<(), ConnectorError> {
        let mut file = self.shared.file.lock().unwrap();
        f(file.get_mut(id).ok_or_else(|| ConnectorError::UnknownConnector(id.into()))?);
        Ok(file.save(&self.shared.dir)?)
    }

    /// Sign in to a remote connector in the browser: `open_url` is called
    /// with the page to open, then this waits (5 minutes at most) for the
    /// user to finish there. The connector reconnects with the new tokens.
    pub fn login(&self, id: &str, open_url: &dyn Fn(&str)) -> Result<(), ConnectorError> {
        self.login_with_timeout(id, open_url, login::LOGIN_TIMEOUT)
    }

    fn login_with_timeout(&self, id: &str, open_url: &dyn Fn(&str), timeout: Duration) -> Result<(), ConnectorError> {
        let config = self.config(id)?;
        let TransportConfig::Http { url } = &config.transport else { return Err(ConnectorError::NoLogin(id.into())) };
        let params = LoginParams {
            id: id.into(),
            name: config.name.clone(),
            url: url.clone(),
            oauth: config.oauth.clone(),
            client_secret: self.shared.secrets.get(&store::client_secret_account(id))?,
            challenge: self.shared.live.lock().unwrap().get(id).and_then(|l| l.challenge.clone()),
            secrets: self.shared.secrets.clone(),
        };
        let cancel = Arc::new(Notify::new());
        self.shared.logins.lock().unwrap().insert(id.into(), cancel.clone());
        let result = self.run(login::prepare(params)).and_then(|prepared| {
            open_url(&prepared.auth_url);
            self.run(login::finish(prepared, cancel, timeout))
        });
        self.shared.logins.lock().unwrap().remove(id);
        if result.is_ok() {
            self.restart(id);
        }
        result
    }

    /// Stop a [`Self::login`] waiting for the browser; it returns
    /// [`ConnectorError::LoginCancelled`].
    pub fn cancel_login(&self, id: &str) {
        if let Some(cancel) = self.shared.logins.lock().unwrap().get(id) {
            cancel.notify_one();
        }
    }

    /// Forget a connector's tokens (the client registration goes with them)
    /// and reconnect: a server that needs a sign-in then says so.
    pub fn logout(&self, id: &str) -> Result<(), ConnectorError> {
        let config = self.config(id)?;
        if !matches!(config.transport, TransportConfig::Http { .. }) {
            return Err(ConnectorError::NoLogin(id.into()));
        }
        self.shared.secrets.delete(&store::credentials_account(id))?;
        self.shared.secrets.delete(&store::state_account(id))?;
        self.restart(id);
        Ok(())
    }

    /// Store a named secret for local servers (`{"secret": NAME}` in their
    /// env) and reconnect those that use it.
    pub fn set_secret(&self, name: &str, value: &str) -> Result<(), ConnectorError> {
        let account = store::named_account(name).ok_or_else(|| ConnectorError::InvalidConfig(format!("secret name {name:?}")))?;
        self.shared.secrets.set(&account, value)?;
        self.restart_users_of(name);
        Ok(())
    }

    pub fn delete_secret(&self, name: &str) -> Result<(), ConnectorError> {
        let account = store::named_account(name).ok_or_else(|| ConnectorError::InvalidConfig(format!("secret name {name:?}")))?;
        self.shared.secrets.delete(&account)?;
        self.restart_users_of(name);
        Ok(())
    }

    /// Whether a named secret is set (its value never leaves the keychain
    /// but to the child process).
    pub fn has_secret(&self, name: &str) -> Result<bool, ConnectorError> {
        let account = store::named_account(name).ok_or_else(|| ConnectorError::InvalidConfig(format!("secret name {name:?}")))?;
        Ok(self.shared.secrets.get(&account)?.is_some())
    }

    fn restart_users_of(&self, secret: &str) {
        let users: Vec<String> = {
            let file = self.shared.file.lock().unwrap();
            file.connectors.iter().filter(|c| c.transport.secret_names().contains(&secret)).map(|c| c.id.clone()).collect()
        };
        for id in users {
            self.restart(&id);
        }
    }

    /// The tools offered to the agent: enabled, of connected connectors.
    pub fn tools(&self) -> Vec<ConnectorTool> {
        let configs = self.shared.file.lock().unwrap().connectors.clone();
        let live = self.shared.live.lock().unwrap();
        configs
            .iter()
            .filter_map(|c| live.get(&c.id).filter(|l| l.state == ConnectionState::Connected).map(|l| (c, l)))
            .flat_map(|(c, l)| {
                l.tools.iter().map(|t| tool_info(c, t)).filter(|t| t.enabled).map(|tool| ConnectorTool {
                    connector_id: c.id.clone(),
                    connector_name: c.name.clone(),
                    tool,
                })
            })
            .collect()
    }

    /// The connector and tool a qualified name stands for, among the tools
    /// listed (offered or not).
    pub fn resolve(&self, qualified_name: &str) -> Option<(String, ToolInfo)> {
        let configs = self.shared.file.lock().unwrap().connectors.clone();
        let live = self.shared.live.lock().unwrap();
        configs.iter().find_map(|c| {
            let l = live.get(&c.id)?;
            l.tools.iter().map(|t| tool_info(c, t)).find(|t| t.qualified_name == qualified_name).map(|t| (c.id.clone(), t))
        })
    }

    /// Call a tool by its qualified name. A tool that reports a failure is
    /// an `Ok` with `is_error`; the agent should see it.
    pub fn call(&self, qualified_name: &str, arguments: Value, timeout: Duration) -> Result<ToolOutput, ConnectorError> {
        let (id, tool) = self.resolve(qualified_name).ok_or_else(|| ConnectorError::UnknownTool(qualified_name.into()))?;
        if !tool.enabled {
            return Err(ConnectorError::ToolDisabled(qualified_name.into()));
        }
        let peer = {
            let live = self.shared.live.lock().unwrap();
            let l = live.get(&id).ok_or_else(|| ConnectorError::UnknownConnector(id.clone()))?;
            match (&l.state, &l.peer) {
                (ConnectionState::Connected, Some(peer)) => peer.clone(),
                (ConnectionState::NeedsLogin, _) => return Err(ConnectorError::NeedsLogin(id)),
                (state, _) => return Err(ConnectorError::NotConnected(format!("{id} is {}", state_word(state)))),
            }
        };
        let arguments = match arguments {
            Value::Object(map) => map,
            Value::Null => Default::default(),
            other => return Err(ConnectorError::Call(format!("arguments must be a JSON object, not {other}"))),
        };
        let params = CallToolRequestParams::new(tool.name.clone()).with_arguments(arguments);
        let shared = self.shared.clone();
        let qualified_name = qualified_name.to_string();
        self.run(async move {
            let result = tokio::time::timeout(timeout, peer.call_tool_once(params))
                .await
                .map_err(|_| ConnectorError::Timeout(format!("{qualified_name} after {} s", timeout.as_secs())))?;
            match result {
                Ok(CallToolResponse::Complete(result)) => Ok(output(result)),
                Ok(CallToolResponse::InputRequired(_)) => Err(ConnectorError::Call("the tool asked for more input, which Yoru can't give yet".into())),
                Ok(CallToolResponse::Task(_)) => Err(ConnectorError::Call("the tool started a long task, which Yoru can't follow yet".into())),
                Ok(_) => Err(ConnectorError::Call("unexpected response".into())),
                Err(e) => Err(shared.call_failed(&id, e)),
            }
        })
    }

    /// Start (or stop) a connector's supervisor as its config says.
    fn restart(&self, id: &str) {
        let Some(config) = self.shared.config(id) else { return };
        let old = {
            let mut live = self.shared.live.lock().unwrap();
            let l = live.entry(id.to_string()).or_default();
            l.peer = None;
            l.task.take()
        };
        if let Some(task) = old {
            task.abort();
        }
        if !config.enabled {
            self.shared.set_state(id, ConnectionState::Disabled);
            return;
        }
        // Set now, not by the task: a caller asking right after sees the restart.
        self.shared.set_state(id, ConnectionState::Connecting);
        let task = self.runtime().spawn(supervise(self.shared.clone(), id.to_string()));
        if let Some(l) = self.shared.live.lock().unwrap().get_mut(id) {
            l.task = Some(task);
        }
    }

    /// Wait until `id` leaves `connecting`, for tests and the CLI, which
    /// want the tools before they ask.
    pub fn wait_ready(&self, id: &str, timeout: Duration) -> ConnectionState {
        let end = Instant::now() + timeout;
        loop {
            let state = self.shared.live.lock().unwrap().get(id).map(|l| l.state.clone()).unwrap_or(ConnectionState::Disabled);
            if state != ConnectionState::Connecting || Instant::now() >= end {
                return state;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Connectors {
    fn drop(&mut self) {
        // Allowed even inside another runtime, unlike dropping it.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

impl Shared {
    fn config(&self, id: &str) -> Option<ConnectorConfig> {
        self.file.lock().unwrap().get(id).cloned()
    }

    fn set_state(&self, id: &str, state: ConnectionState) {
        let changed = {
            let mut live = self.live.lock().unwrap();
            let l = live.entry(id.to_string()).or_default();
            if state != ConnectionState::Connected {
                l.peer = None;
            }
            let changed = l.state != state;
            l.state = state.clone();
            changed
        };
        if changed {
            (self.events)(ConnectorEvent::StateChanged { id: id.into(), state });
        }
    }

    fn set_tools(&self, id: &str, tools: Vec<Tool>) {
        let changed = {
            let mut live = self.live.lock().unwrap();
            let l = live.entry(id.to_string()).or_default();
            let changed = l.tools != tools;
            l.tools = tools;
            changed
        };
        if changed {
            (self.events)(ConnectorEvent::ToolsChanged { id: id.into() });
        }
    }

    fn set_connected(&self, id: &str, peer: Peer<RoleClient>) {
        if let Some(l) = self.live.lock().unwrap().get_mut(id) {
            l.peer = Some(peer);
            l.challenge = None;
        }
        self.set_state(id, ConnectionState::Connected);
    }

    fn set_needs_login(&self, id: &str, challenge: Option<String>, had_token: bool) {
        if let Some(l) = self.live.lock().unwrap().get_mut(id) {
            l.challenge = challenge;
        }
        self.set_state(id, ConnectionState::NeedsLogin);
        if had_token {
            (self.events)(ConnectorEvent::LoginNeeded { id: id.into() });
        }
    }

    /// A failed call: a 401 that refreshing didn't fix means signing in
    /// again; the session will notice the rest on its own.
    fn call_failed(&self, id: &str, e: ServiceError) -> ConnectorError {
        if let Some(challenge) = auth_challenge(&e) {
            self.set_needs_login(id, challenge, true);
            if let Some(l) = self.live.lock().unwrap().get_mut(id)
                && let Some(task) = l.task.take()
            {
                task.abort();
            }
            return ConnectorError::NeedsLogin(id.into());
        }
        ConnectorError::Call(e.to_string())
    }
}

/// Why connecting failed.
enum ConnectFailure {
    NeedsLogin { challenge: Option<String>, had_token: bool },
    Other(String),
}

/// Keep connector `id` connected until the task is aborted.
async fn supervise(shared: Arc<Shared>, id: String) {
    let mut delay = MIN_BACKOFF;
    loop {
        let Some(config) = shared.config(&id) else { return };
        shared.set_state(&id, ConnectionState::Connecting);
        let changed = Arc::new(Notify::new());
        match connect(&shared, &config, changed.clone()).await {
            Ok(running) => {
                let started = Instant::now();
                let message = match serve(&shared, &id, running, changed).await {
                    Ok(()) => "the session closed".to_string(),
                    Err(why) => why,
                };
                shared.set_state(&id, ConnectionState::Error { message });
                if started.elapsed() >= STABLE {
                    delay = MIN_BACKOFF;
                }
            }
            Err(ConnectFailure::NeedsLogin { challenge, had_token }) => {
                shared.set_needs_login(&id, challenge, had_token);
                return;
            }
            Err(ConnectFailure::Other(message)) => shared.set_state(&id, ConnectionState::Error { message }),
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(MAX_BACKOFF);
    }
}

/// Use a session until it closes, listing the tools again whenever the
/// server says they changed.
async fn serve(shared: &Shared, id: &str, running: RunningService<RoleClient, Handler>, changed: Arc<Notify>) -> Result<(), String> {
    let peer = running.peer().clone();
    shared.set_tools(id, list_tools(&peer).await?);
    shared.set_connected(id, peer.clone());
    let closed = running.waiting();
    tokio::pin!(closed);
    loop {
        tokio::select! {
            _ = &mut closed => return Ok(()),
            _ = changed.notified() => match list_tools(&peer).await {
                Ok(tools) => shared.set_tools(id, tools),
                Err(why) => return Err(why),
            },
        }
    }
}

async fn list_tools(peer: &Peer<RoleClient>) -> Result<Vec<Tool>, String> {
    match tokio::time::timeout(LIST_TIMEOUT, peer.list_all_tools()).await {
        Ok(Ok(tools)) => Ok(tools),
        Ok(Err(e)) => Err(format!("listing the tools: {e}")),
        Err(_) => Err("listing the tools timed out".into()),
    }
}

async fn connect(shared: &Shared, config: &ConnectorConfig, changed: Arc<Notify>) -> Result<RunningService<RoleClient, Handler>, ConnectFailure> {
    let handler = Handler { changed };
    match &config.transport {
        TransportConfig::Http { url } => connect_http(shared, config, url, handler).await,
        TransportConfig::Stdio { command, args, env } => connect_stdio(shared, command, args, env, handler).await,
    }
}

async fn connect_http(shared: &Shared, config: &ConnectorConfig, url: &str, handler: Handler) -> Result<RunningService<RoleClient, Handler>, ConnectFailure> {
    let credentials = ConnectorCredentials::new(&config.id, shared.secrets.clone());
    let had_token = {
        let c = credentials.clone();
        tokio::task::spawn_blocking(move || c.has_token()).await.unwrap_or(false)
    };
    let transport_config = StreamableHttpClientTransportConfig::with_uri(url);
    if !had_token {
        // Never signed in: a plain client, so a server without auth never
        // touches the keychain; one that wants a sign-in answers 401.
        let transport = StreamableHttpClientTransport::with_client(reqwest::Client::new(), transport_config);
        return serve_http(handler, transport, false).await;
    }
    let mut manager = AuthorizationManager::new(url).await.map_err(|e| ConnectFailure::Other(e.to_string()))?;
    manager.set_credential_store(credentials);
    manager.set_state_store(ConnectorStates::new(&config.id, shared.secrets.clone()));
    authorize_from_store(shared, config, url, &mut manager).await?;
    let transport = StreamableHttpClientTransport::with_client(AuthClient::new(reqwest::Client::new(), manager), transport_config);
    serve_http(handler, transport, true).await
}

async fn serve_http<C: StreamableHttpClient>(
    handler: Handler,
    transport: StreamableHttpClientTransport<C>,
    had_token: bool,
) -> Result<RunningService<RoleClient, Handler>, ConnectFailure> {
    match tokio::time::timeout(CONNECT_TIMEOUT, handler.serve(transport)).await {
        Err(_) => Err(ConnectFailure::Other("the server didn't answer in time".into())),
        Ok(Ok(running)) => Ok(running),
        Ok(Err(e)) => match init_auth_challenge(&e) {
            Some(challenge) => Err(ConnectFailure::NeedsLogin { challenge, had_token }),
            None => Err(ConnectFailure::Other(e.to_string())),
        },
    }
}

/// Set up the manager with the stored client, so rmcp can use and refresh
/// the stored tokens. Our own client's secret is put back too: rmcp keeps
/// only the client id, and a confidential client can't refresh without it.
async fn authorize_from_store(shared: &Shared, config: &ConnectorConfig, url: &str, manager: &mut AuthorizationManager) -> Result<(), ConnectFailure> {
    if let Some(metadata) = config.oauth.as_ref().and_then(login::explicit_metadata) {
        manager.set_metadata(metadata);
    }
    match manager.initialize_from_store().await {
        Ok(true) => {}
        Ok(false) => return Ok(()),
        Err(e) => return Err(ConnectFailure::Other(format!("OAuth metadata: {e}"))),
    }
    let secrets = shared.secrets.clone();
    let account = store::client_secret_account(&config.id);
    let secret = tokio::task::spawn_blocking(move || secrets.get(&account)).await.ok().and_then(Result::ok).flatten();
    if let Some(secret) = secret {
        let client_id = manager.get_credentials().await.map(|(id, _)| id).map_err(|e| ConnectFailure::Other(e.to_string()))?;
        manager
            .configure_client(OAuthClientConfig::new(client_id, url).with_client_secret(secret))
            .map_err(|e| ConnectFailure::Other(e.to_string()))?;
    }
    Ok(())
}

async fn connect_stdio(
    shared: &Shared,
    command: &str,
    args: &[String],
    env: &std::collections::BTreeMap<String, EnvValue>,
    handler: Handler,
) -> Result<RunningService<RoleClient, Handler>, ConnectFailure> {
    let mut cmd = tokio::process::Command::new(command);
    cmd.args(args);
    for (key, value) in env {
        cmd.env(key, resolve_env(shared, value).await?);
    }
    // No console window per server when the app runs them on Windows.
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let (process, stderr) = TokioChildProcess::builder(cmd)
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| ConnectFailure::Other(format!("can't run {command}: {e}")))?;
    let tail = Arc::new(Mutex::new(VecDeque::new()));
    if let Some(stderr) = stderr {
        tokio::spawn(keep_tail(stderr, tail.clone()));
    }
    let failed = |e: String| {
        let last = tail.lock().unwrap().back().cloned();
        ConnectFailure::Other(match last {
            Some(line) => format!("{e} ({command} said: {line})"),
            None => e,
        })
    };
    match tokio::time::timeout(CONNECT_TIMEOUT, handler.serve(process)).await {
        Err(_) => Err(failed(format!("{command} didn't answer in time"))),
        Ok(Ok(running)) => Ok(running),
        Ok(Err(e)) => Err(failed(e.to_string())),
    }
}

async fn resolve_env(shared: &Shared, value: &EnvValue) -> Result<String, ConnectFailure> {
    match value {
        EnvValue::Plain(v) => Ok(v.clone()),
        EnvValue::Secret { secret } => {
            let account = store::named_account(secret).ok_or_else(|| ConnectFailure::Other(format!("bad secret name {secret:?}")))?;
            let secrets = shared.secrets.clone();
            match tokio::task::spawn_blocking(move || secrets.get(&account)).await {
                Ok(Ok(Some(v))) => Ok(v),
                Ok(Ok(None)) => Err(ConnectFailure::Other(format!("the secret {secret} is not set"))),
                Ok(Err(e)) => Err(ConnectFailure::Other(e.to_string())),
                Err(e) => Err(ConnectFailure::Other(e.to_string())),
            }
        }
    }
}

/// Keep the last lines a local server wrote to stderr.
async fn keep_tail(stderr: tokio::process::ChildStderr, tail: Arc<Mutex<VecDeque<String>>>) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let mut tail = tail.lock().unwrap();
        if tail.len() == STDERR_LINES {
            tail.pop_front();
        }
        tail.push_back(line);
    }
}

/// Our side of each session: who we are, and when the server's tools change.
#[derive(Clone)]
struct Handler {
    changed: Arc<Notify>,
}

impl ClientHandler for Handler {
    fn get_info(&self) -> ClientConfig {
        let info = Implementation::new("yoru", env!("CARGO_PKG_VERSION")).with_title("Yoru");
        ClientConfig::new(ClientCapabilities::default(), info)
    }

    async fn on_tool_list_changed(&self, _context: NotificationContext<RoleClient>) {
        self.changed.notify_one();
    }
}

/// The 401/403 challenge behind a failed initialization, if that's why.
/// `Some(None)` is a sign-in needed without a challenge to go on.
fn init_auth_challenge(e: &ClientInitializeError) -> Option<Option<String>> {
    if let Some(challenge) = e.auth_challenge() {
        return Some(Some(challenge.to_string()));
    }
    auth_in_chain(e)
}

fn auth_challenge(e: &ServiceError) -> Option<Option<String>> {
    auth_in_chain(e)
}

fn auth_in_chain(e: &(dyn std::error::Error + 'static)) -> Option<Option<String>> {
    let mut source = Some(e);
    while let Some(current) = source {
        if let Some(a) = current.downcast_ref::<AuthRequiredError>() {
            return Some(Some(a.www_authenticate_header.clone()));
        }
        if let Some(a) = current.downcast_ref::<InsufficientScopeError>() {
            return Some(Some(a.www_authenticate_header.clone()));
        }
        if let Some(AuthError::AuthorizationRequired) = current.downcast_ref::<AuthError>() {
            return Some(None);
        }
        source = current.source();
    }
    None
}

fn state_word(state: &ConnectionState) -> String {
    match state {
        ConnectionState::Disabled => "turned off".into(),
        ConnectionState::NeedsLogin => "waiting for a sign-in".into(),
        ConnectionState::Connecting => "connecting".into(),
        ConnectionState::Connected => "connected".into(),
        ConnectionState::Error { message } => format!("failing: {message}"),
    }
}

/// A listed tool with the connector's policy for it.
fn tool_info(config: &ConnectorConfig, tool: &Tool) -> ToolInfo {
    let read_only = tool.annotations.as_ref().and_then(|a| a.read_only_hint);
    let policy = config.tools.get(tool.name.as_ref()).copied().unwrap_or_else(|| ToolPolicy::default_for(read_only));
    let destructive = tool.annotations.as_ref().and_then(|a| a.destructive_hint) != Some(false);
    ToolInfo {
        name: tool.name.to_string(),
        qualified_name: names::qualify(&config.id, &tool.name),
        title: tool.title.clone().or_else(|| tool.annotations.as_ref().and_then(|a| a.title.clone())),
        description: tool.description.as_ref().map(|d| d.to_string()),
        input_schema: Value::Object((*tool.input_schema).clone()),
        enabled: policy.enabled,
        acts: policy.acts,
        destructive: policy.acts && destructive,
    }
}

/// A tool result as the agent reads it.
pub(crate) fn output(result: CallToolResult) -> ToolOutput {
    let mut text: Vec<String> = result.content.iter().map(describe).collect();
    if text.is_empty()
        && let Some(structured) = &result.structured_content
    {
        text.push(structured.to_string());
    }
    ToolOutput { text: text.join("\n"), structured: result.structured_content, is_error: result.is_error.unwrap_or(false) }
}

fn describe(block: &ContentBlock) -> String {
    let size = |base64: &str| format!("{} KB", base64.len() * 3 / 4 / 1024);
    match block {
        ContentBlock::Text(t) => t.text.clone(),
        ContentBlock::Image(i) => format!("[image: {}, {}]", i.mime_type, size(&i.data)),
        ContentBlock::Audio(a) => format!("[audio: {}, {}]", a.mime_type, size(&a.data)),
        ContentBlock::Resource(r) => match &r.resource {
            ResourceContents::TextResourceContents { uri, text, .. } => format!("[resource {uri}]\n{text}"),
            ResourceContents::BlobResourceContents { uri, mime_type, blob, .. } => {
                format!("[resource {uri}: {}, {}]", mime_type.as_deref().unwrap_or("binary"), size(blob))
            }
            _ => "[resource]".into(),
        },
        ContentBlock::ResourceLink(l) => match &l.description {
            Some(d) => format!("[link {}: {} — {d}]", l.name, l.uri),
            None => format!("[link {}: {}]", l.name, l.uri),
        },
        _ => "[unsupported content]".into(),
    }
}
