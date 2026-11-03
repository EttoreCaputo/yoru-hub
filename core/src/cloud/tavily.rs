//! [`Api::Tavily`](super::Api::Tavily): [Tavily](https://tavily.com)'s web
//! search, which the board calls itself when the computer isn't there
//! (`search` in `cloud/set`, see [`crate::bridge`]'s cloud module). It has
//! no models: only a key, which Yoru Hub hands to the board.

use std::sync::OnceLock;
use std::time::Duration;

use super::{CloudError, TAVILY};

const TIMEOUT: Duration = Duration::from_secs(15);

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| ureq::Agent::new_with_config(ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(TIMEOUT)).build()))
}

/// Whether the key works, asking for the account's usage: it costs no credit.
pub fn check_key(key: &str) -> Result<(), CloudError> {
    let provider = TAVILY.name;
    let response = agent().get(format!("{}/usage", TAVILY.url())).header("Authorization", format!("Bearer {key}")).call().map_err(|e| CloudError::Network { provider, why: e.to_string() })?;
    match response.status().as_u16() {
        200..=299 => Ok(()),
        401 | 403 => Err(CloudError::BadKey { provider }),
        429 => Err(CloudError::RateLimited { provider, retry_after: None, per_day: false }),
        status => Err(CloudError::Status { provider, status, message: String::new() }),
    }
}
