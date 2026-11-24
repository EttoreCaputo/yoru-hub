//! A track's transcript made by Google's Gemini instead of Parakeet, when
//! the settings choose a Gemini model (docs/meetings.md, Transcript). The
//! audio goes up through the Files API (a recording is often more than the
//! 20 MB a request can carry), `generateContent` answers with timed
//! segments as JSON, and the file is deleted from Google at once (it would
//! be after 48 hours anyway). Gemini hears the whole track, so it can tell
//! the speakers apart itself; its times are to the second. A simple
//! transcript is only the words, in paragraphs: what a model that only
//! transcribes (`gemini-3.5-transcribe`) can give.

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

/// A track's transcript as Gemini wrote it.
pub(super) struct Answer {
    /// Segments as the helper gives them (`text`, `start_ms`, `end_ms`,
    /// `speaker`); untimed, only `text`, one per paragraph.
    pub segments: Value,
    /// Whether the segments carry times (and speakers, if asked for).
    pub timed: bool,
}

/// Transcribe the audio at `path` with Gemini `model`, `detailed` (timed
/// segments, the speakers told apart if `speakers`) or as plain paragraphs.
/// A detailed answer written without times is kept as paragraphs.
/// `on_progress` hears the phase (`load` while uploading, then
/// `transcribe`) and how far it got.
pub(super) fn transcribe(path: &Path, model: &str, language: Option<&str>, speakers: bool, detailed: bool, mut on_progress: impl FnMut(&str, f64)) -> io::Result<Answer> {
    let key = GOOGLE.api_key().ok_or_else(|| io::Error::other("Google: no API key (add one in Settings)"))?;
    on_progress("load", 0.0);
    if !detailed {
        return simple(&key, path, model, language, on_progress);
    }
    let audio = readable(path)?;
    let answer = with_file(&key, &audio.path, audio.mime, |file| {
        on_progress("transcribe", 0.5);
        let (text, cut) = generate(&key, model, file, audio.mime, language, speakers, true)?;
        if cut {
            return Err(io::Error::other("Gemini: the recording is too long for one answer"));
        }
        Ok(answer_of(&text, true))
    })?;
    on_progress("transcribe", 1.0);
    Ok(answer)
}

/// Upload `audio`, hand it to `job` once Google has it ready, and delete it
/// from Google whatever `job` did.
fn with_file<T>(key: &str, audio: &Path, mime: &str, job: impl FnOnce(&Value) -> io::Result<T>) -> io::Result<T> {
    let file = upload(key, audio, mime)?;
    let name = file["name"].as_str().unwrap_or_default().to_string();
    let result = ready(key, file).and_then(|file| job(&file));
    if !name.is_empty() {
        let _ = agent().delete(format!("{ROOT}/v1beta/{name}")).header("x-goog-api-key", key).call();
    }
    result
}

/// A simple transcript is written a piece of about [`PIECE_MS`] at a time:
/// a model that only transcribes can't be asked to go on ("Multiturn chat
/// is not enabled for this model"), and on a long recording it sometimes
/// says one sentence over and over until its 32,768-token limit.
const PIECE_MS: usize = 10 * 60 * 1000;
/// A piece ends at the quietest half second this close to its end, not in
/// the middle of a word.
const SEEK_MS: usize = 20_000;
/// How many times a piece is asked again when the answer loops.
const TRIES: usize = 3;
const RATE: usize = 16_000;

/// The words of the audio at `path`, in paragraphs, piece by piece.
fn simple(key: &str, path: &Path, model: &str, language: Option<&str>, mut on_progress: impl FnMut(&str, f64)) -> io::Result<Answer> {
    let temp = tempfile::Builder::new().prefix("yoru-gemini-").tempdir()?;
    let pcm = temp.path().join("audio.wav");
    convert(&["-f", "WAVE", "-d", "LEI16@16000", "-c", "1"], path, &pcm)?;
    let samples = read_wav(&fs::read(&pcm)?).ok_or_else(|| io::Error::other("couldn't read the converted audio"))?;
    fs::remove_file(&pcm)?;
    let pieces = cuts(&samples);
    let mut segments = Vec::new();
    for (i, (from, to)) in pieces.iter().enumerate() {
        let wav = temp.path().join("piece.wav");
        let aac = temp.path().join("piece.aac");
        fs::write(&wav, wav_of(&samples[*from..*to]))?;
        convert(&["-f", "adts", "-d", "aac@16000", "-c", "1", "-b", "48000"], &wav, &aac)?;
        on_progress("transcribe", i as f64 / pieces.len() as f64);
        let text = with_file(key, &aac, "audio/aac", |file| piece(key, model, file, language))?;
        if let Value::Array(mut parts) = paragraphs(&text) {
            segments.append(&mut parts);
        }
    }
    on_progress("transcribe", 1.0);
    Ok(Answer { segments: Value::Array(segments), timed: false })
}

