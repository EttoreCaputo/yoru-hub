//! Signing in to Google with the user's own OAuth client, and keeping the
//! access token fresh.
//!
//! The client is a "Desktop app" client in the user's own Google Cloud
//! project: Google accepts any loopback port for it (RFC 8252 §7.3), so
//! nothing has to be registered. A "Web application" client still works
//! when the connector fixes its port. The flow is the plain authorization
//! code with PKCE; the secret of an installed app is not a secret, Google
//! just wants it on the token requests.
//!
//! rmcp's OAuth isn't used here: it adds an RFC 8707 `resource` to every
//! request, which Google's REST APIs don't know, and it wants discovery.
//! The tokens are kept as [`Tokens`] in the connector's credentials account.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::sync::Notify;

use super::super::login::{self, CALLBACK_PATH};
use super::super::store::{self, SecretStore};
use super::super::ConnectorError;

pub(crate) const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub(crate) const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// An access token this close to its end is refreshed first.
const MARGIN_SECS: u64 = 60;
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);

/// What the keychain keeps for a signed-in Google connector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Tokens {
    /// The client that got them: another client can't refresh them.
    pub client_id: String,
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds.
    pub expires_at: u64,
    #[serde(default)]
    pub scope: String,
}

impl Tokens {
    fn fresh(&self) -> bool {
        now() + MARGIN_SECS < self.expires_at
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Read the stored tokens; anything else in the account (rmcp's credentials
/// from before) counts as none.
pub(crate) fn load(secrets: &dyn SecretStore, id: &str) -> Result<Option<Tokens>, ConnectorError> {
    Ok(secrets.get(&store::credentials_account(id))?.and_then(|json| serde_json::from_str(&json).ok()))
}

fn save(secrets: &dyn SecretStore, id: &str, tokens: &Tokens) -> Result<(), ConnectorError> {
    let json = serde_json::to_string(tokens).map_err(|e| ConnectorError::Keychain(e.to_string()))?;
    secrets.set(&store::credentials_account(id), &json)
}

/// Why there is no access token.
#[derive(Debug)]
pub(crate) enum TokenError {
    /// Google refused the refresh token (revoked, expired after 7 days of a
    /// project in testing, another client): sign in again.
    SignInAgain(String),
    /// Anything else: the network, the keychain; worth another try.
    Other(String),
}

/// The client a connector signs in with.
#[derive(Clone)]
pub(crate) struct Client {
    pub id: String,
    pub secret: Option<String>,
}

/// One connector's tokens, refreshed when they run out. Cloned into every
/// call; one refresh at a time.
#[derive(Clone)]
pub(crate) struct TokenSource {
    id: String,
    client: Client,
    secrets: Arc<dyn SecretStore>,
    http: reqwest::Client,
    cached: Arc<tokio::sync::Mutex<Option<Tokens>>>,
}

impl TokenSource {
    pub fn new(id: &str, client: Client, secrets: Arc<dyn SecretStore>, http: reqwest::Client) -> Self {
        Self { id: id.into(), client, secrets, http, cached: Arc::default() }
    }

    /// A valid access token, refreshing it when it's about to end.
    pub async fn access_token(&self) -> Result<String, TokenError> {
        let mut cached = self.cached.lock().await;
        if cached.is_none() {
            let (secrets, id) = (self.secrets.clone(), self.id.clone());
            *cached = tokio::task::spawn_blocking(move || load(secrets.as_ref(), &id))
                .await
                .map_err(|e| TokenError::Other(e.to_string()))?
                .map_err(|e| TokenError::Other(e.to_string()))?;
        }
        let Some(tokens) = cached.clone() else { return Err(TokenError::SignInAgain("not signed in".into())) };
        if tokens.fresh() {
            return Ok(tokens.access_token);
        }
        if tokens.client_id != self.client.id {
            return Err(TokenError::SignInAgain("the tokens belong to another OAuth client".into()));
        }
        let refreshed = self.refresh(&tokens).await?;
        let (secrets, id, keep) = (self.secrets.clone(), self.id.clone(), refreshed.clone());
        tokio::task::spawn_blocking(move || save(secrets.as_ref(), &id, &keep))
            .await
            .map_err(|e| TokenError::Other(e.to_string()))?
            .map_err(|e| TokenError::Other(e.to_string()))?;
        let token = refreshed.access_token.clone();
        *cached = Some(refreshed);
        Ok(token)
    }

    /// Forget the cached token, after Google said it's no good.
    pub async fn invalidate(&self) {
        if let Some(t) = self.cached.lock().await.as_mut() {
            t.expires_at = 0;
        }
    }

    async fn refresh(&self, tokens: &Tokens) -> Result<Tokens, TokenError> {
        let mut form = vec![
            ("grant_type", "refresh_token"),
            ("client_id", self.client.id.as_str()),
            ("refresh_token", tokens.refresh_token.as_str()),
        ];
        if let Some(secret) = &self.client.secret {
            form.push(("client_secret", secret));
        }
        match token_request(&self.http, &form).await {
            // Google sends no new refresh token on a refresh: keep ours.
            Ok(answer) => Ok(answer.into_tokens(&self.client.id, Some(&tokens.refresh_token)).map_err(TokenError::Other)?),
            Err(TokenFailure::Refused(why)) => Err(TokenError::SignInAgain(why)),
            Err(TokenFailure::Other(why)) => Err(TokenError::Other(why)),
        }
    }
}

/// The token endpoint's answer.
#[derive(Deserialize)]
struct TokenAnswer {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    scope: Option<String>,
}

impl TokenAnswer {
    fn into_tokens(self, client_id: &str, old_refresh: Option<&str>) -> Result<Tokens, String> {
        let refresh_token = self
            .refresh_token
            .or_else(|| old_refresh.map(str::to_string))
            .ok_or("Google gave no refresh token; remove the connector and connect it again")?;
        Ok(Tokens {
            client_id: client_id.into(),
            access_token: self.access_token,
            refresh_token,
            expires_at: now() + self.expires_in.unwrap_or(3600),
            scope: self.scope.unwrap_or_default(),
        })
    }
}

enum TokenFailure {
    /// `invalid_grant` and friends: the grant is gone.
    Refused(String),
    Other(String),
}

async fn token_request(http: &reqwest::Client, form: &[(&str, &str)]) -> Result<TokenAnswer, TokenFailure> {
    let body = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(form).finish();
    let response = http
        .post(TOKEN_URL)
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body)
        .timeout(HTTP_TIMEOUT)
        .send()
        .await
        .map_err(|e| TokenFailure::Other(format!("Google's token endpoint: {e}")))?;
    let status = response.status();
    let bytes = response.bytes().await.map_err(|e| TokenFailure::Other(e.to_string()))?;
    if status.is_success() {
        return serde_json::from_slice(&bytes).map_err(|e| TokenFailure::Other(format!("Google's token answer: {e}")));
    }
    let error: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
    let code = error["error"].as_str().unwrap_or_default();
    let why = error["error_description"].as_str().map(str::to_string).unwrap_or_else(|| format!("{status} {code}"));
    if matches!(code, "invalid_grant" | "unauthorized_client" | "invalid_client") {
        Err(TokenFailure::Refused(why))
    } else {
        Err(TokenFailure::Other(why))
    }
}

/// A sign-in waiting for the browser.
pub(crate) struct Prepared {
    pub auth_url: String,
    listener: TcpListener,
    state: String,
    verifier: String,
    redirect_uri: String,
}

/// Bind the loopback port and build Google's consent URL.
pub(crate) async fn prepare(client: &Client, scopes: &[&str], port: Option<u16>) -> Result<Prepared, ConnectorError> {
    let listener = TcpListener::bind(("127.0.0.1", port.unwrap_or(0))).await?;
    let redirect_uri = format!("http://127.0.0.1:{}{CALLBACK_PATH}", listener.local_addr()?.port());
    let verifier = random_token()?;
    let state = random_token()?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url = url::Url::parse(AUTH_URL).expect("a valid URL");
    url.query_pairs_mut()
        .append_pair("client_id", &client.id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", &scopes.join(" "))
        .append_pair("state", &state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        // A refresh token comes only with offline access, and again only
        // when the consent screen is shown.
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent");
    Ok(Prepared { auth_url: url.into(), listener, state, verifier, redirect_uri })
}

/// Wait for the browser, exchange the code and keep the tokens.
pub(crate) async fn finish(
    p: Prepared,
    id: &str,
    client: &Client,
    secrets: Arc<dyn SecretStore>,
    cancel: Arc<Notify>,
    timeout: Duration,
) -> Result<(), ConnectorError> {
    let waited = tokio::select! {
        r = tokio::time::timeout(timeout, login::wait_for_code(&p.listener, &p.state)) => r.unwrap_or(Err(ConnectorError::LoginTimeout)),
        _ = cancel.notified() => Err(ConnectorError::LoginCancelled),
    };
    let (mut browser, code, _) = waited?;
    let result = exchange(&p, &code, id, client, secrets).await;
    let _ = login::answer(&mut browser, result.as_ref().err()).await;
    result
}

async fn exchange(p: &Prepared, code: &str, id: &str, client: &Client, secrets: Arc<dyn SecretStore>) -> Result<(), ConnectorError> {
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", p.redirect_uri.as_str()),
        ("client_id", client.id.as_str()),
        ("code_verifier", p.verifier.as_str()),
    ];
    if let Some(secret) = &client.secret {
        form.push(("client_secret", secret));
    }
    let answer = token_request(&reqwest::Client::new(), &form).await.map_err(|e| match e {
        TokenFailure::Refused(why) | TokenFailure::Other(why) => ConnectorError::OAuth(why),
    })?;
    let tokens = answer.into_tokens(&client.id, None).map_err(ConnectorError::OAuth)?;
    let id = id.to_string();
    tokio::task::spawn_blocking(move || save(secrets.as_ref(), &id, &tokens))
        .await
        .map_err(|e| ConnectorError::Keychain(e.to_string()))?
}

/// 32 random bytes, base64url: a PKCE verifier or a state.
fn random_token() -> Result<String, ConnectorError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| ConnectorError::OAuth(e.to_string()))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::MemorySecrets;

    fn client() -> Client {
        Client { id: "cid.apps.googleusercontent.com".into(), secret: Some("s".into()) }
    }

    #[tokio::test]
    async fn the_consent_url_asks_for_offline_access_with_pkce() {
        let p = prepare(&client(), &["https://www.googleapis.com/auth/gmail.modify"], None).await.unwrap();
        let url = url::Url::parse(&p.auth_url).unwrap();
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q["access_type"], "offline");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["code_challenge"], URL_SAFE_NO_PAD.encode(Sha256::digest(p.verifier.as_bytes())));
        assert_eq!(q["state"], p.state);
        assert!(q["redirect_uri"].starts_with("http://127.0.0.1:") && q["redirect_uri"].ends_with("/callback"));
        assert!(!q.contains_key("resource"));
    }

    #[test]
    fn a_refresh_keeps_the_old_refresh_token() {
        let answer = TokenAnswer { access_token: "a2".into(), refresh_token: None, expires_in: Some(3599), scope: None };
        let t = answer.into_tokens("cid", Some("r1")).unwrap();
        assert_eq!((t.access_token.as_str(), t.refresh_token.as_str()), ("a2", "r1"));
        assert!(t.fresh());
        let answer = TokenAnswer { access_token: "a".into(), refresh_token: None, expires_in: None, scope: None };
        assert!(answer.into_tokens("cid", None).is_err());
    }

    #[tokio::test]
    async fn a_fresh_token_is_used_as_stored() {
        let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecrets::default());
        let tokens = Tokens { client_id: client().id, access_token: "a1".into(), refresh_token: "r1".into(), expires_at: now() + 3600, scope: String::new() };
        save(secrets.as_ref(), "gmail", &tokens).unwrap();
        let source = TokenSource::new("gmail", client(), secrets.clone(), reqwest::Client::new());
        assert_eq!(source.access_token().await.unwrap(), "a1");
        // Another client's tokens can't be refreshed.
        let other = TokenSource::new("gmail", Client { id: "other".into(), secret: None }, secrets, reqwest::Client::new());
        other.invalidate().await;
        let mut stale = tokens;
        stale.expires_at = 0;
        *other.cached.lock().await = Some(stale);
        assert!(matches!(other.access_token().await, Err(TokenError::SignInAgain(_))));
    }

    #[test]
    fn rmcp_credentials_from_before_are_no_tokens() {
        let secrets = MemorySecrets::default();
        secrets.set("gmail", r#"{"client_id":"x","token_response":null}"#).unwrap();
        assert_eq!(load(&secrets, "gmail").unwrap(), None);
    }
}
