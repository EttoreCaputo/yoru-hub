//! A track's transcript made by Google's Gemini instead of Parakeet, when
//! the settings choose a Gemini model (docs/meetings.md, Transcript). The
//! audio goes up through the Files API (a recording is often more than the
//! 20 MB a request can carry), `generateContent` answers with timed
//! segments as JSON, and the file is deleted from Google at once (it would
//! be after 48 hours anyway). Gemini hears the whole track, so it can tell
//! the speakers apart itself; its times are to the second.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::cloud::GOOGLE;

/// Google's native API; the assistant speaks its OpenAI-compatible one.
const ROOT: &str = "https://generativelanguage.googleapis.com";
/// An hour's recording takes a few minutes to go up and to transcribe.
const TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// How long Google may take to make an uploaded file ready.
const PROCESSING: Duration = Duration::from_secs(5 * 60);
/// Formats Gemini reads as they are; anything else becomes AAC first.
const READABLE: &[(&str, &str)] = &[("mp3", "audio/mp3"), ("wav", "audio/wav"), ("aiff", "audio/aiff"), ("aif", "audio/aiff"), ("flac", "audio/flac"), ("ogg", "audio/ogg"), ("aac", "audio/aac"), ("m4a", "audio/m4a")];

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| ureq::Agent::new_with_config(ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(TIMEOUT)).build()))
}

/// Transcribe the audio at `path` with Gemini `model`: segments as the
/// helper gives them (`text`, `start_ms`, `end_ms`, `speaker`).
/// `on_progress` hears the phase (`load` while uploading, then
/// `transcribe`) and how far it got.
pub(super) fn transcribe(path: &Path, model: &str, language: Option<&str>, speakers: bool, mut on_progress: impl FnMut(&str, f64)) -> io::Result<Value> {
    let key = GOOGLE.api_key().ok_or_else(|| io::Error::other("Google: no API key (add one in Settings)"))?;
    on_progress("load", 0.0);
    let audio = readable(path)?;
    let file = upload(&key, &audio.path, audio.mime)?;
    let mime = audio.mime;
    drop(audio);
    let name = file["name"].as_str().unwrap_or_default().to_string();
    let result = ready(&key, file).and_then(|file| {
        on_progress("transcribe", 0.5);
        generate(&key, model, &file, mime, language, speakers)
    });
    if !name.is_empty() {
        let _ = agent().delete(format!("{ROOT}/v1beta/{name}")).header("x-goog-api-key", &key).call();
    }
    let segments = result?;
    on_progress("transcribe", 1.0);
    Ok(segments)
}

/// A private temporary directory owns any converted audio and removes it even
/// if conversion or uploading fails. The original track is never touched.
struct Readable {
    path: PathBuf,
    mime: &'static str,
    _temp: Option<tempfile::TempDir>,
}

/// Gemini accepts M4A directly. For CAF and other formats, decode to mono
/// PCM first: afconvert can't transcode some stereo AAC sources to mono AAC
/// in one step (ExtAudioFileSetProperty cclo, -66564).
fn readable(path: &Path) -> io::Result<Readable> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    if let Some((_, mime)) = READABLE.iter().find(|(e, _)| *e == ext) {
        return Ok(Readable { path: path.to_path_buf(), mime, _temp: None });
    }
    let temp = tempfile::Builder::new().prefix("yoru-gemini-").tempdir()?;
    let pcm = temp.path().join("audio.wav");
    let copy = temp.path().join("audio.aac");
    convert(&["-f", "WAVE", "-d", "LEI16@16000", "-c", "1"], path, &pcm)?;
    convert(&["-f", "adts", "-d", "aac@16000", "-c", "1", "-b", "48000"], &pcm, &copy)?;
    fs::remove_file(&pcm)?;
    Ok(Readable { path: copy, mime: "audio/aac", _temp: Some(temp) })
}

fn convert(options: &[&str], input: &Path, output: &Path) -> io::Result<()> {
    let result = Command::new("/usr/bin/afconvert").args(options).arg(input).arg(output).output().map_err(|e| io::Error::other(format!("couldn't start audio conversion for Gemini: {e}")))?;
    if result.status.success() {
        return Ok(());
    }
    let detail: String = String::from_utf8_lossy(&result.stderr).trim().chars().take(300).collect();
    Err(io::Error::other(if detail.is_empty() { format!("couldn't convert the audio for Gemini ({})", result.status) } else { format!("couldn't convert the audio for Gemini: {detail}") }))
}