/// One piece's words. An answer that loops is asked for again; if every
/// try loops, what came before the loop is kept.
fn piece(key: &str, model: &str, file: &Value, language: Option<&str>) -> io::Result<String> {
    let mut best = String::new();
    for _ in 0..TRIES {
        let (text, cut) = generate(key, model, file, "audio/aac", language, false, false)?;
        match looping(&text) {
            None if !cut => return Ok(text),
            Some(at) if at > best.len() => best = text[..at].to_string(),
            None if text.len() > best.len() => best = text,
            _ => {}
        }
    }
    Ok(best)
}

/// Where to cut `samples` into pieces: about [`PIECE_MS`] each, at a quiet
/// moment; the last piece takes up to half a piece more rather than leave
/// a scrap.
fn cuts(samples: &[i16]) -> Vec<(usize, usize)> {
    let (piece, seek) = (PIECE_MS * RATE / 1000, SEEK_MS * RATE / 1000);
    let mut out = Vec::new();
    let mut from = 0;
    while samples.len() - from > piece * 3 / 2 {
        let to = quietest(samples, from + piece - seek, from + piece + seek);
        out.push((from, to));
        from = to;
    }
    out.push((from, samples.len()));
    out
}

/// The middle of the quietest half second between `from` and `to`.
fn quietest(samples: &[i16], from: usize, to: usize) -> usize {
    let (block, window) = (RATE / 10, 5);
    let loud: Vec<u64> = samples[from..to].chunks(block).map(|b| b.iter().map(|s| s.unsigned_abs() as u64).sum()).collect();
    let best = (0..loud.len().saturating_sub(window)).min_by_key(|&i| loud[i..i + window].iter().sum::<u64>()).unwrap_or(0);
    from + (best * block) + window * block / 2
}

/// The 16-bit samples of a WAV file (its `data` chunk).
fn read_wav(bytes: &[u8]) -> Option<Vec<i16>> {
    if bytes.get(..4)? != b"RIFF" || bytes.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        if &bytes[at..at + 4] == b"data" {
            let data = bytes.get(at + 8..(at + 8 + size).min(bytes.len()))?;
            return Some(data.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect());
        }
        at += 8 + size + size % 2;
    }
    None
}

