//! Ollama's chat, through its own API (`/api/chat`) rather than the
//! OpenAI-compatible one: only there does `think` work. A model that thinks
//! (lfm2.5, qwen3) then answers its thoughts in their own field, and the
//! reply stays clean; through `/v1` they come back inside the text, as
//! `<think>…`. The rest (the models, the key check) goes through
//! [`super::openai`], which Ollama speaks too.
//!
//! Requests and answers keep OpenAI's shape for the callers: this module
//! turns one into the other.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Map, Value, json};

use super::{Client, CloudError, openai};
use crate::stream::{self, Delta, OnDelta};

/// A model on this computer that thinks as long as it likes can take minutes.
const TIMEOUT: Duration = Duration::from_secs(600);

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| ureq::Agent::new_with_config(ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(TIMEOUT)).build()))
}

/// `http://localhost:11434/v1` → `http://localhost:11434/api/chat`.
fn url(client: &Client) -> String {
    let base = client.model.provider.url();
    format!("{}/api/chat", base.strip_suffix("/v1").unwrap_or(&base))
}

/// Whether the request asks the model to think: its `think`, else
/// llama.cpp's `enable_thinking`; `None`: the model's default.
fn think(request: &Value) -> Option<bool> {
    request["think"].as_bool().or_else(|| request.pointer("/chat_template_kwargs/enable_thinking").and_then(Value::as_bool))
}

/// OpenAI's messages in Ollama's form: a call's arguments are an object, not
/// a string, and a tool's result names the tool instead of the call's id.
fn messages(messages: &[Value]) -> Vec<Value> {
    let mut names: Map<String, Value> = Map::new();
    messages
        .iter()
        .map(|m| {
            let mut m = m.clone();
            if let Some(calls) = m["tool_calls"].as_array_mut() {
                for call in calls.iter_mut() {
                    if let (Some(id), Some(name)) = (call["id"].as_str(), call.pointer("/function/name")) {
                        names.insert(id.to_string(), name.clone());
                    }
                    if let Some(arguments) = call.pointer_mut("/function/arguments")
                        && let Some(text) = arguments.as_str()
                    {
                        *arguments = serde_json::from_str(text).unwrap_or_else(|_| json!({}));
                    }
                }
            }
            if m["content"].is_null() {
                m["content"] = "".into();
            }
            if m["role"] == "tool"
                && let Some(name) = m["tool_call_id"].as_str().and_then(|id| names.get(id))
            {
                m["tool_name"] = name.clone();
            }
            m
        })
        .collect()
}

/// OpenAI's `response_format` as Ollama's `format`.
fn format(response_format: &Value) -> Option<Value> {
    match response_format["type"].as_str()? {
        "json_schema" => response_format.pointer("/json_schema/schema").cloned(),
        "json_object" => Some("json".into()),
        _ => None,
    }
}

/// The body for `/api/chat`. No `max_tokens` from the assistant: a model
/// on this computer costs nothing, and a thought cut short is a broken answer.
fn chat_request(client: &Client, request: &Value) -> Value {
    let mut body = json!({
        "model": client.model_name(),
        "messages": messages(request["messages"].as_array().map(Vec::as_slice).unwrap_or_default()),
        "stream": false,
    });
    if let Some(tools) = request.get("tools").filter(|t| t.as_array().is_some_and(|a| !a.is_empty())) {
        body["tools"] = tools.clone();
    }
    if let Some(think) = think(request) {
        body["think"] = think.into();
    }
    if let Some(format) = format(&request["response_format"]) {
        body["format"] = format;
    }
    let mut options = Map::new();
    for (from, to) in [("temperature", "temperature"), ("top_p", "top_p"), ("top_k", "top_k"), ("seed", "seed"), ("stop", "stop"), ("max_tokens", "num_predict")] {
        if let Some(v) = request.get(from) {
            options.insert(to.into(), v.clone());
        }
    }
    if !options.is_empty() {
        body["options"] = Value::Object(options);
    }
    body
}

/// Ollama's answer as OpenAI's message: the calls get an id, their
/// arguments a string; the thoughts are left out.
fn message(reply: &Value) -> Option<Value> {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let m = reply.get("message")?;
    let mut out = json!({"role": "assistant", "content": m["content"].as_str().unwrap_or_default()});
    let calls: Vec<Value> = m["tool_calls"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|call| {
            let id = call["id"].as_str().map(str::to_string).unwrap_or_else(|| format!("call_{}", NEXT_ID.fetch_add(1, Ordering::Relaxed)));
            let arguments = match &call["function"]["arguments"] {
                Value::String(s) => s.clone(),
                Value::Null => "{}".into(),
                v => v.to_string(),
            };
            json!({"id": id, "type": "function", "function": {"name": call["function"]["name"], "arguments": arguments}})
        })
        .collect();
    if !calls.is_empty() {
        out["tool_calls"] = calls.into();
    }
    Some(out)
}

