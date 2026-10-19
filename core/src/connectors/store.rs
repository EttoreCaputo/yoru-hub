//! Secrets in the system keychain: the OAuth tokens of each connector, its
//! client secret, and the named secrets local servers read from their
//! environment (`{"secret": "GITHUB_TOKEN"}`).
//!
//! Everything is under one keychain service, [`SERVICE`], one entry per
//! account:
//!
//! | account               | what                                             |
//! |-----------------------|--------------------------------------------------|
//! | `<id>`                | rmcp's `StoredCredentials` (client id, tokens) as JSON |
//! | `<id>.oauth-state`    | the PKCE state of sign-ins in progress, as JSON  |
//! | `<id>.client-secret`  | our own OAuth client's secret                    |
//! | `secret.<NAME>`       | a named secret for local servers                 |
//!
//! Ids have no `.` ([`super::config::is_valid_id`]), so these never collide.
//! [`SecretStore`] hides the keychain so tests (and a caller without one)
//! can keep secrets in memory.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rmcp::transport::auth::{AuthError, CredentialStore, StateStore, StoredAuthorizationState, StoredCredentials};

use super::ConnectorError;

/// The keychain service every entry is under.
pub const SERVICE: &str = "it.bytebusters.yoru.connectors";

/// A sign-in left half done is forgotten after this long.
const STATE_TTL_SECS: u64 = 15 * 60;

/// Where secrets are kept. Calls block (the keychain may ask the user), so
/// async code reaches it through `spawn_blocking`.
pub trait SecretStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<String>, ConnectorError>;
    fn set(&self, account: &str, value: &str) -> Result<(), ConnectorError>;
    /// Deleting what isn't there is not an error.
    fn delete(&self, account: &str) -> Result<(), ConnectorError>;
}

/// The system keychain: Keychain Services on macOS, the Credential Manager
/// on Windows, the Secret Service on Linux.
#[derive(Debug, Default, Clone, Copy)]
pub struct Keychain;

impl Keychain {
    fn entry(account: &str) -> Result<keyring::Entry, ConnectorError> {
        keyring::Entry::new(SERVICE, account).map_err(keychain_error)
    }
}

impl SecretStore for Keychain {
    fn get(&self, account: &str) -> Result<Option<String>, ConnectorError> {
        match Self::entry(account)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keychain_error(e)),
        }
    }

    fn set(&self, account: &str, value: &str) -> Result<(), ConnectorError> {
        Self::entry(account)?.set_password(value).map_err(keychain_error)
    }

    fn delete(&self, account: &str) -> Result<(), ConnectorError> {
        match Self::entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keychain_error(e)),
        }
    }
}

fn keychain_error(e: keyring::Error) -> ConnectorError {
    ConnectorError::Keychain(e.to_string())
}

/// Secrets kept in memory only, for tests.
#[derive(Debug, Default)]
pub struct MemorySecrets(Mutex<HashMap<String, String>>);

impl SecretStore for MemorySecrets {
    fn get(&self, account: &str) -> Result<Option<String>, ConnectorError> {
        Ok(self.0.lock().unwrap().get(account).cloned())
    }

    fn set(&self, account: &str, value: &str) -> Result<(), ConnectorError> {
        self.0.lock().unwrap().insert(account.into(), value.into());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), ConnectorError> {
        self.0.lock().unwrap().remove(account);
        Ok(())
    }
}

pub(crate) fn credentials_account(id: &str) -> String {
    id.to_string()
}

pub(crate) fn state_account(id: &str) -> String {
    format!("{id}.oauth-state")
}

pub(crate) fn client_secret_account(id: &str) -> String {
    format!("{id}.client-secret")
}

/// A named secret's account; `None` for a name that isn't an environment
/// variable's (`[A-Za-z0-9_]+`).
pub(crate) fn named_account(name: &str) -> Option<String> {
    let ok = !name.is_empty() && name.len() <= 128 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    ok.then(|| format!("secret.{name}"))
}

/// Every entry that belongs to connector `id` (not the named secrets, which
/// may be shared).
pub(crate) fn connector_accounts(id: &str) -> [String; 3] {
    [credentials_account(id), state_account(id), client_secret_account(id)]
}

/// Run a blocking store call off the async workers.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ConnectorError> + Send + 'static) -> Result<T, AuthError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| AuthError::CredentialStoreError(e.to_string()))?
        .map_err(|e| AuthError::CredentialStoreError(e.to_string()))
}

/// rmcp's credential store for one connector, on a [`SecretStore`].
#[derive(Clone)]
pub(crate) struct ConnectorCredentials {
    pub account: String,
    pub secrets: Arc<dyn SecretStore>,
}

impl ConnectorCredentials {
    pub fn new(id: &str, secrets: Arc<dyn SecretStore>) -> Self {
        Self { account: credentials_account(id), secrets }
    }

    /// Whether a token was ever stored: a 401 then means it expired or was
    /// revoked, not that the user never signed in.
    pub fn has_token(&self) -> bool {
        matches!(self.secrets.get(&self.account), Ok(Some(json)) if serde_json::from_str::<StoredCredentials>(&json).is_ok_and(|c| c.token_response.is_some()))
    }
}