/// Send `audio` with the Files API's resumable upload: its `file` record.
fn upload(key: &str, audio: &Path, mime: &str) -> io::Result<Value> {
    let bytes = fs::read(audio)?;
    let start = agent()
        .post(format!("{ROOT}/upload/v1beta/files"))
        .header("x-goog-api-key", key)
        .header("X-Goog-Upload-Protocol", "resumable")
        .header("X-Goog-Upload-Command", "start")
        .header("X-Goog-Upload-Header-Content-Length", bytes.len().to_string())
        .header("X-Goog-Upload-Header-Content-Type", mime)
        .header("Content-Type", "application/json")
        .send(json!({"file": {"display_name": "meeting"}}).to_string().as_bytes())
        .map_err(network)?;
    let start = checked(start)?;
    let url = start.headers().get("x-goog-upload-url").and_then(|v| v.to_str().ok()).ok_or_else(|| io::Error::other("Google: no upload address"))?.to_string();
    let sent = agent()
        .post(url)
        .header("X-Goog-Upload-Offset", "0")
        .header("X-Goog-Upload-Command", "upload, finalize")
        .send(&bytes[..])
        .map_err(network)?;
    let reply = body(checked(sent)?)?;
    Ok(reply["file"].clone())
}

/// `file` once Google has processed it (audio takes a few seconds).
fn ready(key: &str, mut file: Value) -> io::Result<Value> {
    let since = Instant::now();
    while file["state"] == "PROCESSING" {
        if since.elapsed() > PROCESSING {
            return Err(io::Error::other("Google: the audio is still being processed"));
        }
        thread::sleep(Duration::from_secs(2));
        let name = file["name"].as_str().unwrap_or_default();
        file = body(checked(agent().get(format!("{ROOT}/v1beta/{name}")).header("x-goog-api-key", key).call().map_err(network)?)?)?;
    }
    if file["state"] == "FAILED" {
        return Err(io::Error::other("Google couldn't read the audio"));
    }
    Ok(file)
}

/// Ask `model` for the transcript of the uploaded `file`. It thinks a little
/// if it can: a model that only transcribes refuses any thinking setting, so
/// that refusal is asked again without one.
fn generate(key: &str, model: &str, file: &Value, mime: &str, language: Option<&str>, speakers: bool) -> io::Result<Value> {
    let mut request = request(file, mime, language, speakers);
    match send(key, model, &request) {
        Err(e) if refuses_thinking(&e.to_string()) => {
            request["generationConfig"].as_object_mut().map(|c| c.remove("thinkingConfig"));
            send(key, model, &request)
        }
        answer => answer,
    }
}

/// Google's 400 for a thinking setting the model doesn't have.
fn refuses_thinking(error: &str) -> bool {
    let error = error.to_lowercase();
    error.contains("thinking") && error.contains("not supported")
}

fn request(file: &Value, mime: &str, language: Option<&str>, speakers: bool) -> Value {
    json!({
        "contents": [{"role": "user", "parts": [
            {"fileData": {"mimeType": mime, "fileUri": file["uri"]}},
            {"text": prompt(language, speakers)},
        ]}],
        "generationConfig": {
            "responseMimeType": "application/json",
            "responseSchema": schema(speakers),
            "maxOutputTokens": 65_536,
            "thinkingConfig": {"thinkingLevel": "low"},
        },
    })
}

fn send(key: &str, model: &str, request: &Value) -> io::Result<Value> {
    let sent = agent()
        .post(format!("{ROOT}/v1beta/models/{model}:generateContent"))
        .header("x-goog-api-key", key)
        .header("Content-Type", "application/json")
        .send(request.to_string().as_bytes())
        .map_err(network)?;
    let reply = body(checked(sent)?)?;
    let candidate = &reply["candidates"][0];
    if candidate["finishReason"] == "MAX_TOKENS" {
        return Err(io::Error::other("Gemini: the recording is too long for one answer"));
    }
    let text: String = candidate["content"]["parts"].as_array().into_iter().flatten().filter(|p| p["thought"] != true).filter_map(|p| p["text"].as_str()).collect();
    if text.trim().is_empty() {
        let why = candidate["finishReason"].as_str().or(reply.pointer("/promptFeedback/blockReason").and_then(Value::as_str)).unwrap_or("no answer");
        return Err(io::Error::other(format!("Gemini didn't transcribe it ({why})")));
    }
    let segments: Value = serde_json::from_str(&text).map_err(|e| io::Error::other(format!("Gemini: unreadable answer ({e})")))?;
    Ok(segments_of(&segments))
}

