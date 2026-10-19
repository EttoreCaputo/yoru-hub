//! Signing in to a connector in the browser: OAuth 2.1 authorization code
//! with PKCE and a loopback redirect (RFC 8252).
//!
//! 1. [`prepare`]: bind `127.0.0.1` (any free port, or the one the config
//!    fixes), find the server's authorization server (rmcp's discovery,
//!    seeded by the `WWW-Authenticate` challenge of the last 401 when there
//!    is one, or the endpoints the config gives), and register a client
//!    named "Yoru" when the config has no client id (dynamic client
//!    registration, RFC 7591). That gives the URL to open.
//! 2. The caller opens it in the browser.
//! 3. [`finish`]: wait for the browser to come back to
//!    `http://127.0.0.1:<port>/callback` (5 minutes at most, cancellable),
//!    check the state, exchange the code and store the tokens in the
//!    keychain (rmcp does it through [`ConnectorCredentials`]), and answer
//!    the browser with a short page.
//!
//! Nothing here can be tested end to end offline; [`parse_callback`] is the
//! part that is.

use std::sync::Arc;
use std::time::Duration;

use rmcp::transport::auth::{AuthError, AuthorizationManager, AuthorizationMetadata, AuthorizationRequest, AuthorizationSession};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

use super::config::OAuthConfig;
use super::store::{ConnectorCredentials, ConnectorStates, SecretStore};
use super::ConnectorError;

/// How long the user has to sign in.
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// How long one browser connection may take to send its request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// The client name shown on the service's consent screen.
const CLIENT_NAME: &str = "Yoru";
const CALLBACK_PATH: &str = "/callback";

/// A sign-in waiting for the browser.
pub(crate) struct Prepared {
    pub auth_url: String,
    session: AuthorizationSession,
    listener: TcpListener,
    state: String,
}

/// What a sign-in needs from the connector.
pub(crate) struct LoginParams {
    pub id: String,
    pub name: String,
    pub url: String,
    pub oauth: Option<OAuthConfig>,
    pub client_secret: Option<String>,
    /// The `WWW-Authenticate` header of the server's last 401.
    pub challenge: Option<String>,
    pub secrets: Arc<dyn SecretStore>,
}

pub(crate) async fn prepare(p: LoginParams) -> Result<Prepared, ConnectorError> {
    let oauth = p.oauth.clone().unwrap_or_default();
    let listener = TcpListener::bind(("127.0.0.1", oauth.redirect_port.unwrap_or(0))).await?;
    let redirect_uri = format!("http://127.0.0.1:{}{CALLBACK_PATH}", listener.local_addr()?.port());

    let mut manager = AuthorizationManager::new(p.url.as_str()).await.map_err(oauth_error)?;
    manager.set_credential_store(ConnectorCredentials::new(&p.id, p.secrets.clone()));
    manager.set_state_store(ConnectorStates::new(&p.id, p.secrets.clone()));
    let metadata = match explicit_metadata(&oauth) {
        Some(m) => m,
        None => manager.resolve_metadata_from_challenge(p.challenge.as_deref()).await.map_err(oauth_error)?.metadata,
    };
    manager.set_metadata(metadata);

    let mut request = AuthorizationRequest::new(redirect_uri).with_client_name(CLIENT_NAME).with_scopes(oauth.scopes.iter().cloned());
    if let Some(client_id) = &oauth.client_id {
        request = request.with_preregistered_client(client_id);
        if let Some(secret) = p.client_secret {
            request = request.with_client_secret(secret);
        }
    }
    if let Some(challenge) = p.challenge {
        request = request.with_challenge(challenge);
    }
    let session = AuthorizationSession::new(manager, request)
        .await
        .map_err(|(_, e)| session_error(e, &p.name, oauth.client_id.is_some()))?;

    let auth_url = with_params(session.get_authorization_url(), &oauth)?;
    let state = query_param(&auth_url, "state").ok_or_else(|| ConnectorError::OAuth("the authorization URL has no state".into()))?;
    Ok(Prepared { auth_url, session, listener, state })
}

/// Wait for the browser, then exchange the code.
pub(crate) async fn finish(p: Prepared, cancel: Arc<Notify>, timeout: Duration) -> Result<(), ConnectorError> {
    let waited = tokio::select! {
        r = tokio::time::timeout(timeout, wait_for_code(&p.listener, &p.state)) => r.unwrap_or(Err(ConnectorError::LoginTimeout)),
        _ = cancel.notified() => Err(ConnectorError::LoginCancelled),
    };
    let (mut browser, code, issuer) = waited?;
    let result = p
        .session
        .handle_callback_with_issuer(&code, &p.state, issuer.as_deref())
        .await
        .map(|_| ())
        .map_err(oauth_error);
    let _ = answer(&mut browser, result.as_ref().err()).await;
    result
}

