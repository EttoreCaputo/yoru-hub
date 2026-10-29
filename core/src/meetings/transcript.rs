//! What comes of a recording: its transcript, made on this computer by
//! `yoru-audio` with Parakeet (docs/meetings.md), track by track so the
//! microphone is "you", and within each track who speaks when (Nemotron 3
//! Diarization), people the user can name; files brought in from
//! elsewhere; the transcript as Markdown, SRT or text; and the meetings'
//! settings (`meetings/settings.json`).

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Meeting, State, Track, checked, dir, folder, get, helper, message, new_folder, next_event, save};

/// When a recording is transcribed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum When {
    /// Once it stops (and once a file is imported).
    #[default]
    After,
    /// While it runs. Not built yet (phase M5): treated as `After`.
    Live,
    /// Only when asked, from the meeting.
    Manual,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub transcribe: When,
    /// `it`, `en`…, or empty: Parakeet finds the language.
    pub language: String,
    /// Tell who speaks within each track (the diarization model, about
    /// 190 MB, is fetched the first time).
    pub speakers: bool,
    /// What writes the summaries.
    pub summary_by: SummaryBy,
    /// Summarize a meeting once its transcript is made.
    pub auto_summary: bool,
}

/// The model that summarizes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryBy {
    /// The language model on this computer (llama.cpp).
    #[default]
    Local,
    /// The assistant's brain: Claude, OpenAI, Groq or Ollama.
    Brain,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { transcribe: When::After, language: String::new(), speakers: true, summary_by: SummaryBy::Local, auto_summary: true }
    }
}

fn settings_file() -> io::Result<PathBuf> {
    Ok(dir()?.join("settings.json"))
}