fn prompt(language: Option<&str>, speakers: bool) -> String {
    let language = match language {
        Some(l) => format!("The speech is in the language whose ISO 639-1 code is `{l}`; write it in that language."),
        None => "Write it in the language spoken.".into(),
    };
    let speaker = if speakers {
        " `speaker` is a number for who speaks: 0 for the first voice heard, 1 for the next new one, and so on, the same number whenever the same person speaks again."
    } else {
        ""
    };
    format!(
        "Transcribe this recording word for word. {language} Split it into segments, each one speaker's sentence or few sentences, at most about 30 seconds. \
         `start` and `end` are times from the start of the recording as MM:SS (minutes go past 59 in a long recording).{speaker} \
         `text` is exactly what is said, with punctuation. Leave out silence, music and noise. Don't summarize, translate or correct anything."
    )
}

fn schema(speakers: bool) -> Value {
    let mut properties = json!({"start": {"type": "STRING"}, "end": {"type": "STRING"}, "text": {"type": "STRING"}});
    let mut required = vec!["start", "end", "text"];
    if speakers {
        properties["speaker"] = json!({"type": "INTEGER"});
        required.push("speaker");
    }
    json!({"type": "ARRAY", "items": {"type": "OBJECT", "properties": properties, "required": required, "propertyOrdering": ["start", "end", "speaker", "text"]}})
}

/// Gemini's segments in the helper's shape.
fn segments_of(answer: &Value) -> Value {
    let segments: Vec<Value> = answer
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let start = ms(s["start"].as_str()?)?;
            let end = s["end"].as_str().and_then(ms).filter(|e| *e >= start).unwrap_or(start);
            Some(json!({"text": s["text"], "start_ms": start, "end_ms": end, "speaker": s["speaker"]}))
        })
        .collect();
    Value::Array(segments)
}

/// `MM:SS`, `HH:MM:SS`, either with a fraction of a second, in milliseconds.
fn ms(time: &str) -> Option<u64> {
    let mut seconds = 0.0;
    for part in time.trim().split(':') {
        seconds = seconds * 60.0 + part.trim().parse::<f64>().ok()?;
    }
    (seconds >= 0.0).then(|| (seconds * 1000.0).round() as u64)
}

fn network(e: ureq::Error) -> io::Error {
    io::Error::other(format!("Google: {e}"))
}

/// A 2xx answer, or Google's message as the error.
fn checked(mut response: ureq::http::Response<ureq::Body>) -> io::Result<ureq::http::Response<ureq::Body>> {
    let status = response.status().as_u16();
    if (200..300).contains(&status) {
        return Ok(response);
    }
    let bytes = response.body_mut().with_config().limit(1024 * 1024).read_to_vec().unwrap_or_default();
    let message = serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| String::from_utf8_lossy(&bytes).chars().take(300).collect());
    Err(io::Error::other(match status {
        401 | 403 => "Google: the API key was refused".to_string(),
        429 => format!("Google: limit reached ({message})"),
        _ => format!("Google ({status}): {message}"),
    }))
}

