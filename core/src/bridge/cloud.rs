//! What the board talks to when the computer isn't there (firmware 0.14.0,
//! the `cloud` capability, `cloud/set` in the firmware's `docs/protocol.md`):
//! the same online services the voice uses here, with their keys, so Yoru
//! transcribes, thinks and speaks on its own over WiFi.
//!
//! Speech to text is the voice's cloud Whisper, else Groq's or OpenAI's with
//! a key; the brain is the assistant's when it's online (not Ollama, which
//! the board can't reach); the voice is the cloud voice of the language the
//! answers are in. Without one of the three the board is told to forget
//! them, and goes back to its few English phrases.
//!
//! The keys only travel on USB or on the paired board's sealed WiFi link,
//! and are kept in the board's flash.

use serde_json::{json, Value};

use super::BridgeConfig;
use crate::assistant::Brain;
use crate::cloud::{Api, CloudRef, TAVILY};
use crate::stt::SttEngine;
use crate::tts::Tts;

/// What the board reads of the memory: its prompt has room for little.
const MEMORY_CHARS: usize = 1200;

/// How Yoru talks, the assistant's own words for it.
const CHARACTER: &str = "You are friendly and to the point, with a light touch of warmth and a little playful; \
never chatty, never servile.";

/// `cloud/set`'s arguments, or `{"clear": true}` when something is missing.
pub(super) fn services(config: &BridgeConfig) -> Value {
    let voice = config.voice.lock().unwrap();
    let language = voice.language.clone();
    let stt = stt(voice.stt.as_ref().map(|s| &s.config().engine), language.as_deref());
    let tts = voice.tts.as_ref().and_then(|t| tts(t, language.as_deref()));
    drop(voice);
    let settings = config.assistant.as_ref().map(|a| a.settings());
    let llm = llm(settings.as_ref());
    let (Some(stt), Some(llm), Some(tts)) = (stt, llm, tts) else {
        return json!({"clear": true});
    };
    let mut out = json!({"name": "Yoru", "character": CHARACTER, "stt": stt, "llm": llm, "tts": tts});
    // Optional: with a Tavily key the board looks things up on the web too.
    if let Some(key) = TAVILY.api_key() {
        out["search"] = json!({"url": format!("{}/search", TAVILY.url()), "key": key});
    }
    if let (Some(assistant), Some(settings)) = (&config.assistant, &settings) {
        out["name"] = settings.name.clone().into();
        out["memory"] = assistant.memory.for_prompt_within(MEMORY_CHARS).into();
    }
    out
}

/// The voice's cloud Whisper, else the first of Groq's and OpenAI's with a key.
fn stt(engine: Option<&SttEngine>, language: Option<&str>) -> Option<Value> {
    let chosen = match engine {
        Some(SttEngine::Cloud(client)) if client.model.provider.api == Api::OpenAi => Some(client.model),
        _ => None,
    };
    let model = chosen.or_else(|| {
        ["groq:whisper-large-v3-turbo", "openai:whisper-1"]
            .into_iter()
            .filter_map(CloudRef::by_id)
            .find(|r| r.provider.api_key().is_some())
    })?;
    let mut out = json!({
        "url": format!("{}/audio/transcriptions", model.provider.url()),
        "key": model.provider.api_key()?,
        "model": model.model.model,
    });
    if let Some(l) = language {
        out["language"] = l.into();
    }
    Some(out)
}

/// The assistant's brain, else the first online one with a key.
fn llm(settings: Option<&crate::assistant::Settings>) -> Option<Value> {
    let brain = settings.and_then(|s| s.brain()).filter(|b| *b != Brain::Ollama && b.has_key()).or_else(Brain::auto)?;
    let provider = brain.provider();
    let model = settings.map(|s| s.model_of(brain).to_string()).filter(|m| !m.is_empty()).unwrap_or_else(|| default_model(brain).into());
    let key = provider.api_key()?;
    Some(match provider.api {
        Api::Anthropic => json!({"api": "anthropic", "url": format!("{}/v1/messages", provider.url()), "key": key, "model": model}),
        _ => {
            // The catalog's settings for it: reasoning off, which also lets OpenAI's answer at once.
            let params = provider.models.iter().find(|m| m.id == model).and_then(|m| serde_json::from_str::<Value>(m.params).ok());
            json!({
                "api": "openai",
                "url": format!("{}/chat/completions", provider.url()),
                "key": key,
                "model": model,
                "params": params.unwrap_or_else(|| json!({})),
            })
        }
    })
}

fn default_model(brain: Brain) -> &'static str {
    match brain {
        Brain::Anthropic => crate::assistant::DEFAULT_MODEL,
        Brain::OpenAi => crate::assistant::DEFAULT_OPENAI_MODEL,
        Brain::Groq | Brain::Ollama => crate::assistant::DEFAULT_GROQ_MODEL,
    }
}

/// The cloud voice of the answers' language (Italian when they follow the
/// user's, the owner's), if it is one: ElevenLabs' or Orpheus on Groq.
fn tts(tts: &Tts, language: Option<&str>) -> Option<Value> {
    let id = tts.voice_for(language.unwrap_or("it"))?;
    let client = tts.config().clients.get(id)?;
    let model = client.model;
    let key = model.provider.api_key()?;
    match model.provider.api {
        Api::ElevenLabs => Some(json!({
            "api": "elevenlabs",
            "url": model.provider.url(),
            "key": key,
            "model": model.model.model,
            "voice": client.voice_id.clone()?,
        })),
        Api::OpenAi => Some(json!({
            "api": "openai",
            "url": model.provider.url(),
            "key": key,
            "model": model.model.model,
            "voice": model.model.voice?,
            // Orpheus answers in WAV only, with its rate in the header.
            "format": "wav",
            "max_chars": model.model.max_chars,
        })),
        Api::Anthropic | Api::Tavily => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_without_services() {
        let config = BridgeConfig::default();
        assert_eq!(services(&config), json!({"clear": true}));
    }
}
