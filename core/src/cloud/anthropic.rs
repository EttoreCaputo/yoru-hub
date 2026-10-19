//! [`Api::Anthropic`](super::Api::Anthropic): Claude's Messages API, the
//! assistant's brain ([`crate::assistant`]). Raw HTTP: there is no official
//! Rust SDK.
//!
//! Requests are `POST /v1/messages` bodies as the API documents them; the
//! answer comes back as JSON. Opus-tier requests opt into server-side
//! fallbacks (`fallbacks: "default"`, beta `server-side-fallback-2026-07-01`),
//! so a request a safety classifier declines is answered by the model
//! Anthropic recommends for it instead of coming back refused. A refusal
//! that remains is still a normal answer: callers check `stop_reason`.

use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

use serde_json::Value;

use super::{ANTHROPIC, CloudError};

const BASE_URL: &str = "https://api.anthropic.com";

/// The API's address: `ANTHROPIC_BASE_URL` when set (a proxy, or a test
/// server), as the official SDKs do.
fn base_url() -> String {
    std::env::var("ANTHROPIC_BASE_URL").ok().map(|u| u.trim_end_matches('/').to_string()).filter(|u| !u.is_empty()).unwrap_or_else(|| BASE_URL.into())
}
const VERSION: &str = "2023-06-01";
/// The fallbacks' beta, for the `"default"` form.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
/// A turn with tools and thinking can take minutes on a hard request.
const TIMEOUT: Duration = Duration::from_secs(600);
/// Retries of an overloaded or rate-limited request, with a growing pause.
const RETRIES: u32 = 2;

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| ureq::Agent::new_with_config(ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(TIMEOUT)).build()))
}

/// The models that take server-side fallbacks. Haiku has none, and a list
/// sent to it is refused.
fn takes_fallbacks(model: &str) -> bool {
    model.starts_with("claude-opus-5") || model.starts_with("claude-fable-5-1") || model == "claude-sonnet-5-5"
}

/// The key, from `ANTHROPIC_API_KEY` or `keys.json`.
pub fn api_key() -> Option<String> {
    ANTHROPIC.api_key()
}

/// One Messages API request. Rate limits (429), overload (529) and server
/// errors (5xx) are retried a couple of times before they are returned.
pub fn messages(key: &str, body: &Value) -> Result<Value, CloudError> {
    let mut body = body.clone();
    let model = body["model"].as_str().unwrap_or_default().to_string();
    let mut betas: Vec<&str> = Vec::new();
    if takes_fallbacks(&model) && body.get("fallbacks").is_none() {
        body["fallbacks"] = Value::String("default".into());
        betas.push(FALLBACK_BETA);
    }
    let payload = serde_json::to_vec(&body).map_err(|e| CloudError::Invalid { provider: ANTHROPIC.name, why: e.to_string() })?;
    let mut attempt = 0;
    loop {
        let mut request = agent()
            .post(format!("{}/v1/messages", base_url()))
            .header("x-api-key", key)
            .header("anthropic-version", VERSION)
            .header("content-type", "application/json");
        if !betas.is_empty() {
            request = request.header("anthropic-beta", betas.join(","));
        }
        match read(request.send(&payload[..])) {
            Err(e) if attempt < RETRIES && retryable(&e) => {
                let wait = match &e {
                    CloudError::RateLimited { retry_after: Some(after), .. } => *after,
                    _ => Duration::from_secs(2u64.pow(attempt + 1)),
                };
                thread::sleep(wait.min(Duration::from_secs(30)));
                attempt += 1;
            }
            Err(e) => return Err(e),
            Ok(bytes) => return serde_json::from_slice(&bytes).map_err(|e| CloudError::Invalid { provider: ANTHROPIC.name, why: e.to_string() }),
        }
    }
}

/// Whether the key works: listing the models is free.
pub fn check_key(key: &str) -> Result<(), CloudError> {
    read(agent().get(format!("{}/v1/models", base_url())).header("x-api-key", key).header("anthropic-version", VERSION).call()).map(|_| ())
}

fn retryable(e: &CloudError) -> bool {
    match e {
        CloudError::RateLimited { .. } | CloudError::Network { .. } => true,
        CloudError::Status { status, .. } => *status >= 500,
        _ => false,
    }
}

fn read(sent: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Vec<u8>, CloudError> {
    let provider = ANTHROPIC.name;
    let mut response = sent.map_err(|e| CloudError::Network { provider, why: e.to_string() })?;
    let status = response.status().as_u16();
    let retry_after = response.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<f64>().ok()).map(Duration::from_secs_f64);
    let body = response.body_mut().with_config().limit(64 * 1024 * 1024).read_to_vec().map_err(|e| CloudError::Network { provider, why: e.to_string() })?;
    match status {
        200..=299 => Ok(body),
        401 | 403 => Err(CloudError::BadKey { provider }),
        429 => Err(CloudError::RateLimited { provider, retry_after, per_day: false }),
        _ => Err(CloudError::Status { provider, status, message: error_message(&body) }),
    }
}

/// `{"type": "error", "error": {"type", "message"}}`, or the start of whatever it is.
fn error_message(body: &[u8]) -> String {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| String::from_utf8_lossy(body).chars().take(300).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_some_models_get_fallbacks() {
        assert!(takes_fallbacks("claude-opus-5-5"));
        assert!(takes_fallbacks("claude-sonnet-5-5"));
        assert!(!takes_fallbacks("claude-haiku-5-5"));
    }

    #[test]
    fn errors_say_what_the_api_said() {
        let body = br#"{"type":"error","error":{"type":"invalid_request_error","message":"max_tokens: too big"}}"#;
        assert_eq!(error_message(body), "max_tokens: too big");
        assert!(retryable(&CloudError::Status { provider: "Anthropic", status: 529, message: String::new() }));
        assert!(!retryable(&CloudError::Status { provider: "Anthropic", status: 400, message: String::new() }));
    }
}
