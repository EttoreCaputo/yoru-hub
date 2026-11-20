//! [`Api::OpenAi`](super::Api::OpenAi): OpenAI's HTTP API, which OpenAI,
//! Groq, Ollama and most other services speak.

use std::sync::OnceLock;
use std::time::Duration;

use serde_json::{Map, Value};

use super::{Client, CloudError, TranscriptionReply};
use crate::stream::{self, OnDelta};

/// An assistant's turn with tools takes longer than a voice command's.
const TIMEOUT: Duration = Duration::from_secs(60);
/// A JSON answer is background work (a meeting's summary, facts to keep):
/// a long transcript keeps a thinking model busy well past a minute.
const JSON_TIMEOUT: Duration = Duration::from_secs(300);
/// What a chat request may carry: llama.cpp's own settings (`top_k`,
/// `chat_template_kwargs`) are refused by most services.
const CHAT_FIELDS: &[&str] = &["messages", "tools", "tool_choice", "temperature", "top_p", "max_tokens", "seed", "stop", "response_format"];

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    // Error statuses are read like any answer, for the provider's message.
    AGENT.get_or_init(|| ureq::Agent::new_with_config(ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(TIMEOUT)).build()))
}

/// [`agent`], for a JSON answer: [`JSON_TIMEOUT`].
fn json_agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| ureq::Agent::new_with_config(ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(JSON_TIMEOUT)).build()))
}

fn url(client: &Client, path: &str) -> String {
    format!("{}{path}", client.model.provider.url())
}

/// The request with the key, unless the service needs none (Ollama).
fn authorized<B>(client: &Client, request: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
    if client.key.is_empty() { request } else { request.header("Authorization", format!("Bearer {}", client.key)) }
}