pub(super) fn chat(client: &Client, request: &Value) -> Result<Value, CloudError> {
    let provider = client.model.provider.name;
    let body = serde_json::to_vec(&chat_request(client, request)).map_err(|e| CloudError::Invalid { provider, why: e.to_string() })?;
    let sent = agent().post(url(client)).header("Content-Type", "application/json").send(&body[..]);
    let reply: Value = serde_json::from_slice(&openai::read(client, sent)?).map_err(|e| CloudError::Invalid { provider, why: e.to_string() })?;
    message(&reply).ok_or_else(|| CloudError::Invalid { provider, why: format!("no message in {}", reply.to_string().chars().take(300).collect::<String>()) })
}

/// [`chat`], streamed: a JSON object a line, its words handed to `on` as
/// they come. A line's calls are whole, not in pieces.
pub(super) fn chat_stream(client: &Client, request: &Value, on: OnDelta) -> Result<Value, CloudError> {
    let provider = client.model.provider.name;
    let mut body = chat_request(client, request);
    body["stream"] = true.into();
    let body = serde_json::to_vec(&body).map_err(|e| CloudError::Invalid { provider, why: e.to_string() })?;
    let response = openai::open(client, agent().post(url(client)).header("Content-Type", "application/json").send(&body[..]))?;
    let (mut content, mut calls) = (String::new(), Vec::new());
    stream::lines(response.into_body().into_reader(), |line| {
        if line.trim().is_empty() {
            return Ok(true);
        }
        let chunk: Value = serde_json::from_str(line).map_err(|e| format!("bad line: {e}"))?;
        if let Some(error) = chunk["error"].as_str() {
            return Err(error.to_string());
        }
        if let Some(text) = chunk.pointer("/message/content").and_then(Value::as_str).filter(|t| !t.is_empty()) {
            content.push_str(text);
            on(Delta::Text(text));
        }
        if let Some(more) = chunk.pointer("/message/tool_calls").and_then(Value::as_array).filter(|c| !c.is_empty()) {
            calls.extend(more.iter().cloned());
            on(Delta::Tools);
        }
        Ok(chunk["done"].as_bool() != Some(true))
    })
    .map_err(|why| CloudError::Network { provider, why })?;
    let reply = json!({"message": {"content": content, "tool_calls": calls}});
    message(&reply).ok_or_else(|| CloudError::Invalid { provider, why: "no message".into() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_goes_to_ollamas_form() {
        let client = Client::new("ollama:lfm2.5:8b", crate::models::Kind::Llm).unwrap();
        let request = json!({
            "messages": [
                {"role": "user", "content": "Ciao"},
                {"role": "assistant", "content": null, "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "notify", "arguments": "{\"text\":\"hi\"}"}}]},
                {"role": "tool", "tool_call_id": "c1", "content": "done"},
            ],
            "tools": [{"type": "function", "function": {"name": "notify"}}],
            "think": true,
            "response_format": {"type": "json_object"},
        });
        let body = chat_request(&client, &request);
        assert_eq!((body["model"].as_str(), body["stream"].as_bool(), body["think"].as_bool()), (Some("lfm2.5:8b"), Some(false), Some(true)));
        assert_eq!(body["messages"][1]["content"], "");
        assert_eq!(body["messages"][1]["tool_calls"][0]["function"]["arguments"]["text"], "hi");
        assert_eq!(body["messages"][2]["tool_name"], "notify");
        assert_eq!(body["format"], "json");
        // No limit unless asked; llama.cpp's switch for thinking is read too.
        assert!(body.get("options").is_none());
        let body = chat_request(&client, &json!({"messages": [], "max_tokens": 80, "chat_template_kwargs": {"enable_thinking": false}}));
        assert_eq!((body["think"].as_bool(), body["options"]["num_predict"].as_u64()), (Some(false), Some(80)));
        assert!(chat_request(&client, &json!({"messages": []})).get("think").is_none());
    }

    #[test]
    fn an_answer_comes_back_in_openais_form() {
        let reply = json!({"message": {"role": "assistant", "content": "", "thinking": "hmm", "tool_calls": [{"function": {"name": "notify", "arguments": {"text": "hi"}}}]}});
        let m = message(&reply).unwrap();
        assert!(m.get("thinking").is_none());
        let call = &m["tool_calls"][0];
        assert!(call["id"].as_str().unwrap().starts_with("call_"));
        assert_eq!(serde_json::from_str::<Value>(call["function"]["arguments"].as_str().unwrap()).unwrap()["text"], "hi");
        assert!(message(&json!({"message": {"content": "Ciao"}})).unwrap().get("tool_calls").is_none());
    }
}