#[async_trait::async_trait]
impl CredentialStore for ConnectorCredentials {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        let this = self.clone();
        let json = blocking(move || this.secrets.get(&this.account)).await?;
        // An entry that no longer parses is as good as none: sign in again.
        Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let json = serde_json::to_string(&credentials).map_err(|e| AuthError::CredentialStoreError(e.to_string()))?;
        let this = self.clone();
        blocking(move || this.secrets.set(&this.account, &json)).await
    }

    async fn clear(&self) -> Result<(), AuthError> {
        let this = self.clone();
        blocking(move || this.secrets.delete(&this.account)).await
    }
}

/// rmcp's PKCE state store for one connector: one entry holding every
/// sign-in in progress, keyed by CSRF token, since the keychain can't list
/// entries to clean up abandoned ones.
#[derive(Clone)]
pub(crate) struct ConnectorStates {
    pub account: String,
    pub secrets: Arc<dyn SecretStore>,
}

type StateMap = HashMap<String, StoredAuthorizationState>;

impl ConnectorStates {
    pub fn new(id: &str, secrets: Arc<dyn SecretStore>) -> Self {
        Self { account: state_account(id), secrets }
    }

    fn read(&self) -> Result<StateMap, ConnectorError> {
        let json = self.secrets.get(&self.account)?;
        Ok(json.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default())
    }

    fn write(&self, map: &StateMap) -> Result<(), ConnectorError> {
        if map.is_empty() {
            return self.secrets.delete(&self.account);
        }
        let json = serde_json::to_string(map).map_err(|e| ConnectorError::Keychain(e.to_string()))?;
        self.secrets.set(&self.account, &json)
    }

    /// Apply `f` to the map, dropping stale sign-ins on the way.
    async fn update<T: Send + 'static>(&self, f: impl FnOnce(&mut StateMap) -> T + Send + 'static) -> Result<T, AuthError> {
        let this = self.clone();
        blocking(move || {
            let mut map = this.read()?;
            let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            map.retain(|_, s| now.saturating_sub(s.created_at) < STATE_TTL_SECS);
            let out = f(&mut map);
            this.write(&map)?;
            Ok(out)
        })
        .await
    }
}

#[async_trait::async_trait]
impl StateStore for ConnectorStates {
    async fn save(&self, csrf_token: &str, state: StoredAuthorizationState) -> Result<(), AuthError> {
        let key = csrf_token.to_string();
        self.update(move |map| {
            map.insert(key, state);
        })
        .await
    }

    async fn load(&self, csrf_token: &str) -> Result<Option<StoredAuthorizationState>, AuthError> {
        let key = csrf_token.to_string();
        self.update(move |map| map.get(&key).cloned()).await
    }

    async fn delete(&self, csrf_token: &str) -> Result<(), AuthError> {
        let key = csrf_token.to_string();
        self.update(move |map| {
            map.remove(&key);
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounts_never_collide() {
        let id = "gmail";
        let all = connector_accounts(id);
        assert_eq!(all, ["gmail".to_string(), "gmail.oauth-state".into(), "gmail.client-secret".into()]);
        assert_eq!(named_account("GITHUB_TOKEN").as_deref(), Some("secret.GITHUB_TOKEN"));
        assert_eq!(named_account("bad name"), None);
        assert_eq!(named_account(""), None);
    }

    #[tokio::test]
    async fn credentials_round_trip() {
        let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecrets::default());
        let store = ConnectorCredentials::new("x", secrets.clone());
        assert!(store.load().await.unwrap().is_none());
        assert!(!store.has_token());
        store.save(StoredCredentials::new("client".into(), None, vec!["a".into()], Some(1))).await.unwrap();
        let back = store.load().await.unwrap().unwrap();
        assert_eq!(back.client_id, "client");
        assert_eq!(back.granted_scopes, vec!["a".to_string()]);
        // No token yet: a client id alone (after registration) is not a sign-in.
        assert!(!store.has_token());
        store.clear().await.unwrap();
        assert!(secrets.get("x").unwrap().is_none());
    }

    #[tokio::test]
    async fn states_expire_and_are_removed() {
        let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecrets::default());
        let store = ConnectorStates::new("x", secrets.clone());
        let fresh: StoredAuthorizationState = serde_json::from_value(serde_json::json!({
            "pkce_verifier": "v", "csrf_token": "c1",
            "created_at": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
        }))
        .unwrap();
        let mut stale = fresh.clone();
        stale.created_at = 1;
        store.save("c1", fresh).await.unwrap();
        store.save("c0", stale).await.unwrap();
        assert!(store.load("c0").await.unwrap().is_none());
        assert_eq!(store.load("c1").await.unwrap().unwrap().pkce_verifier, "v");
        store.delete("c1").await.unwrap();
        assert!(store.load("c1").await.unwrap().is_none());
        // The last one gone, the entry goes too.
        assert!(secrets.get("x.oauth-state").unwrap().is_none());
    }
}