fn body(mut response: ureq::http::Response<ureq::Body>) -> io::Result<Value> {
    let bytes = response.body_mut().with_config().limit(64 * 1024 * 1024).read_to_vec().map_err(network)?;
    serde_json::from_slice(&bytes).map_err(|e| io::Error::other(format!("Google: unreadable answer ({e})")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_without_thinking_is_told_apart() {
        assert!(refuses_thinking("Google (400): Thinking level is not supported for this model"));
        assert!(!refuses_thinking("Google (400): Request contains an invalid argument"));
    }

    #[test]
    fn reads_times() {
        assert_eq!(ms("01:05"), Some(65_000));
        assert_eq!(ms("75:00"), Some(4_500_000));
        assert_eq!(ms("1:02:03.5"), Some(3_723_500));
        assert_eq!(ms("soon"), None);
    }

    #[test]
    fn makes_the_helpers_segments() {
        let answer = json!([{"start": "00:01", "end": "00:04", "speaker": 1, "text": "Ciao."}, {"start": "x", "text": "?"}]);
        let segments = segments_of(&answer);
        assert_eq!(segments, json!([{"text": "Ciao.", "start_ms": 1000, "end_ms": 4000, "speaker": 1}]));
    }

    #[test]
    fn m4a_is_sent_without_conversion_or_deleting_the_original() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("meeting.M4A");
        fs::write(&path, b"original").unwrap();
        let audio = readable(&path).unwrap();
        assert_eq!(audio.path, path);
        assert_eq!(audio.mime, "audio/m4a");
        drop(audio);
        assert_eq!(fs::read(path).unwrap(), b"original");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn converts_stereo_and_mono_caf_without_touching_the_original() {
        let folder = tempfile::tempdir().unwrap();
        // Speech only on the right: downmixing must not silently drop it.
        for channels in [1u16, 2] {
            let wav = folder.path().join(format!("{channels}-input.wav"));
            let mut samples = Vec::new();
            for i in 0..48_000 {
                let right = ((i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 12_000.0) as i16;
                if channels == 2 { samples.extend_from_slice(&0i16.to_le_bytes()); }
                samples.extend_from_slice(&right.to_le_bytes());
            }
            let mut bytes = Vec::new();
            bytes.extend_from_slice(b"RIFF");
            bytes.extend_from_slice(&(36u32 + samples.len() as u32).to_le_bytes());
            bytes.extend_from_slice(b"WAVEfmt ");
            bytes.extend_from_slice(&16u32.to_le_bytes());
            bytes.extend_from_slice(&1u16.to_le_bytes());
            bytes.extend_from_slice(&channels.to_le_bytes());
            bytes.extend_from_slice(&48_000u32.to_le_bytes());
            bytes.extend_from_slice(&(48_000u32 * channels as u32 * 2).to_le_bytes());
            bytes.extend_from_slice(&(channels * 2).to_le_bytes());
            bytes.extend_from_slice(&16u16.to_le_bytes());
            bytes.extend_from_slice(b"data");
            bytes.extend_from_slice(&(samples.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&samples);
            fs::write(&wav, bytes).unwrap();
            let input = folder.path().join(format!("{channels}.caf"));
            convert(&["-f", "caff", "-d", "aac@48000", "-c", &channels.to_string(), "-b", "128000"], &wav, &input).unwrap();
            let original = fs::read(&input).unwrap();
            let audio = readable(&input).unwrap();
            assert_eq!(audio.mime, "audio/aac");
            assert!(fs::metadata(&audio.path).unwrap().len() > 0);
            let decoded = folder.path().join(format!("{channels}-decoded.wav"));
            convert(&["-f", "WAVE", "-d", "LEI16@16000", "-c", "1"], &audio.path, &decoded).unwrap();
            let bytes = fs::read(decoded).unwrap();
            let data = bytes.windows(4).position(|x| x == b"data").unwrap() + 8;
            let peak = bytes[data..].chunks_exact(2).map(|v| i16::from_le_bytes([v[0], v[1]]).unsigned_abs()).max().unwrap();
            assert!(peak > 100, "the audio was lost with {channels} channels");
            let converted = audio.path.clone();
            drop(audio);
            assert!(!converted.exists());
            assert_eq!(fs::read(input).unwrap(), original);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn conversion_error_keeps_the_original_and_reports_the_cause() {
        let folder = tempfile::tempdir().unwrap();
        let input = folder.path().join("broken.caf");
        fs::write(&input, b"not audio").unwrap();
        let error = match readable(&input) { Ok(_) => panic!("invalid audio was accepted"), Err(e) => e.to_string() };
        assert!(error.contains("couldn't convert the audio for Gemini:"));
        assert_eq!(fs::read(input).unwrap(), b"not audio");
    }
}

#[cfg(test)]
mod live {
    /// `GEMINI_TRY=<audio file> cargo test -p yoru-core --lib gemini::live -- --ignored --nocapture`
    #[test]
    #[ignore = "sends audio to Google"]
    fn transcribes_a_file() {
        let path = std::env::var("GEMINI_TRY").expect("GEMINI_TRY");
        let model = std::env::var("GEMINI_MODEL").unwrap_or("gemini-3.8-flash".into());
        let started = std::time::Instant::now();
        let segments = super::transcribe(path.as_ref(), &model, None, true, |p, f| eprintln!("{p} {f}")).unwrap();
        eprintln!("{:?}", started.elapsed());
        println!("{}", serde_json::to_string_pretty(&segments).unwrap());
    }
}