/// Accept connections until one is our callback; anything else (a favicon)
/// gets a 404. A callback with an error or a wrong state ends the wait.
async fn wait_for_code(listener: &TcpListener, state: &str) -> Result<(TcpStream, String, Option<String>), ConnectorError> {
    loop {
        let (mut stream, _) = listener.accept().await?;
        let Ok(Ok(target)) = tokio::time::timeout(REQUEST_TIMEOUT, read_target(&mut stream)).await else { continue };
        match parse_callback(&target, state) {
            Callback::Other => {
                let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            }
            Callback::Code { code, issuer } => return Ok((stream, code, issuer)),
            Callback::Failed(e) => {
                let _ = answer(&mut stream, Some(&e)).await;
                return Err(e);
            }
        }
    }
}

/// The request target (`/callback?code=…`) of an HTTP request line.
async fn read_target(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 16 * 1024 {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let text = String::from_utf8_lossy(&buf);
    let line = text.lines().next().unwrap_or_default();
    let mut parts = line.split_whitespace();
    match (parts.next(), parts.next()) {
        (Some("GET"), Some(target)) => Ok(target.to_string()),
        _ => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "not a GET")),
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum Callback {
    /// Not the callback path.
    Other,
    Code { code: String, issuer: Option<String> },
    Failed(ConnectorError),
}

/// Read the redirect the browser brings back (RFC 6749 §4.1.2).
pub(crate) fn parse_callback(target: &str, expected_state: &str) -> Callback {
    let Ok(url) = url::Url::parse(&format!("http://127.0.0.1{target}")) else { return Callback::Other };
    if url.path() != CALLBACK_PATH {
        return Callback::Other;
    }
    let param = |name: &str| url.query_pairs().find(|(k, _)| k == name).map(|(_, v)| v.into_owned());
    if param("state").as_deref() != Some(expected_state) {
        return Callback::Failed(ConnectorError::LoginStateMismatch);
    }
    if let Some(error) = param("error") {
        let why = param("error_description").unwrap_or_else(|| error.clone());
        return Callback::Failed(if error == "access_denied" { ConnectorError::LoginDenied(why) } else { ConnectorError::OAuth(why) });
    }
    match param("code") {
        Some(code) if !code.is_empty() => Callback::Code { code, issuer: param("iss") },
        _ => Callback::Failed(ConnectorError::OAuth("the browser came back without a code".into())),
    }
}

/// The page the browser shows once it's done.
async fn answer(stream: &mut TcpStream, error: Option<&ConnectorError>) -> std::io::Result<()> {
    let (title, text) = match error {
        None => ("Yoru is connected", "Yoru is connected, you can close this tab.".to_string()),
        Some(e) => ("Yoru could not connect", format!("Yoru could not connect: {}. You can close this tab and try again.", html_escape(&e.to_string()))),
    };
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title>\
         <style>body{{font:16px system-ui,sans-serif;display:grid;place-items:center;height:90vh;margin:0}}</style></head>\
         <body><p>{text}</p></body></html>"
    );
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.shutdown().await
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Endpoints the config gives, for a service without discovery.
pub(crate) fn explicit_metadata(oauth: &OAuthConfig) -> Option<AuthorizationMetadata> {
    let (Some(auth), Some(token)) = (&oauth.authorization_url, &oauth.token_url) else { return None };
    let mut m = AuthorizationMetadata::default();
    m.authorization_endpoint = auth.clone();
    m.token_endpoint = token.clone();
    Some(m)
}

/// Add the config's extra parameters to the authorization URL.
fn with_params(auth_url: &str, oauth: &OAuthConfig) -> Result<String, ConnectorError> {
    if oauth.auth_params.is_empty() {
        return Ok(auth_url.to_string());
    }
    let mut url = url::Url::parse(auth_url).map_err(|e| ConnectorError::OAuth(e.to_string()))?;
    url.query_pairs_mut().extend_pairs(oauth.auth_params.iter());
    Ok(url.to_string())
}

fn query_param(url: &str, name: &str) -> Option<String> {
    url::Url::parse(url).ok()?.query_pairs().find(|(k, _)| k == name).map(|(_, v)| v.into_owned())
}

fn oauth_error(e: AuthError) -> ConnectorError {
    ConnectorError::OAuth(e.to_string())
}

/// A failed registration without a client id means the service wants one of
/// its own: say so rather than pass on an HTTP error.
fn session_error(e: AuthError, name: &str, has_client_id: bool) -> ConnectorError {
    match e {
        AuthError::RegistrationFailed(why) if !has_client_id => ConnectorError::ClientIdRequired(format!(
            "{name} doesn't let apps register themselves ({why}). Create an OAuth client in {name}'s developer console, \
             with redirect URI http://127.0.0.1/callback, and set its client id on the connector"
        )),
        AuthError::NoAuthorizationSupport => ConnectorError::OAuth(format!("{name} publishes no OAuth metadata: set authorization_url and token_url")),
        e => oauth_error(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_good_callback_gives_the_code() {
        assert_eq!(
            parse_callback("/callback?code=abc%2F1&state=s1&iss=https%3A%2F%2Fas.example", "s1"),
            Callback::Code { code: "abc/1".into(), issuer: Some("https://as.example".into()) }
        );
        assert_eq!(parse_callback("/callback?state=s1&code=x", "s1"), Callback::Code { code: "x".into(), issuer: None });
    }

    #[test]
    fn a_wrong_state_is_refused() {
        assert_eq!(parse_callback("/callback?code=abc&state=other", "s1"), Callback::Failed(ConnectorError::LoginStateMismatch));
        assert_eq!(parse_callback("/callback?code=abc", "s1"), Callback::Failed(ConnectorError::LoginStateMismatch));
        // Even an error is not trusted without our state.
        assert_eq!(parse_callback("/callback?error=access_denied", "s1"), Callback::Failed(ConnectorError::LoginStateMismatch));
    }

    #[test]
    fn a_refusal_says_so() {
        assert_eq!(
            parse_callback("/callback?error=access_denied&error_description=User+said+no&state=s1", "s1"),
            Callback::Failed(ConnectorError::LoginDenied("User said no".into()))
        );
        assert_eq!(parse_callback("/callback?error=server_error&state=s1", "s1"), Callback::Failed(ConnectorError::OAuth("server_error".into())));
        assert!(matches!(parse_callback("/callback?state=s1", "s1"), Callback::Failed(ConnectorError::OAuth(_))));
    }

    #[test]
    fn other_paths_are_ignored() {
        assert_eq!(parse_callback("/favicon.ico", "s1"), Callback::Other);
        assert_eq!(parse_callback("/", "s1"), Callback::Other);
    }

    #[test]
    fn extra_params_are_added() {
        let mut oauth = OAuthConfig::default();
        assert_eq!(with_params("https://a.example/auth?state=x", &oauth).unwrap(), "https://a.example/auth?state=x");
        oauth.auth_params.insert("access_type".into(), "offline".into());
        let url = with_params("https://a.example/auth?state=x", &oauth).unwrap();
        assert_eq!(query_param(&url, "access_type").as_deref(), Some("offline"));
        assert_eq!(query_param(&url, "state").as_deref(), Some("x"));
    }

    #[test]
    fn explicit_endpoints_need_both() {
        let mut oauth = OAuthConfig { authorization_url: Some("https://a.example/auth".into()), ..Default::default() };
        assert!(explicit_metadata(&oauth).is_none());
        oauth.token_url = Some("https://a.example/token".into());
        let m = explicit_metadata(&oauth).unwrap();
        assert_eq!(m.token_endpoint, "https://a.example/token");
    }

    #[test]
    fn no_registration_asks_for_a_client_id() {
        let e = session_error(AuthError::RegistrationFailed("Dynamic client registration not supported".into()), "GitHub", false);
        assert!(matches!(e, ConnectorError::ClientIdRequired(ref m) if m.contains("client id")), "{e}");
        let e = session_error(AuthError::RegistrationFailed("x".into()), "GitHub", true);
        assert!(matches!(e, ConnectorError::OAuth(_)));
    }

    #[tokio::test]
    async fn the_browser_gets_a_page_and_the_code() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let browser = tokio::spawn(async move {
            let get = |path: &'static str| async move {
                let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
                s.write_all(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes()).await.unwrap();
                let mut out = String::new();
                s.read_to_string(&mut out).await.unwrap();
                out
            };
            assert!(get("/favicon.ico").await.starts_with("HTTP/1.1 404"));
            get("/callback?code=c0de&state=s1").await
        });
        let (mut stream, code, _) = wait_for_code(&listener, "s1").await.unwrap();
        assert_eq!(code, "c0de");
        answer(&mut stream, None).await.unwrap();
        drop(stream);
        assert!(browser.await.unwrap().contains("Yoru is connected, you can close this tab."));
    }
}
