//! Google's own API (not its OpenAI-compatible one, which has no speech to
//! text and lists its models without saying what they do): the voice's
//! transcription with Gemini, and every model of the account.
//!
//! A Gemini model hears a WAV through `generateContent` and answers with
//! the words and their language. Google's Live transcription models
//! (`…-transcribe-live`) are made for exactly this, but speak only over the
//! Live API ([`super::live::transcribe_with`]).

use base64::Engine as _;
use serde_json::{Value, json};

use super::openai::{agent, json as body_json, read};
use super::{Client, CloudError, TranscriptionReply};

/// Google's own API; the assistant speaks its OpenAI-compatible one.
const ROOT: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Whether `model` only speaks over the Live API.
fn is_live(model: &str) -> bool {
    model.contains("-live")
}

/// Transcribe a WAV (16 kHz mono, as [`crate::voice::wav`] writes it) in
/// `language` (`it`, `en`) or whichever it hears: whisper-server's reply,
/// with one confident segment (Gemini gives no probabilities).
pub(super) fn transcribe(client: &Client, wav: &[u8], language: Option<&str>, prompt: &str) -> Result<TranscriptionReply, CloudError> {
    let model = client.model_name();
    let (text, heard) = if is_live(model) { live(client, wav, language, prompt)? } else { generate(client, wav, language, prompt)? };
    Ok(json!({"text": text, "language": heard, "segments": [{"avg_logprob": 0.0}]}))
}

fn live(client: &Client, wav: &[u8], language: Option<&str>, prompt: &str) -> Result<(String, String), CloudError> {
    let provider = client.model.provider.name;
    let samples: Vec<i16> = wav.get(44..).unwrap_or_default().chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    let languages: &[&str] = match language {
        Some("it") => &["it-IT"],
        Some("en") => &["en-US"],
        _ => &["it-IT", "en-US"],
    };
    let words: Vec<&str> = prompt.split([',', '.']).map(str::trim).filter(|w| !w.is_empty() && w.split_whitespace().count() <= 2).collect();
    let text = super::live::transcribe_with(&client.key, client.model_name(), &samples, languages, &words).map_err(|why| CloudError::Invalid { provider, why })?;
    let heard = language.map(str::to_string).unwrap_or_else(|| guess_language(&text).into());
    Ok((text, heard))
}

fn generate(client: &Client, wav: &[u8], language: Option<&str>, prompt: &str) -> Result<(String, String), CloudError> {
    let asked = match language {
        Some("it") => "It is in Italian.",
        Some("en") => "It is in English.",
        _ => "It is in Italian or in English.",
    };
    let instructions = format!(
        "Transcribe this recording word for word, in the language spoken. {asked} Write only what is said, with no notes or descriptions; \
         if nothing is said, the text is empty. Words that may come up, spelled this way: {prompt}"
    );
    let body = json!({
        "contents": [{"role": "user", "parts": [
            {"text": instructions},
            {"inline_data": {"mime_type": "audio/wav", "data": base64::engine::general_purpose::STANDARD.encode(wav)}},
        ]}],
        "generationConfig": {
            "temperature": 0,
            "responseMimeType": "application/json",
            "responseSchema": {"type": "OBJECT", "properties": {
                "text": {"type": "STRING"},
                "language": {"type": "STRING", "description": "ISO 639-1 code of the language spoken: it, en…"},
            }, "required": ["text", "language"]},
        },
    });
    let url = format!("{}/models/{}:generateContent", root(client), client.model_name());
    let reply = body_json(client, &read(client, agent().post(url).header("x-goog-api-key", &client.key).header("Content-Type", "application/json").send(&body.to_string().into_bytes()[..]))?)?;
    let provider = client.model.provider.name;
    let answer: String = reply.pointer("/candidates/0/content/parts").and_then(Value::as_array).into_iter().flatten().filter(|p| !p["thought"].as_bool().unwrap_or(false)).filter_map(|p| p["text"].as_str()).collect();
    let said: Value = serde_json::from_str(answer.trim()).map_err(|_| CloudError::Invalid { provider, why: format!("not a transcript: {answer:?}") })?;
    let text = said["text"].as_str().unwrap_or_default().trim().to_string();
    let heard = said["language"].as_str().map(|l| l.trim().to_lowercase()).filter(|l| !l.is_empty()).or(language.map(str::to_string)).unwrap_or_else(|| guess_language(&text).into());
    Ok((text, heard))
}

/// Where Google's own API is: `GEMINI_BASE_URL` points the OpenAI-compatible
/// one elsewhere (a test's server), and this one with it.
fn root(client: &Client) -> String {
    let url = client.model.provider.url();
    url.strip_suffix("/openai").map(str::to_string).unwrap_or_else(|| if url == client.model.provider.base_url { ROOT.into() } else { url })
}

/// Italian or English, by their commonest words: the Live models don't say
/// which they heard.
fn guess_language(text: &str) -> &'static str {
    const IT: &[&str] = &["il", "lo", "la", "di", "che", "e", "è", "un", "una", "per", "non", "mi", "ti", "ci", "sono", "cosa", "come", "quando", "domani", "oggi"];
    const EN: &[&str] = &["the", "a", "an", "of", "and", "is", "to", "i", "you", "it", "what", "how", "when", "tomorrow", "today", "please", "my", "for"];
    let words: Vec<String> = text.split(|c: char| !c.is_alphanumeric() && c != 'è').map(str::to_lowercase).filter(|w| !w.is_empty()).collect();
    let count = |list: &[&str]| words.iter().filter(|w| list.contains(&w.as_str())).count();
    if count(EN) > count(IT) { "en" } else { "it" }
}

/// A model of the account, as Google describes it.
pub(super) struct Listed {
    pub model: String,
    pub name: String,
    pub description: String,
    pub generates: bool,
}

/// Every model the key can use.
pub(super) fn list_models(client: &Client) -> Result<Vec<Listed>, CloudError> {
    let url = format!("{}/models?pageSize=1000", root(client));
    let reply = body_json(client, &read(client, agent().get(url).header("x-goog-api-key", &client.key).call())?)?;
    Ok(reply["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let model = m["name"].as_str()?.trim_start_matches("models/").to_string();
            let methods = m["supportedGenerationMethods"].as_array();
            let generates = methods.is_some_and(|ms| ms.iter().any(|x| x == "generateContent"));
            Some(Listed { model, name: m["displayName"].as_str().unwrap_or_default().into(), description: m["description"].as_str().unwrap_or_default().into(), generates })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_language_is_guessed_from_its_words() {
        assert_eq!(guess_language("che tempo fa domani a Milano"), "it");
        assert_eq!(guess_language("what is the weather like tomorrow"), "en");
        assert_eq!(guess_language(""), "it");
    }

    #[test]
    fn live_models_are_told_apart() {
        assert!(is_live("gemini-3.5-transcribe-live"));
        assert!(!is_live("gemini-3.6-flash"));
    }
}