/// The body of a successful answer, or the error it stands for.
pub(super) fn read(client: &Client, sent: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Vec<u8>, CloudError> {
    let provider = client.model.provider.name;
    let mut response = open(client, sent)?;
    response.body_mut().with_config().limit(32 * 1024 * 1024).read_to_vec().map_err(|e| CloudError::Network { provider, why: e.to_string() })
}

/// A successful answer, its body still to read (a stream), or the error it stands for.
pub(super) fn open(client: &Client, sent: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<ureq::http::Response<ureq::Body>, CloudError> {
    let provider = client.model.provider.name;
    let mut response = sent.map_err(|e| CloudError::Network { provider, why: e.to_string() })?;
    let status = response.status().as_u16();
    if (200..300).contains(&status) {
        return Ok(response);
    }
    let retry_after = response.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<f64>().ok()).map(Duration::from_secs_f64);
    let body = response.body_mut().with_config().limit(1024 * 1024).read_to_vec().map_err(|e| CloudError::Network { provider, why: e.to_string() })?;
    let message = error_message(&body);
    match status {
        401 | 403 => Err(CloudError::BadKey { provider }),
        // Google answers a wrong key with a 400.
        400 if message.contains("API key not valid") || message.contains("API_KEY_INVALID") => Err(CloudError::BadKey { provider }),
        429 => {
            let (wait, per_day) = google_limit(&body).unwrap_or_else(|| limit_of(&message));
            Err(CloudError::RateLimited { provider, retry_after: retry_after.or(wait), per_day })
        }
        _ => Err(CloudError::Status { provider, status, message }),
    }
}

/// The error of an answer: `{"error": …}`, or Google's `[{"error": …}]`.
fn error_of(body: &[u8]) -> Option<Value> {
    let v = serde_json::from_slice::<Value>(body).ok()?;
    let v = if v.is_array() { v.get(0)?.clone() } else { v };
    v.get("error").cloned()
}

/// `{"error": {"message": ...}}`, or the start of whatever it is.
fn error_message(body: &[u8]) -> String {
    error_of(body)
        .and_then(|e| e["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| String::from_utf8_lossy(body).chars().take(300).collect())
}

/// What Google's 429 says in its details: `RetryInfo`'s `retryDelay`
/// (`"34s"`), and whether a per-day quota ran out (`…PerDay…` in a
/// `QuotaFailure` violation's `quotaId`). `None` when it has no details.
fn google_limit(body: &[u8]) -> Option<(Option<Duration>, bool)> {
    let details = error_of(body)?.get("details")?.as_array()?.clone();
    let wait = details.iter().find_map(|d| d["retryDelay"].as_str()).and_then(parse_wait);
    let per_day = details
        .iter()
        .filter_map(|d| d["violations"].as_array())
        .flatten()
        .any(|v| v["quotaId"].as_str().is_some_and(|q| q.contains("PerDay")));
    Some((wait, per_day))
}

/// What a 429's message says: when to try again, and whether the day's
/// allowance ran out. Groq's: "Rate limit reached for model `…` on tokens
/// per day (TPD): Limit 200000, Used …. Please try again in 7m12.5s."
fn limit_of(message: &str) -> (Option<Duration>, bool) {
    let per_day = message.contains("per day");
    let wait = message.split_once("try again in ").and_then(|(_, rest)| parse_wait(rest.split_whitespace().next()?));
    (wait, per_day)
}

/// `1h2m7.5s`, `7m12.5s`, `450ms`, `3.2s.`
fn parse_wait(text: &str) -> Option<Duration> {
    let text = text.trim_end_matches(['.', ',']);
    let mut total = 0.0;
    let mut number = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() || c == '.' {
            number.push(c);
            continue;
        }
        let n: f64 = number.parse().ok()?;
        number.clear();
        total += match c {
            'h' => n * 3600.0,
            'm' if chars.peek() == Some(&'s') => {
                chars.next();
                n / 1000.0
            }
            'm' => n * 60.0,
            's' => n,
            _ => return None,
        };
    }
    (number.is_empty() && total > 0.0).then(|| Duration::from_secs_f64(total))
}

fn json(client: &Client, body: &[u8]) -> Result<Value, CloudError> {
    serde_json::from_slice(body).map_err(|e| CloudError::Invalid { provider: client.model.provider.name, why: e.to_string() })
}

pub(super) fn check(client: &Client) -> Result<(), CloudError> {
    read(client, authorized(client, agent().get(url(client, "/models"))).call()).map(|_| ())
}

/// The ids of the models the service has.
pub(super) fn list_models(client: &Client) -> Result<Vec<String>, CloudError> {
    let reply = json(client, &read(client, authorized(client, agent().get(url(client, "/models"))).call())?)?;
    let mut ids: Vec<String> = reply["data"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str().map(str::to_string)).collect();
    ids.sort();
    Ok(ids)
}

/// `request` cut down to [`CHAT_FIELDS`], with the model and its params.
fn chat_request(client: &Client, request: &Value) -> Value {
    let mut body: Map<String, Value> = request.as_object().into_iter().flatten().filter(|(k, _)| CHAT_FIELDS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
    // A request without tools mustn't choose among them.
    if !body.contains_key("tools") {
        body.remove("tool_choice");
    }
    body.insert("model".into(), client.model_name().into());
    let asked = body.get("max_tokens").and_then(Value::as_u64);
    if let Ok(Value::Object(params)) = serde_json::from_str::<Value>(client.model.model.params) {
        body.extend(params);
    }
    // The params' limit is a floor for a model that thinks before it answers;
    // a request that needs more (the assistant's) keeps its own.
    if let (Some(asked), Some(floor)) = (asked, body.get("max_tokens").and_then(Value::as_u64)) {
        body.insert("max_tokens".into(), asked.max(floor).into());
    }
    if client.model.provider.id == super::GOOGLE.id
        && let Some(Value::Array(messages)) = body.get_mut("messages")
    {
        sign_tool_calls(messages);
    }
    // OpenAI's own models refuse `max_tokens`: they count their reasoning too.
    if client.model.provider.id == "openai"
        && let Some(max) = body.remove("max_tokens")
    {
        body.insert("max_completion_tokens".into(), max);
    }
    Value::Object(body)
}

/// Google's Gemini 3 and 4 refuse a tool call sent back without the thought
/// signature they gave it (`extra_content.google.thought_signature`, kept
/// with the call). One that has none (the conversation began with another
/// brain) gets the stand-in Google allows for calls it didn't make; only
/// the first call of a turn is checked.
const NO_SIGNATURE: &str = "skip_thought_signature_validator";

fn sign_tool_calls(messages: &mut [Value]) {
    for message in messages.iter_mut().filter(|m| m["role"] == "assistant") {
        let Some(first) = message.get_mut("tool_calls").and_then(Value::as_array_mut).and_then(|c| c.first_mut()) else { continue };
        if first.pointer("/extra_content/google/thought_signature").is_none() {
            first["extra_content"] = serde_json::json!({"google": {"thought_signature": NO_SIGNATURE}});
        }
    }
}

pub(super) fn chat(client: &Client, request: &Value) -> Result<Value, CloudError> {
    let body = serde_json::to_vec(&chat_request(client, request)).map_err(|e| CloudError::Invalid { provider: client.model.provider.name, why: e.to_string() })?;
    let agent = if request.get("response_format").is_some() { json_agent() } else { agent() };
    let sent = authorized(client, agent.post(url(client, "/chat/completions"))).header("Content-Type", "application/json").send(&body[..]);
    let reply = json(client, &read(client, sent)?)?;
    reply.pointer("/choices/0/message").cloned().ok_or_else(|| CloudError::Invalid {
        provider: client.model.provider.name,
        why: format!("no message in {}", reply.to_string().chars().take(300).collect::<String>()),
    })
}

/// [`chat`], streamed: the words go to `on` as the model writes them.
pub(super) fn chat_stream(client: &Client, request: &Value, on: OnDelta) -> Result<Value, CloudError> {
    let provider = client.model.provider.name;
    let mut body = chat_request(client, request);
    body["stream"] = true.into();
    let body = serde_json::to_vec(&body).map_err(|e| CloudError::Invalid { provider, why: e.to_string() })?;
    let sent = authorized(client, agent().post(url(client, "/chat/completions"))).header("Content-Type", "application/json").send(&body[..]);
    let response = open(client, sent)?;
    stream::chat_message(response.into_body().into_reader(), on).map_err(|why| CloudError::Network { provider, why })
}

pub(super) fn transcribe(client: &Client, wav: &[u8], language: Option<&str>, prompt: &str) -> Result<TranscriptionReply, CloudError> {
    let mut fields = vec![("model", client.model.model.model), ("response_format", "verbose_json"), ("temperature", "0"), ("prompt", prompt)];
    if let Some(l) = language {
        fields.push(("language", l));
    }
    let (content_type, body) = crate::stt::multipart(wav, &fields).map_err(|e| CloudError::Invalid { provider: client.model.provider.name, why: e.to_string() })?;
    let sent = authorized(client, agent().post(url(client, "/audio/transcriptions"))).header("Content-Type", content_type).send(&body[..]);
    json(client, &read(client, sent)?)
}

pub(super) fn speak(client: &Client, text: &str) -> Result<Vec<u8>, CloudError> {
    let m = client.model.model;
    let body = serde_json::json!({"model": m.model, "voice": m.voice.unwrap_or_default(), "input": text, "response_format": "wav"}).to_string();
    let sent = authorized(client, agent().post(url(client, "/audio/speech"))).header("Content-Type", "application/json").send(body.as_bytes());
    read(client, sent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::CloudRef;
    use serde_json::json;

    #[test]
    fn chat_request_drops_llama_settings_and_adds_params() {
        let client = Client { model: CloudRef::by_id("groq:openai/gpt-oss-20b").unwrap(), key: "k".into(), voice_id: None, name: None };
        let request = json!({"messages": [], "tools": [], "tool_choice": "auto", "top_k": 20, "max_tokens": 256, "chat_template_kwargs": {}});
        let body = chat_request(&client, &request);
        assert_eq!(body["model"], "openai/gpt-oss-20b");
        assert_eq!(body["max_tokens"], 1024);
        assert_eq!(body["reasoning_effort"], "low");
        assert!(body.get("top_k").is_none() && body.get("chat_template_kwargs").is_none());
        let body = chat_request(&client, &json!({"messages": [], "tool_choice": "none"}));
        assert!(body.get("tool_choice").is_none());
        // More than the params' floor is kept.
        assert_eq!(chat_request(&client, &json!({"messages": [], "max_tokens": 4096}))["max_tokens"], 4096);
    }

    #[test]
    fn openai_counts_completion_tokens_and_ollama_takes_any_model() {
        let openai = Client { model: CloudRef::by_id("openai:gpt-6-luna").unwrap(), key: "k".into(), voice_id: None, name: None };
        let body = chat_request(&openai, &json!({"messages": [], "max_tokens": 4096}));
        assert_eq!((body["model"].as_str(), body["max_completion_tokens"].as_u64()), (Some("gpt-6-luna"), Some(4096)));
        assert!(body.get("max_tokens").is_none());
        assert_eq!(body["reasoning_effort"], "none");
        let ollama = Client::new("ollama:qwen3:8b", crate::models::Kind::Llm).unwrap();
        assert_eq!(ollama.model_name(), "qwen3:8b");
        let body = chat_request(&ollama, &json!({"messages": [], "max_tokens": 1024}));
        assert_eq!((body["model"].as_str(), body["max_tokens"].as_u64()), (Some("qwen3:8b"), Some(1024)));
        assert!(Client::new("ollama:", crate::models::Kind::Llm).is_err());
    }

    #[test]
    fn reads_groqs_limits() {
        let day = "Rate limit reached for model `openai/gpt-oss-120b` in organization `org_1` service tier `on_demand` on tokens per day (TPD): \
                   Limit 200000, Used 199655, Requested 1530. Please try again in 8m31.2s. Need more tokens?";
        let (wait, per_day) = limit_of(day);
        assert!(per_day);
        assert_eq!(wait.map(|d| d.as_millis()), Some(511_200));
        let minute = "Rate limit reached ... on tokens per minute (TPM): Limit 8000, Used 7000, Requested 1900. Please try again in 6.75s.";
        assert_eq!(limit_of(minute), (Some(Duration::from_millis(6750)), false));
        assert_eq!(parse_wait("1h2m3s"), Some(Duration::from_secs(3723)));
        assert_eq!(parse_wait("450ms"), Some(Duration::from_millis(450)));
        assert_eq!(limit_of("too many"), (None, false));
    }

    #[test]
    fn reads_the_error_message() {
        assert_eq!(error_message(br#"{"error": {"message": "model not found", "type": "invalid_request_error"}}"#), "model not found");
        assert_eq!(error_message(br#"[{"error": {"code": 400, "message": "API key not valid. Please pass a valid API key."}}]"#), "API key not valid. Please pass a valid API key.");
        assert_eq!(error_message(b"Bad Gateway"), "Bad Gateway");
    }

    #[test]
    fn reads_googles_limits() {
        let day = br#"[{"error": {"code": 429, "status": "RESOURCE_EXHAUSTED", "message": "You exceeded your current quota", "details": [
            {"@type": "type.googleapis.com/google.rpc.QuotaFailure", "violations": [{"quotaId": "GenerateRequestsPerDayPerProjectPerModel-FreeTier"}]},
            {"@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "34s"}]}}]"#;
        assert_eq!(google_limit(day), Some((Some(Duration::from_secs(34)), true)));
        let minute = br#"{"error": {"details": [{"violations": [{"quotaId": "GenerateRequestsPerMinutePerProjectPerModel-FreeTier"}]}]}}"#;
        assert_eq!(google_limit(minute), Some((None, false)));
        assert_eq!(google_limit(br#"{"error": {"message": "too many"}}"#), None);
    }

    #[test]
    fn gemini_gets_its_params_and_signed_calls() {
        let client = Client { model: CloudRef::by_id("google:gemini-3.6-flash").unwrap(), key: "k".into(), voice_id: None, name: None };
        let call = |sig: Option<&str>| {
            let mut c = json!({"id": "c", "type": "function", "function": {"name": "f", "arguments": "{}"}});
            if let Some(s) = sig {
                c["extra_content"] = json!({"google": {"thought_signature": s}});
            }
            c
        };
        let request = json!({"messages": [
            {"role": "assistant", "content": null, "tool_calls": [call(Some("real")), call(None)]},
            {"role": "assistant", "content": null, "tool_calls": [call(None)]},
            {"role": "assistant", "content": "ok"},
        ], "max_tokens": 1024});
        let body = chat_request(&client, &request);
        assert_eq!((body["model"].as_str(), body["reasoning_effort"].as_str(), body["max_tokens"].as_u64()), (Some("gemini-3.6-flash"), Some("low"), Some(4096)));
        let sig = |m: usize, c: usize| body["messages"][m]["tool_calls"][c].pointer("/extra_content/google/thought_signature").cloned();
        assert_eq!(sig(0, 0), Some(json!("real")));
        assert_eq!(sig(0, 1), None);
        assert_eq!(sig(1, 0), Some(json!(NO_SIGNATURE)));
        // Other providers' calls are sent as they are.
        let groq = Client { model: CloudRef::by_id("groq:openai/gpt-oss-20b").unwrap(), key: "k".into(), voice_id: None, name: None };
        assert!(chat_request(&groq, &request)["messages"][1]["tool_calls"][0].get("extra_content").is_none());
    }
}
