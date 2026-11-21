//! How full the brain's context is, and with what: the last request of a
//! conversation's turn, split into Yoru's instructions, his notes, the tools
//! he was given, the conversation and what the tools answered.
//!
//! The total is the provider's own count when it gives one (Claude's
//! `usage`; OpenAI's, Google's and Groq's `prompt_tokens` on an answer that
//! wasn't streamed); the split is by the parts' sizes, scaled to it.
//! Without a count, about four characters make a token: Ollama's
//! `prompt_eval_count` leaves out the prompt it had cached, so it isn't one.

use serde::Serialize;
use serde_json::Value;

/// Characters per token when the provider gives no count.
const CHARS_PER_TOKEN: usize = 4;

/// What one request carried.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContextUse {
    /// The model that read it (`claude-opus-5-5`, `groq:openai/gpt-oss-120b`).
    pub model: String,
    /// Tokens of the request.
    pub tokens: u64,
    /// The model's context window, when known.
    pub window: Option<u64>,
    /// `tokens` is the provider's count, not an estimate.
    pub exact: bool,
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Part {
    /// `instructions`, `memory`, `tools`, `conversation` or `results`.
    pub kind: &'static str,
    pub tokens: u64,
}

/// The parts' sizes, in characters.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Sizes {
    pub instructions: usize,
    pub memory: usize,
    pub tools: usize,
    pub conversation: usize,
    pub results: usize,
}

impl Sizes {
    fn total(&self) -> usize {
        self.instructions + self.memory + self.tools + self.conversation + self.results
    }

    /// Add the messages of a Claude request: tool results (a web search's
    /// too) apart from the rest. Thinking is left out: the API drops it from
    /// turns before the last.
    pub fn claude_messages(&mut self, messages: &[Value]) {
        for block in messages.iter().filter_map(|m| m["content"].as_array()).flatten() {
            match block["type"].as_str().unwrap_or_default() {
                "text" => self.conversation += len(&block["text"]),
                "tool_use" | "server_tool_use" => self.conversation += block["input"].to_string().len() + len(&block["name"]),
                "thinking" | "redacted_thinking" => {}
                _ => self.results += block.get("content").map_or(0, |c| c.to_string().len()),
            }
        }
    }

    /// Add the messages of an OpenAI-style request (system messages apart).
    pub fn compat_messages(&mut self, messages: &[Value]) {
        for m in messages.iter().filter(|m| m["role"] != "system") {
            let content = len(&m["content"]);
            if m["role"] == "tool" {
                self.results += content;
            } else {
                self.conversation += content + m.get("tool_calls").map_or(0, |c| c.to_string().len());
            }
        }
    }

    /// The use, with the provider's count (`measured`) when there is one.
    pub fn measured(self, model: String, measured: Option<u64>, window: Option<u64>) -> ContextUse {
        let chars = self.total().max(1) as f64;
        let tokens = measured.unwrap_or((self.total() / CHARS_PER_TOKEN) as u64);
        let share = |n: usize| (n as f64 * tokens as f64 / chars).round() as u64;
        let parts = [
            ("instructions", self.instructions),
            ("memory", self.memory),
            ("tools", self.tools),
            ("conversation", self.conversation),
            ("results", self.results),
        ]
        .into_iter()
        .map(|(kind, n)| Part { kind, tokens: share(n) })
        .collect();
        ContextUse { model, tokens, window, exact: measured.is_some(), parts }
    }
}

/// A string's length, or the JSON's for anything else (`null`: 0).
fn len(v: &Value) -> usize {
    match v {
        Value::Null => 0,
        Value::String(s) => s.len(),
        v => v.to_string().len(),
    }
}

/// The tokens Claude read: the new ones and the cached ones.
pub fn claude_tokens(usage: &Value) -> Option<u64> {
    let read = |k: &str| usage[k].as_u64().unwrap_or(0);
    usage["input_tokens"].as_u64().map(|n| n + read("cache_read_input_tokens") + read("cache_creation_input_tokens"))
}

/// The tokens an OpenAI-style brain read (`prompt_tokens`).
pub fn compat_tokens(usage: &Value) -> Option<u64> {
    usage["prompt_tokens"].as_u64().filter(|n| *n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn splits_a_claude_request_and_scales_to_the_count() {
        let mut sizes = Sizes { instructions: 400, memory: 200, tools: 200, ..Default::default() };
        sizes.claude_messages(&[
            json!({"role": "user", "content": [{"type": "text", "text": "a".repeat(100)}]}),
            json!({"role": "assistant", "content": [{"type": "thinking", "thinking": "x".repeat(999)}, {"type": "tool_use", "name": "n", "input": {}}]}),
            json!({"role": "user", "content": [{"type": "tool_result", "tool_use_id": "1", "content": "b".repeat(98)}]}),
        ]);
        assert_eq!((sizes.conversation, sizes.results), (103, 100));
        let used = sizes.measured("claude-opus-5-5".into(), Some(2006), Some(1_000_000));
        assert!(used.exact);
        let of = |k: &str| used.parts.iter().find(|p| p.kind == k).unwrap().tokens;
        assert_eq!((of("instructions"), of("memory"), of("results")), (800, 400, 200));
    }

    #[test]
    fn estimates_without_a_count() {
        let mut sizes = Sizes { instructions: 40, ..Default::default() };
        sizes.compat_messages(&[json!({"role": "system", "content": "s"}), json!({"role": "tool", "content": "r".repeat(40)}), json!({"role": "user", "content": "u".repeat(20)})]);
        let used = sizes.measured("groq:x".into(), None, None);
        assert_eq!((used.tokens, used.exact), (25, false));
        assert_eq!(used.parts.iter().map(|p| p.tokens).collect::<Vec<_>>(), [10, 0, 0, 5, 10]);
        assert_eq!(claude_tokens(&json!({"input_tokens": 5, "cache_read_input_tokens": 100})), Some(105));
        assert_eq!(compat_tokens(&json!({"prompt_tokens": 0})), None);
    }
}
