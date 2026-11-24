//! Transcripts made by Whisper on a service that speaks OpenAI's API
//! (`groq:whisper-large-v3-turbo`, `openai:whisper-1`): the track is cut
//! into pieces at quiet moments, small enough for the services' 25 MB
//! limit, and each piece's timed segments are moved to where it starts.
//! Whisper doesn't tell speakers apart: each track's lines have none.

use std::fs;
use std::io;
use std::path::Path;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

use super::gemini::{Answer, RATE, convert, cuts, paragraphs, read_wav, wav_of};
use crate::cloud::{Client, CloudError};
use crate::models::Kind;

/// Five minutes of 16 kHz WAV is about 10 MB; the last piece may be half
/// as long again, still well under the 25 MB a request may carry.
const PIECE_MS: usize = 5 * 60 * 1000;
/// How many times a piece waits out a limit for the minute (or the hour,
/// Groq's audio seconds) before giving up.
const WAITS: usize = 5;
/// A segment Whisper is this sure holds no speech is one it made up over
/// silence ("Sottotitoli a cura di…").
const NO_SPEECH: f64 = 0.6;

/// Transcribe the audio at `path` with `model` (`provider:model`): timed
/// segments when `detailed`, else paragraphs. `on_progress` hears `load`,
/// then `transcribe` and how far it got.
pub(super) fn transcribe(path: &Path, model: &str, language: Option<&str>, detailed: bool, mut on_progress: impl FnMut(&str, f64)) -> io::Result<Answer> {
    let client = Client::new(model, Kind::Whisper).map_err(io::Error::other)?;
    on_progress("load", 0.0);
    let temp = tempfile::Builder::new().prefix("yoru-whisper-").tempdir()?;
    let pcm = temp.path().join("audio.wav");
    convert(&["-f", "WAVE", "-d", "LEI16@16000", "-c", "1"], path, &pcm)?;
    let samples = read_wav(&fs::read(&pcm)?).ok_or_else(|| io::Error::other("couldn't read the converted audio"))?;
    fs::remove_file(&pcm)?;
    let pieces = cuts(&samples, PIECE_MS);
    let mut segments = Vec::new();
    let mut text = String::new();
    for (i, (from, to)) in pieces.iter().enumerate() {
        on_progress("transcribe", i as f64 / pieces.len() as f64);
        let reply = ask(&client, &wav_of(&samples[*from..*to]), language)?;
        let offset = (*from * 1000 / RATE) as u64;
        for s in heard(&reply) {
            text.push_str(s["text"].as_str().unwrap_or_default());
            text.push(' ');
            segments.push(json!({"start_ms": s["start_ms"].as_u64().unwrap_or(0) + offset, "end_ms": s["end_ms"].as_u64().unwrap_or(0) + offset, "text": s["text"]}));
        }
    }
    on_progress("transcribe", 1.0);
    Ok(if detailed { Answer { segments: Value::Array(segments), timed: true } } else { Answer { segments: paragraphs(text.trim()), timed: false } })
}

/// One piece, waiting out the service's limits for the minute or the hour;
/// a day's limit ends it.
fn ask(client: &Client, wav: &[u8], language: Option<&str>) -> io::Result<Value> {
    for _ in 0..WAITS {
        match client.transcribe(wav, language, "") {
            Err(CloudError::RateLimited { retry_after, per_day: false, .. }) => thread::sleep(retry_after.unwrap_or(Duration::from_secs(60)) + Duration::from_secs(2)),
            reply => return reply.map_err(|e| io::Error::other(e.to_string())),
        }
    }
    Err(io::Error::other(format!("{}: still busy after waiting several minutes; try again later", client.id())))
}

/// The segments of a `verbose_json` reply that hold speech, in milliseconds
/// from the piece's start; a reply without segments is one, untimed.
fn heard(reply: &Value) -> Vec<Value> {
    let Some(segments) = reply["segments"].as_array() else {
        let text = reply["text"].as_str().unwrap_or_default().trim();
        return if text.is_empty() { Vec::new() } else { vec![json!({"start_ms": 0, "end_ms": 0, "text": text})] };
    };
    let ms = |v: &Value| (v.as_f64().unwrap_or(0.0) * 1000.0).round() as u64;
    segments
        .iter()
        .filter(|s| s["no_speech_prob"].as_f64().unwrap_or(0.0) < NO_SPEECH)
        .filter_map(|s| {
            let text = s["text"].as_str()?.trim();
            (!text.is_empty()).then(|| json!({"start_ms": ms(&s["start"]), "end_ms": ms(&s["end"]), "text": text}))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_speech_in_milliseconds() {
        let reply = json!({"text": "Ciao. Iniziamo.", "segments": [
            {"start": 0.0, "end": 1.52, "text": " Ciao.", "no_speech_prob": 0.01},
            {"start": 1.52, "end": 3.0, "text": " Sottotitoli a cura di QTSS", "no_speech_prob": 0.9},
            {"start": 3.0, "end": 4.25, "text": " Iniziamo.", "no_speech_prob": 0.1},
        ]});
        let got = heard(&reply);
        assert_eq!(got, vec![json!({"start_ms": 0, "end_ms": 1520, "text": "Ciao."}), json!({"start_ms": 3000, "end_ms": 4250, "text": "Iniziamo."})]);
        assert_eq!(heard(&json!({"text": " Solo testo. "})), vec![json!({"start_ms": 0, "end_ms": 0, "text": "Solo testo."})]);
        assert!(heard(&json!({"text": ""})).is_empty());
    }
}

#[cfg(test)]
mod live {
    /// `WHISPER_TRY=<audio file> WHISPER_MODEL=groq:whisper-large-v3-turbo cargo test -p yoru-core --lib whisper::live -- --ignored --nocapture`
    #[test]
    #[ignore = "sends audio to the service"]
    fn transcribes_a_file() {
        let path = std::env::var("WHISPER_TRY").expect("WHISPER_TRY");
        let model = std::env::var("WHISPER_MODEL").unwrap_or("groq:whisper-large-v3-turbo".into());
        let detailed = std::env::var("WHISPER_SIMPLE").is_err();
        let language = std::env::var("WHISPER_LANGUAGE").ok();
        let started = std::time::Instant::now();
        let answer = super::transcribe(path.as_ref(), &model, language.as_deref(), detailed, |p, f| eprintln!("{p} {f}")).unwrap();
        eprintln!("{:?}, timed: {}", started.elapsed(), answer.timed);
        println!("{}", serde_json::to_string_pretty(&answer.segments).unwrap());
    }
}
