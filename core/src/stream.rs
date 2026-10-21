//! Answers that arrive a piece at a time: what a streamed chat request
//! hands its caller while the model writes ([`Delta`]), and the readers for
//! the two forms services stream in, server-sent events and one JSON
//! object a line.
//!
//! The voice speaks a sentence as soon as it is written instead of waiting
//! for the whole answer ([`crate::voice`]).

use std::io::{self, BufRead, BufReader, Read};

use serde_json::{Value, json};

/// A piece of an answer being written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delta<'a> {
    /// More of the words.
    Text(&'a str),
    /// The model called a tool: the words so far may not be the answer,
    /// and it will be a while before more come.
    Tools,
}

/// Where a streamed request's pieces go.
pub type OnDelta<'a> = &'a mut dyn FnMut(Delta);

/// For a request nobody listens to as it comes.
pub fn ignore(_: Delta) {}

/// Each line of `reader` as it arrives, without its line break, until
/// `on_line` says to stop (`false`) or the stream ends.
pub(crate) fn lines(reader: impl Read, mut on_line: impl FnMut(&str) -> Result<bool, String>) -> Result<(), String> {
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return Ok(()),
            Ok(_) => {
                if !on_line(line.trim_end_matches(['\r', '\n']))? {
                    return Ok(());
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// The `data` of each server-sent event, until `[DONE]`. Events here fit
/// on one `data:` line each (OpenAI's and Anthropic's do).
pub(crate) fn events(reader: impl Read, mut on_data: impl FnMut(&str) -> Result<(), String>) -> Result<(), String> {
    lines(reader, |line| {
        let Some(data) = line.strip_prefix("data:") else { return Ok(true) };
        let data = data.trim_start();
        if data == "[DONE]" {
            return Ok(false);
        }
        on_data(data).map(|_| true)
    })
}

/// An OpenAI chat completion put back together from its streamed chunks.
#[derive(Debug, Default)]
pub(crate) struct ChatMessage {
    content: String,
    calls: Vec<Value>,
}

impl ChatMessage {
    /// One chunk (`{"choices": [{"delta": …}]}`): its words go to `on` too.
    /// An error the service sends instead of a chunk is returned.
    pub fn add(&mut self, chunk: &Value, on: OnDelta) -> Result<(), String> {
        if let Some(error) = chunk.get("error") {
            return Err(error["message"].as_str().map_or_else(|| error.to_string(), str::to_string));
        }
        let Some(delta) = chunk.pointer("/choices/0/delta") else { return Ok(()) };
        if let Some(text) = delta["content"].as_str().filter(|t| !t.is_empty()) {
            self.content.push_str(text);
            on(Delta::Text(text));
        }
        for call in delta["tool_calls"].as_array().into_iter().flatten() {
            let index = call["index"].as_u64().map_or(self.calls.len().saturating_sub(1), |i| i as usize);
            if index >= self.calls.len() {
                if self.calls.is_empty() {
                    on(Delta::Tools);
                }
                self.calls.resize_with(index + 1, || json!({"id": "", "type": "function", "function": {"name": "", "arguments": ""}}));
            }
            let slot = &mut self.calls[index];
            if let Some(id) = call["id"].as_str().filter(|id| !id.is_empty()) {
                slot["id"] = id.into();
            }
            for field in ["name", "arguments"] {
                if let Some(piece) = call.pointer(&format!("/function/{field}")).and_then(Value::as_str) {
                    let whole = format!("{}{piece}", slot["function"][field].as_str().unwrap_or_default());
                    slot["function"][field] = whole.into();
                }
            }
        }
        Ok(())
    }

    /// The message, as a request that wasn't streamed would have it.
    pub fn message(self) -> Value {
        let mut message = json!({"role": "assistant", "content": self.content});
        if !self.calls.is_empty() {
            let calls: Vec<Value> = self
                .calls
                .into_iter()
                .enumerate()
                .map(|(i, mut call)| {
                    if call["id"].as_str().is_none_or(str::is_empty) {
                        call["id"] = format!("call_{i}").into();
                    }
                    call
                })
                .collect();
            message["tool_calls"] = calls.into();
        }
        message
    }
}

/// A streamed chat completion read from `reader`: the message, its words
/// handed to `on` as they come.
pub(crate) fn chat_message(reader: impl Read, on: OnDelta) -> Result<Value, String> {
    let mut message = ChatMessage::default();
    events(reader, |data| {
        let chunk: Value = serde_json::from_str(data).map_err(|e| format!("bad chunk: {e}"))?;
        message.add(&chunk, on)
    })?;
    Ok(message.message())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_become_one_message() {
        let stream = concat!(
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"Ci\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"ao.\"}}]}\n\n",
            ": a comment\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"set_face\",\"arguments\":\"{\\\"fa\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"ce\\\":1}\"}}]}}]}\n\n",
            "data: [DONE]\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"never read\"}}]}\n\n",
        );
        let mut seen = Vec::new();
        let message = chat_message(stream.as_bytes(), &mut |d| seen.push(format!("{d:?}"))).unwrap();
        assert_eq!(message["content"], "Ciao.");
        assert_eq!(message["tool_calls"][0]["id"], "c1");
        assert_eq!(message["tool_calls"][0]["function"]["name"], "set_face");
        assert_eq!(message["tool_calls"][0]["function"]["arguments"], "{\"face\":1}");
        assert_eq!(seen, ["Text(\"Ci\")", "Text(\"ao.\")", "Tools"]);
    }

    #[test]
    fn an_error_in_the_stream_is_returned() {
        let stream = "data: {\"error\":{\"message\":\"overloaded\"}}\n\n";
        assert_eq!(chat_message(stream.as_bytes(), &mut ignore).unwrap_err(), "overloaded");
    }
}
