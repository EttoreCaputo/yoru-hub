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

use serde_json::{Value, json};

use super::{ANTHROPIC, CloudError};
use crate::stream::{self, Delta, OnDelta};

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
    let provider = ANTHROPIC.name;
    let mut response = post(key, body)?;
    let bytes = response.body_mut().with_config().limit(64 * 1024 * 1024).read_to_vec().map_err(|e| CloudError::Network { provider, why: e.to_string() })?;
    serde_json::from_slice(&bytes).map_err(|e| CloudError::Invalid { provider, why: e.to_string() })
}

/// [`messages`], streamed: the text goes to `on` as Claude writes it, and
/// [`Delta::Tools`] when it starts a tool call (a web search, too). The
/// answer comes back as a request that wasn't streamed would have it, with
/// what a server-side fallback left behind it taken out (`after_fallback`).
/// Only the request is retried: once words went out, an error is returned.
pub fn messages_stream(key: &str, body: &Value, on: OnDelta) -> Result<Value, CloudError> {
    let provider = ANTHROPIC.name;
    let mut body = body.clone();
    body["stream"] = true.into();
    let response = post(key, &body)?;
    let mut message = json!({});
    let mut blocks: Vec<Value> = Vec::new();
    // Tool inputs come as pieces of JSON text, a block's by its index.
    let mut inputs: Vec<String> = Vec::new();
    stream::events(response.into_body().into_reader(), |data| {
        let event: Value = serde_json::from_str(data).map_err(|e| format!("bad event: {e}"))?;
        let index = event["index"].as_u64().unwrap_or(0) as usize;
        match event["type"].as_str().unwrap_or_default() {
            "message_start" => message = event["message"].clone(),
            "content_block_start" => {
                let block = event["content_block"].clone();
                if matches!(block["type"].as_str(), Some("tool_use" | "server_tool_use")) {
                    on(Delta::Tools);
                }
                if blocks.len() <= index {
                    blocks.resize(index + 1, Value::Null);
                    inputs.resize(index + 1, String::new());
                }
                blocks[index] = block;
            }
            "content_block_delta" => {
                let (Some(block), delta) = (blocks.get_mut(index), &event["delta"]) else { return Ok(()) };
                match delta["type"].as_str().unwrap_or_default() {
                    "text_delta" => {
                        let text = delta["text"].as_str().unwrap_or_default();
                        append(block, "text", text);
                        on(Delta::Text(text));
                    }
                    "thinking_delta" => append(block, "thinking", delta["thinking"].as_str().unwrap_or_default()),
                    "signature_delta" => block["signature"] = delta["signature"].clone(),
                    "input_json_delta" => inputs[index].push_str(delta["partial_json"].as_str().unwrap_or_default()),
                    "citations_delta" => match block["citations"].as_array_mut() {
                        Some(list) => list.push(delta["citation"].clone()),
                        None => block["citations"] = json!([delta["citation"]]),
                    },
                    _ => {}
                }
            }
            "content_block_stop" => {
                if let (Some(block), Some(input)) = (blocks.get_mut(index), inputs.get(index).filter(|i| !i.trim().is_empty())) {
                    block["input"] = serde_json::from_str(input).unwrap_or_else(|_| json!({}));
                }
            }
            "message_delta" => {
                for (k, v) in event["delta"].as_object().into_iter().flatten() {
                    message[k] = v.clone();
                }
            }
            "error" => return Err(event.pointer("/error/message").and_then(Value::as_str).unwrap_or("the stream broke off").to_string()),
            _ => {}
        }
        Ok(())
    })
    .map_err(|why| CloudError::Network { provider, why })?;
    message["content"] = after_fallback(blocks.into_iter().filter(|b| !b.is_null()).collect()).into();
    Ok(message)
}

fn append(block: &mut Value, field: &str, more: &str) {
    let whole = format!("{}{more}", block[field].as_str().unwrap_or_default());
    block[field] = whole.into();
}

/// A streamed answer that fell back to another model mid-way keeps what
/// the first wrote before the `fallback` block. Its text stays; its
/// thinking, its tool calls (never to be run) and a web search without its
/// result don't go back in the conversation, as the API asks.
fn after_fallback(content: Vec<Value>) -> Vec<Value> {
    let Some(boundary) = content.iter().rposition(|b| b["type"] == "fallback") else { return content };
    let answered: Vec<Value> = content[..boundary].iter().filter_map(|b| b.get("tool_use_id").cloned()).collect();
    content
        .into_iter()
        .enumerate()
        .filter(|(i, b)| {
            *i >= boundary
                || b["type"] == "text"
                || b.get("tool_use_id").is_some()
                || (b["type"] == "server_tool_use" && answered.contains(&b["id"]))
        })
        .map(|(_, b)| b)
        .collect()
}

/// The request sent, until the API answers 200: rate limits (429), overload
/// (529) and server errors (5xx) are tried again a couple of times. Opus-tier
/// models get the server-side fallbacks.
fn post(key: &str, body: &Value) -> Result<ureq::http::Response<ureq::Body>, CloudError> {
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
        match open(request.send(&payload[..])) {
            Err(e) if attempt < RETRIES && retryable(&e) => {
                let wait = match &e {
                    CloudError::RateLimited { retry_after: Some(after), .. } => *after,
                    _ => Duration::from_secs(2u64.pow(attempt + 1)),
                };
                thread::sleep(wait.min(Duration::from_secs(30)));
                attempt += 1;
            }
            reply => return reply,
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
    open(sent)?.body_mut().with_config().limit(64 * 1024 * 1024).read_to_vec().map_err(|e| CloudError::Network { provider, why: e.to_string() })
}

/// A 200 answer, its body still to read, or the error it stands for.
fn open(sent: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<ureq::http::Response<ureq::Body>, CloudError> {
    let provider = ANTHROPIC.name;
    let mut response = sent.map_err(|e| CloudError::Network { provider, why: e.to_string() })?;
    let status = response.status().as_u16();
    if (200..300).contains(&status) {
        return Ok(response);
    }
    let retry_after = response.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<f64>().ok()).map(Duration::from_secs_f64);
    let body = response.body_mut().with_config().limit(1024 * 1024).read_to_vec().map_err(|e| CloudError::Network { provider, why: e.to_string() })?;
    match status {
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

    #[test]
    fn a_fallback_drops_what_the_first_model_left_unfinished() {
        let content = vec![
            json!({"type": "thinking", "thinking": "", "signature": "s"}),
            json!({"type": "text", "text": "Cerco."}),
            json!({"type": "server_tool_use", "id": "a", "name": "web_search"}),
            json!({"type": "web_search_tool_result", "tool_use_id": "a", "content": []}),
            json!({"type": "server_tool_use", "id": "b", "name": "web_search"}),
            json!({"type": "tool_use", "id": "c", "name": "notify", "input": {}}),
            json!({"type": "fallback"}),
            json!({"type": "thinking", "thinking": "", "signature": "t"}),
            json!({"type": "text", "text": "Ecco."}),
        ];
        let kept: Vec<String> = after_fallback(content).iter().map(|b| b["id"].as_str().or(b["text"].as_str()).unwrap_or(b["type"].as_str().unwrap()).to_string()).collect();
        assert_eq!(kept, ["Cerco.", "a", "web_search_tool_result", "fallback", "thinking", "Ecco."]);
    }
}