/// 16 kHz mono 16-bit `samples` as a WAV file.
fn wav_of(samples: &[i16]) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(RATE as u32).to_le_bytes());
    out.extend_from_slice(&(RATE as u32 * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    samples.iter().for_each(|s| out.extend_from_slice(&s.to_le_bytes()));
    out
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

/// Ask `model` for the transcript of the uploaded `file`: the answer's text,
/// and whether it stopped at the most it may write. A detailed one thinks a
/// little and answers in JSON if it can: a model that only transcribes
/// (`gemini-3.5-transcribe`) refuses a thinking setting or JSON mode, so
/// each refusal is asked again without it; the prompt asks for the same
/// JSON anyway. A free tier's limit for the minute is waited out, and a model
/// too busy to answer is asked again.
fn generate(key: &str, model: &str, file: &Value, mime: &str, language: Option<&str>, speakers: bool, detailed: bool) -> io::Result<(String, bool)> {
    let mut request = if detailed { request(file, mime, language, speakers) } else { plain_request(file, mime, language) };
    let (mut waits, mut busy) = (0, 0);
    loop {
        let answer = send(key, model, &request);
        let config = request["generationConfig"].as_object_mut().expect("the request's config");
        match answer {
            Err(e) if refuses_thinking(&e.to_string()) && config.contains_key("thinkingConfig") => {
                config.remove("thinkingConfig");
            }
            Err(e) if refuses_json(&e.to_string()) && config.contains_key("responseMimeType") => {
                config.remove("responseMimeType");
                config.remove("responseSchema");
            }
            Err(e) if waits < 5 && for_the_minute(&e.to_string()) => {
                waits += 1;
                thread::sleep(Duration::from_secs(65));
            }
            Err(e) if busy < 4 && overloaded(&e.to_string()) => {
                busy += 1;
                thread::sleep(Duration::from_secs(10 << busy));
            }
            Err(e) if overloaded(&e.to_string()) => {
                return Err(io::Error::other(format!("Google: {model} is too busy right now (asked for five minutes); try again later")));
            }
            answer => return answer,
        }
    }
}

/// Google's 503 ("This model is currently experiencing high demand") or
/// 500: worth asking again a little later (20 s, 40 s, 80 s, 160 s).
fn overloaded(error: &str) -> bool {
    error.starts_with("Google (503)") || error.starts_with("Google (500)") || error.to_lowercase().contains("high demand")
}

/// Google's 429 for a limit that a minute's wait lifts (not the day's).
fn for_the_minute(error: &str) -> bool {
    let error = error.to_lowercase();
    error.contains("limit reached") && error.contains("per minute")
}

/// Where `text` starts saying the same thing over and over, if it does: a
/// sentence of some length heard a fourth time within the last few, the
/// loop starting at its second. Speech repeats short phrases, not that.
fn looping(text: &str) -> Option<usize> {
    let mut seen: Vec<(&str, usize)> = Vec::new();
    let mut start = 0;
    for (i, c) in text.char_indices() {
        if !matches!(c, '.' | '?' | '!') {
            continue;
        }
        let end = i + c.len_utf8();
        let sentence = text[start..end].trim();
        if sentence.chars().count() >= 20 {
            let recent = &seen[seen.len().saturating_sub(40)..];
            let again: Vec<usize> = recent.iter().filter(|(s, _)| *s == sentence).map(|(_, at)| *at).collect();
            if again.len() >= 3 {
                return Some(again[1]);
            }
        }
        seen.push((sentence, start + (text[start..end].len() - text[start..end].trim_start().len())));
        start = end;
    }
    None
}

/// Google's 400 for a thinking setting the model doesn't have.
fn refuses_thinking(error: &str) -> bool {
    let error = error.to_lowercase();
    error.contains("thinking") && error.contains("not supported")
}

/// Google's 400 for JSON mode on a model without it ("JSON mode is not
/// enabled for this model").
fn refuses_json(error: &str) -> bool {
    let error = error.to_lowercase();
    (error.contains("json mode") || error.contains("response_mime_type") || error.contains("responsemimetype") || error.contains("response schema"))
        && (error.contains("not enabled") || error.contains("not supported"))
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

/// Just the words, in paragraphs: no JSON mode or thinking, which a model
/// that only transcribes doesn't have.
fn plain_request(file: &Value, mime: &str, language: Option<&str>) -> Value {
    json!({
        "contents": [{"role": "user", "parts": [
            {"fileData": {"mimeType": mime, "fileUri": file["uri"]}},
            {"text": plain_prompt(language)},
        ]}],
        "generationConfig": {"maxOutputTokens": 65_536},
    })
}

/// The transcript in `text`: timed segments if `detailed` and they're
/// there, else paragraphs.
fn answer_of(text: &str, detailed: bool) -> Answer {
    match read_answer(text).filter(|s| detailed && s.as_array().is_some_and(|a| !a.is_empty())) {
        Some(segments) => Answer { segments, timed: true },
        None => Answer { segments: paragraphs(text), timed: false },
    }
}

/// Send `request`: the answer's text, and whether it was cut off at the
/// most it may write.
fn send(key: &str, model: &str, request: &Value) -> io::Result<(String, bool)> {
    let sent = agent()
        .post(format!("{ROOT}/v1beta/models/{model}:generateContent"))
        .header("x-goog-api-key", key)
        .header("Content-Type", "application/json")
        .send(request.to_string().as_bytes())
        .map_err(network)?;
    let reply = body(checked(sent)?)?;
    let candidate = &reply["candidates"][0];
    let cut = candidate["finishReason"] == "MAX_TOKENS";
    let text: String = candidate["content"]["parts"].as_array().into_iter().flatten().filter(|p| p["thought"] != true).filter_map(|p| p["text"].as_str()).collect();
    if text.trim().is_empty() && !cut {
        let why = candidate["finishReason"].as_str().or(reply.pointer("/promptFeedback/blockReason").and_then(Value::as_str)).unwrap_or("no answer");
        return Err(io::Error::other(format!("Gemini didn't transcribe it ({why})")));
    }
    Ok((text, cut))
}

/// About how long a paragraph of a simple transcript is.
const PARAGRAPH: usize = 600;

/// Plain text as untimed segments, one per paragraph (or line). A model
/// that only transcribes writes ten minutes as one block: a long one is
/// split after whole sentences, about [`PARAGRAPH`] characters each.
fn paragraphs(text: &str) -> Value {
    let text = text.trim().trim_start_matches("```").trim_end_matches("```");
    let mut parts = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && *l != "```") {
        let mut from = 0;
        for (i, c) in line.char_indices() {
            let end = i + c.len_utf8();
            let ends_sentence = matches!(c, '.' | '?' | '!') && line[end..].starts_with(' ');
            if ends_sentence && end - from >= PARAGRAPH {
                parts.push(json!({"text": line[from..end].trim()}));
                from = end;
            }
        }
        if !line[from..].trim().is_empty() {
            parts.push(json!({"text": line[from..].trim()}));
        }
    }
    Value::Array(parts)
}

/// The segments in an answer: JSON as asked (in JSON mode, or written out
/// with a code fence around it), else one line per segment that starts with
/// its time (`[00:12 - 00:18] Speaker 1: text`), as a model without JSON
/// mode may write.
fn read_answer(text: &str) -> Option<Value> {
    let text = text.trim();
    let json = serde_json::from_str::<Value>(text).ok().or_else(|| {
        let (from, to) = (text.find('[')?, text.rfind(']')?);
        serde_json::from_str::<Value>(text.get(from..=to)?).ok()
    });
    if let Some(answer) = json.filter(Value::is_array) {
        return Some(segments_of(&answer));
    }
    let mut lines: Vec<Value> = text.lines().filter_map(timed_line).collect();
    // Speakers counted from 1 (no "Speaker 0") are counted from 0, as asked.
    if !lines.iter().any(|l| l["speaker"] == 0) {
        for line in lines.iter_mut().filter(|l| l["speaker"].is_u64()) {
            line["speaker"] = json!(line["speaker"].as_u64().unwrap_or(1).saturating_sub(1));
        }
    }
    (!lines.is_empty()).then(|| segments_of(&Value::Array(lines)))
}

/// `[00:12 - 00:18] Speaker 1: text`, `00:12 Speaker 2: text`, `(01:02:03) text`…
/// as a segment in the JSON's shape, the speaker's number as written.
fn timed_line(line: &str) -> Option<Value> {
    let line = line.trim().trim_start_matches(['-', '*', ' ']);
    let is_time = |c: char| c.is_ascii_digit() || c == ':' || c == '.';
    let rest = line.trim_start_matches(['[', '(']);
    let start_len = rest.find(|c: char| !is_time(c)).unwrap_or(rest.len());
    let start = &rest[..start_len];
    if !start.contains(':') {
        return None;
    }
    let mut rest = rest[start_len..].trim_start();
    let mut end = None;
    if let Some(after) = rest.strip_prefix(['-', '–']).map(str::trim_start) {
        let len = after.find(|c: char| !is_time(c)).unwrap_or(after.len());
        if after[..len].contains(':') {
            end = Some(after[..len].to_string());
            rest = after[len..].trim_start();
        }
    }
    let mut rest = rest.trim_start_matches([']', ')', ' ', '-', '–']).trim_start();
    let mut speaker = None;
    if let Some((who, said)) = rest.split_once(':') {
        let who = who.trim().to_lowercase();
        let number = who.strip_prefix("speaker").or_else(|| who.strip_prefix("parlante")).map(str::trim).and_then(|n| n.parse::<u64>().ok());
        if let Some(n) = number {
            speaker = Some(n);
            rest = said.trim_start();
        }
    }
    if rest.is_empty() {
        return None;
    }
    let mut segment = json!({"start": start, "text": rest});
    if let Some(end) = end {
        segment["end"] = json!(end);
    }
    if let Some(n) = speaker {
        segment["speaker"] = json!(n);
    }
    Some(segment)
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
         `text` is exactly what is said, with punctuation. Leave out silence, music and noise. Don't summarize, translate or correct anything. \
         Answer with only a JSON array of these segments, objects with `start`, `end`{}, `text`.",
        if speakers { ", `speaker`" } else { "" }
    )
}

fn plain_prompt(language: Option<&str>) -> String {
    let language = match language {
        Some(l) => format!("The speech is in the language whose ISO 639-1 code is `{l}`; write it in that language."),
        None => "Write it in the language spoken.".into(),
    };
    format!(
        "Transcribe this recording word for word, from start to end. {language} Write plain text with punctuation, in paragraphs: \
         a new paragraph whenever someone else speaks or the subject changes. No times, speaker names, headings or comments. \
         Leave out silence, music and noise. Don't summarize, translate or correct anything."
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
    fn a_model_without_json_mode_is_told_apart() {
        assert!(refuses_json("Google (400): JSON mode is not enabled for this model"));
        assert!(!refuses_json("Google (400): Thinking level is not supported for this model"));
        assert!(!refuses_json("Google (400): Request contains an invalid argument"));
    }

    #[test]
    fn reads_an_answer_written_without_json_mode() {
        let fenced = "```json\n[{\"start\": \"00:01\", \"end\": \"00:04\", \"speaker\": 0, \"text\": \"Ciao.\"}]\n```";
        assert_eq!(read_answer(fenced).unwrap(), json!([{"text": "Ciao.", "start_ms": 1000, "end_ms": 4000, "speaker": 0}]));
        let lines = "[00:01 - 00:04] Speaker 1: Ciao.\n00:05 Speaker 2: Buongiorno a tutti.\nNote: nothing else.";
        assert_eq!(
            read_answer(lines).unwrap(),
            json!([
                {"text": "Ciao.", "start_ms": 1000, "end_ms": 4000, "speaker": 0},
                {"text": "Buongiorno a tutti.", "start_ms": 5000, "end_ms": 5000, "speaker": 1},
            ])
        );
        assert_eq!(read_answer("Just words, no times."), None);
    }

    #[test]
    fn a_looping_answer_is_cut_where_the_loop_begins() {
        let said = "Partiamo dal budget. Ciao, ciao. Ciao, ciao. Ciao, ciao. Ciao, ciao. Fine.";
        assert_eq!(looping(said), None);
        let stuck = "Partiamo dal budget. Poi vediamo i negozi di Roma. Poi vediamo i negozi di Roma. Poi vediamo i negozi di Roma. Poi vediamo i negozi di Roma.";
        let from = looping(stuck).unwrap();
        assert_eq!(&stuck[..from], "Partiamo dal budget. Poi vediamo i negozi di Roma. ");
    }

    #[test]
    fn a_busy_model_is_asked_again() {
        assert!(overloaded("Google (503): This model is currently experiencing high demand. Spikes in demand are usually temporary."));
        assert!(!overloaded("Google (400): Multiturn chat is not enabled for this model"));
    }

    #[test]
    fn a_minute_limit_is_waited_out_but_not_the_days() {
        assert!(for_the_minute("Google: limit reached (Quota exceeded for metric: input_token_count, limit: 10000 … tokens per minute limit exceeded on Free Tier)"));
        assert!(!for_the_minute("Google: limit reached (Quota exceeded: requests per day)"));
    }

    #[test]
    fn long_audio_is_cut_at_a_quiet_moment() {
        let minute = 60 * RATE;
        let mut samples = vec![1000i16; 25 * minute];
        // A quiet second just after ten minutes.
        samples[10 * minute + 5 * RATE..10 * minute + 6 * RATE].fill(0);
        let pieces = cuts(&samples);
        assert_eq!(pieces.len(), 2);
        let cut = pieces[0].1;
        assert!((10 * minute + 5 * RATE..10 * minute + 6 * RATE).contains(&cut), "{cut}");
        assert_eq!(pieces[1], (cut, samples.len()));
        assert_eq!(cuts(&samples[..14 * minute]), vec![(0, 14 * minute)]);
    }

    #[test]
    fn wav_round_trips() {
        let samples = vec![0, 1, -1, i16::MAX, i16::MIN];
        assert_eq!(read_wav(&wav_of(&samples)).unwrap(), samples);
    }

    #[test]
    fn plain_text_becomes_paragraphs() {
        assert_eq!(paragraphs("Ciao a tutti.\n\n  Iniziamo.  \n"), json!([{"text": "Ciao a tutti."}, {"text": "Iniziamo."}]));
        let sentence = format!("{}.", "parola ".repeat(50).trim());
        let block = vec![sentence.as_str(); 5].join(" ");
        let split = paragraphs(&block);
        assert_eq!(split.as_array().unwrap().len(), 3, "{split}");
        assert!(split.as_array().unwrap().iter().all(|p| p["text"].as_str().unwrap().ends_with('.')));
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
        let detailed = std::env::var("GEMINI_SIMPLE").is_err();
        let language = std::env::var("GEMINI_LANGUAGE").ok();
        let answer = super::transcribe(path.as_ref(), &model, language.as_deref(), true, detailed, |p, f| eprintln!("{p} {f}")).unwrap();
        eprintln!("{:?}, timed: {}", started.elapsed(), answer.timed);
        println!("{}", serde_json::to_string_pretty(&answer.segments).unwrap());
    }
}