pub fn settings() -> Settings {
    settings_file().ok().and_then(|f| fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn set_settings(settings: &Settings) -> io::Result<()> {
    let file = settings_file()?;
    fs::create_dir_all(file.parent().unwrap())?;
    fs::write(file, serde_json::to_vec_pretty(settings)?)
}

/// A stretch of speech in the transcript.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Line {
    /// The track it was heard on, and its kind (`mic`: you, `others`).
    pub track: String,
    pub kind: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    /// Who speaks within the track, 0 for the first heard; none when the
    /// speakers weren't told apart.
    #[serde(default)]
    pub speaker: Option<u32>,
}

impl Line {
    /// The speaker's key in [`Meeting::speakers`]: `<track>/<speaker>`.
    pub fn who(&self) -> String {
        format!("{}/{}", self.track, self.speaker.map_or_else(|| "-".to_string(), |s| s.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    /// What made it: `parakeet-ultra`.
    pub engine: String,
    /// The language asked for, if one was.
    #[serde(default)]
    pub language: Option<String>,
    /// Every track's lines, in time order.
    pub lines: Vec<Line>,
    /// The people heard, in the order they first speak; filled when read.
    /// Always serialized: the app reads it (the copy in the file is ignored).
    #[serde(default)]
    pub speakers: Vec<Speaker>,
}

/// Someone heard in a meeting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Speaker {
    /// What [`Line::who`] gives for their lines.
    pub key: String,
    /// 0 is the user (the microphone's first voice); the others 1, 2… in
    /// the order they first speak, for "Person 1".
    pub number: u32,
    /// The name the user gave them.
    pub name: Option<String>,
}

/// The people heard in `transcript`, numbered as [`Speaker::number`] says.
fn speakers_of(meeting: &Meeting, transcript: &Transcript) -> Vec<Speaker> {
    let mut out: Vec<Speaker> = Vec::new();
    for line in &transcript.lines {
        let key = line.who();
        if out.iter().any(|s| s.key == key) {
            continue;
        }
        let you = line.kind == "mic" && line.speaker.unwrap_or(0) == 0;
        let number = if you { 0 } else { out.iter().filter(|s| s.number > 0).count() as u32 + 1 };
        out.push(Speaker { name: meeting.speakers.get(&key).cloned(), key, number });
    }
    out
}

/// What to call each speaker: their name, else `you` or `person` and a number.
pub(super) fn labels(speakers: &[Speaker], you: &str, person: &str) -> BTreeMap<String, String> {
    speakers
        .iter()
        .map(|s| (s.key.clone(), s.name.clone().unwrap_or_else(|| if s.number == 0 { you.to_string() } else { format!("{person} {}", s.number) })))
        .collect()
}

/// Call speaker `key` of meeting `id` `name` (empty: their number again).
pub fn rename_speaker(id: &str, key: &str, name: &str) -> io::Result<Transcript> {
    let mut meeting = get(id)?;
    match name.trim() {
        "" => meeting.speakers.remove(key),
        name => meeting.speakers.insert(key.to_string(), name.to_string()),
    };
    save(&folder(id)?, &meeting)?;
    transcript(id)?.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no transcript yet"))
}

const ENGINE: &str = "parakeet-ultra";

/// Where the model is kept: `models/parakeet-ultra` in the data folder, the
/// folder FluidAudio names after the repository whatever the path asked for.
fn model_dir() -> io::Result<PathBuf> {
    crate::stt::models_dir().map(|d| d.join("parakeet-ultra")).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data folder"))
}

/// Where FluidAudio keeps the diarization model: `models/nemotron-3-diarization`.
fn speakers_dir() -> io::Result<PathBuf> {
    crate::stt::models_dir().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data folder"))
}

/// Run `yoru-audio` with `args` until it is done: its `progress` events go
/// to `on_progress`, an `error` ends it, and `done` is returned.
fn run(args: &[&OsStr], mut on_progress: impl FnMut(&str, f64)) -> io::Result<Value> {
    let mut child = Command::new(helper()?).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let mut lines = BufReader::new(child.stdout.take().expect("piped"));
    let mut outcome = Err(io::Error::other("yoru-audio stopped"));
    while let Some(event) = next_event(&mut lines) {
        match event["event"].as_str() {
            Some("progress") => on_progress(event["phase"].as_str().unwrap_or(""), event["fraction"].as_f64().unwrap_or(0.0)),
            Some("error") => outcome = Err(io::Error::other(message(&event))),
            Some("done" | "model") => outcome = Ok(event),
            _ => {}
        }
    }
    let _ = child.wait();
    outcome
}

pub fn model_present() -> bool {
    let Ok(folder) = model_dir() else { return false };
    run(&["model".as_ref(), folder.as_os_str()], |_, _| {}).is_ok_and(|e| e["present"] == true)
}

/// Fetch the model (about 630 MB) if it isn't there.
pub fn download_model(mut on_progress: impl FnMut(f64)) -> io::Result<()> {
    let folder = model_dir()?;
    let done = run(&["model".as_ref(), folder.as_os_str(), "--download".as_ref()], |_, f| on_progress(f))?;
    if done["present"] == true { Ok(()) } else { Err(io::Error::other("the model didn't download")) }
}

/// Transcribe every track of meeting `id` and keep the transcript.
/// `on_progress` hears the track, the phase and how far it got.
/// With `speakers`, who speaks is told apart within each track.
pub fn transcribe(id: &str, language: Option<&str>, speakers: bool, mut on_progress: impl FnMut(&str, &str, f64)) -> io::Result<Meeting> {
    let mut meeting = get(id)?;
    if meeting.state == State::Recording {
        return Err(io::Error::new(io::ErrorKind::ResourceBusy, "still recording"));
    }
    let model = model_dir()?;
    let diarizer = speakers_dir()?;
    let folder = folder(id)?;
    let mut lines = Vec::new();
    for track in &meeting.tracks {
        let Some(file) = track.file.as_deref() else { continue };
        let path = folder.join(checked(file)?);
        let mut args: Vec<&OsStr> = vec!["transcribe".as_ref(), path.as_os_str(), "--model".as_ref(), model.as_os_str()];
        if let Some(language) = language {
            args.extend([OsStr::new("--language"), OsStr::new(language)]);
        }
        if speakers {
            args.extend([OsStr::new("--speakers"), diarizer.as_os_str()]);
        }
        on_progress(&track.name, "load", 0.0);
        let done = run(&args, |phase, f| on_progress(&track.name, phase, f))?;
        lines.extend(lines_of(track, &done["segments"]));
    }
    lines.sort_by_key(|l| l.start_ms);
    let transcript = Transcript { engine: ENGINE.into(), language: language.map(str::to_string), lines, speakers: Vec::new() };
    fs::write(folder.join("transcript.json"), serde_json::to_vec_pretty(&transcript)?)?;
    meeting.transcribed_at = Some(chrono::Local::now().timestamp_millis());
    save(&folder, &meeting)?;
    Ok(meeting)
}

fn lines_of(track: &Track, segments: &Value) -> Vec<Line> {
    segments
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let text = s["text"].as_str()?.trim();
            (!text.is_empty()).then(|| Line {
                track: track.name.clone(),
                kind: track.kind.clone(),
                start_ms: s["start_ms"].as_u64().unwrap_or(0),
                end_ms: s["end_ms"].as_u64().unwrap_or(0),
                text: text.to_string(),
                speaker: s["speaker"].as_u64().map(|n| n as u32),
            })
        })
        .collect()
}

/// The transcript of meeting `id`, if it has one.
pub fn transcript(id: &str) -> io::Result<Option<Transcript>> {
    let mut transcript: Transcript = match fs::read(folder(id)?.join("transcript.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    transcript.speakers = speakers_of(&get(id)?, &transcript);
    Ok(Some(transcript))
}

/// An audio or video file from elsewhere, as a new meeting of one track
/// named after the file.
pub fn import(path: &Path) -> io::Result<Meeting> {
    let now = chrono::Local::now();
    let (id, folder) = new_folder(now)?;
    let done = match run(&["import".as_ref(), path.as_os_str(), folder.as_os_str()], |_, _| {}) {
        Ok(done) => done,
        Err(e) => {
            let _ = fs::remove_dir_all(&folder);
            return Err(e);
        }
    };
    let tracks: Vec<Track> = serde_json::from_value(done["tracks"].clone()).unwrap_or_default();
    let meeting = Meeting {
        id,
        title: path.file_stem().map(|s| s.to_string_lossy().into_owned()),
        started_at: now.timestamp_millis(),
        duration_ms: tracks.iter().map(|t| t.duration_ms).max().unwrap_or(0),
        state: State::Done,
        tracks,
        transcribed_at: None,
        summarized_at: None,
        speakers: BTreeMap::new(),
    };
    save(&folder, &meeting)?;
    Ok(meeting)
}

/// How the transcript is written out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Markdown,
    Srt,
    Text,
}

/// Write meeting `id`'s transcript to Downloads. The speakers are called by
/// their names, else `you` or `person` and their number. Returns the
/// file's path.
pub fn export(id: &str, format: Format, you: &str, person: &str) -> io::Result<PathBuf> {
    let meeting = get(id)?;
    let transcript = transcript(id)?.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no transcript yet"))?;
    let title = meeting.title.clone().unwrap_or_else(|| format!("Meeting {}", meeting.id));
    let body = render(&transcript, format, &title, &labels(&transcript.speakers, you, person));
    let dest = crate::paths::home_dir().map(|h| h.join("Downloads")).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no home folder"))?;
    let ext = match format {
        Format::Markdown => "md",
        Format::Srt => "srt",
        Format::Text => "txt",
    };
    let stem: String = title.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
    let mut path = dest.join(format!("{}.{ext}", stem.trim()));
    for n in 1.. {
        if !path.exists() {
            break;
        }
        path = dest.join(format!("{} {n}.{ext}", stem.trim()));
    }
    fs::write(&path, body)?;
    Ok(path)
}

fn render(transcript: &Transcript, format: Format, title: &str, labels: &BTreeMap<String, String>) -> String {
    let who = |l: &Line| labels.get(&l.who()).map_or("?", String::as_str);
    let mut out = String::new();
    match format {
        Format::Markdown => {
            out.push_str(&format!("# {title}\n\n"));
            for l in &transcript.lines {
                out.push_str(&format!("**{}** ({}): {}\n\n", who(l), clock(l.start_ms), l.text));
            }
        }
        Format::Text => {
            for l in &transcript.lines {
                out.push_str(&format!("[{}] {}: {}\n", clock(l.start_ms), who(l), l.text));
            }
        }
        Format::Srt => {
            for (i, l) in transcript.lines.iter().enumerate() {
                out.push_str(&format!("{}\n{} --> {}\n{}: {}\n\n", i + 1, srt_time(l.start_ms), srt_time(l.end_ms.max(l.start_ms + 500)), who(l), l.text));
            }
        }
    }
    out
}

/// `4:05` or `1:02:30`.
fn clock(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// `01:02:03,456`.
fn srt_time(ms: u64) -> String {
    format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(kind: &str, speaker: Option<u32>, start_ms: u64, text: &str) -> Line {
        Line { track: kind.into(), kind: kind.into(), start_ms, end_ms: start_ms + 2000, text: text.into(), speaker }
    }

    #[test]
    fn sends_the_speakers() {
        let transcript = Transcript { engine: "parakeet-ultra".into(), language: None, lines: vec![], speakers: vec![] };
        assert!(serde_json::to_value(&transcript).unwrap().get("speakers").is_some());
    }

    fn meeting() -> Meeting {
        Meeting { id: "m".into(), title: None, started_at: 0, duration_ms: 0, state: State::Done, tracks: vec![], transcribed_at: None, summarized_at: None, speakers: BTreeMap::new() }
    }

    #[test]
    fn writes_each_format() {
        let mut t = Transcript { engine: ENGINE.into(), language: None, lines: vec![line("mic", Some(0), 0, "Ciao."), line("others", Some(0), 3_725_000, "Ciao a te.")], speakers: vec![] };
        t.speakers = speakers_of(&meeting(), &t);
        let labels = labels(&t.speakers, "You", "Person");
        assert_eq!(render(&t, Format::Text, "T", &labels), "[0:00] You: Ciao.\n[1:02:05] Person 1: Ciao a te.\n");
        assert!(render(&t, Format::Markdown, "Weekly", &labels).starts_with("# Weekly\n\n**You** (0:00): Ciao.\n"));
        assert!(render(&t, Format::Srt, "T", &labels).contains("2\n01:02:05,000 --> 01:02:07,000\nPerson 1: Ciao a te.\n"));
    }

    #[test]
    fn numbers_people_as_they_first_speak_and_keeps_names() {
        let t = Transcript {
            engine: ENGINE.into(),
            language: None,
            lines: vec![line("others", Some(1), 0, "a"), line("mic", None, 1000, "b"), line("others", Some(0), 2000, "c"), line("mic", Some(1), 3000, "d")],
            speakers: vec![],
        };
        let mut m = meeting();
        m.speakers.insert("others/0".into(), "Marco".into());
        let found: Vec<_> = speakers_of(&m, &t).into_iter().map(|s| (s.key, s.number, s.name)).collect();
        assert_eq!(
            found,
            [("others/1".into(), 1, None), ("mic/-".into(), 0, None), ("others/0".into(), 2, Some("Marco".into())), ("mic/1".into(), 3, None)]
        );
    }

    #[test]
    fn reads_the_helpers_segments() {
        let track = Track { name: "mic".into(), source: "Mic".into(), kind: "mic".into(), file: None, duration_ms: 0, rate: 0, channels: 0 };
        let segments = serde_json::json!([{"text": " Buongiorno. ", "start_ms": 120, "end_ms": 900}, {"text": "", "start_ms": 1000, "end_ms": 1100}]);
        let lines = lines_of(&track, &segments);
        assert_eq!(lines.len(), 1);
        assert_eq!((lines[0].text.as_str(), lines[0].start_ms, lines[0].kind.as_str()), ("Buongiorno.", 120, "mic"));
    }

    #[test]
    fn settings_default_to_after_the_meeting() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.transcribe, When::After);
        assert!(s.language.is_empty());
        assert!(s.speakers);
    }
}
