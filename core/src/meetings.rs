//! Meetings: what the computer plays and what its microphones hear,
//! recorded as tracks (docs/meetings.md). The recording itself is done by
//! `yoru-audio`, the Swift helper next to the app (`audio/`), which speaks
//! JSON lines on stdout; this side keeps the meetings in `meetings/` in the
//! data folder, one folder each with `meeting.json` and a file per track.
//! What comes of a recording (its transcript, imported files, exports, the
//! settings) is in [`transcript`].

mod summary;
mod transcript;

pub use summary::{ActionItem, Complete, Summary, summarize, summary};

pub use transcript::{Format, Line, Settings, Speaker, SummaryBy, Transcript, When, download_model, export, import, model_present, rename_speaker, set_settings, settings, transcribe, transcript};

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A recording and what came of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meeting {
    /// When it started (`20261029-153012`): its folder's name, and its order.
    pub id: String,
    /// Set by the user; the app shows the date without one.
    #[serde(default)]
    pub title: Option<String>,
    /// Unix milliseconds.
    pub started_at: i64,
    pub duration_ms: u64,
    pub state: State,
    pub tracks: Vec<Track>,
    /// When its transcript was made (Unix milliseconds), if it has one.
    #[serde(default)]
    pub transcribed_at: Option<i64>,
    /// When its summary was made (Unix milliseconds), if it has one.
    #[serde(default)]
    pub summarized_at: Option<i64>,
    /// The names the user gave the people heard, by [`Line::who`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub speakers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Recording,
    Done,
    /// Cut short (the app quit or crashed): what was recorded is kept.
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    /// `others` (the tap) or `mic`, `mic-2`…
    pub name: String,
    /// What it recorded: "System audio", an app's name, a microphone's.
    pub source: String,
    /// `others` or `mic`: the microphone is the user.
    pub kind: String,
    /// The file in the meeting's folder, once there is one.
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub rate: u32,
    #[serde(default)]
    pub channels: u32,
}

/// What can be recorded right now.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Sources {
    pub apps: Vec<AudioApp>,
    pub microphones: Vec<Microphone>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioApp {
    /// Its bundle id.
    pub id: String,
    pub name: String,
    /// Playing sound right now.
    pub playing: bool,
    /// Its icon, a 128-pixel PNG in base64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Microphone {
    /// Core Audio's device UID.
    pub id: String,
    pub name: String,
    pub default: bool,
}

/// What to record.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Everything the computer plays.
    #[serde(default)]
    pub system: bool,
    /// Or one app's sound, by bundle id.
    #[serde(default)]
    pub app: Option<String>,
    /// Microphones by UID; `default` is the system's.
    #[serde(default)]
    pub microphones: Vec<String>,
    #[serde(default)]
    pub title: Option<String>,
}

/// What a recording reports while it runs.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// Each track's loudest sample in the last quarter second, 0 to 1.
    Level { levels: BTreeMap<String, f32> },
    /// Something went wrong; the recording goes on if it can. `code` says
    /// what, for the app to explain: `no_system_audio` (macOS gives Yoru no
    /// sound from the computer: the permission) or `no_microphone_sound`.
    Error {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
    /// Finished: the meeting as kept.
    Done { meeting: Meeting },
    /// Transcribing meeting `id`, track `track`: `phase` is `load` or
    /// `transcribe`, `fraction` 0 to 1.
    Progress { id: String, track: String, phase: String, fraction: f64 },
    /// The transcript is ready.
    Transcribed { meeting: Meeting },
    /// Transcribing meeting `id` failed.
    Failed { id: String, message: String },
    /// The transcription model is downloading, 0 to 1.
    Model { fraction: f64 },
    /// Summarizing meeting `id`, 0 to 1.
    Summarizing { id: String, fraction: f64 },
    /// The summary is ready.
    Summarized { meeting: Meeting, summary: Summary },
    /// Summarizing meeting `id` failed.
    SummaryFailed { id: String, message: String },
}

fn helper() -> io::Result<PathBuf> {
    crate::sidecar::find_program("yoru-audio", "YORU_AUDIO")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "yoru-audio isn't installed with the app"))
}

fn dir() -> io::Result<PathBuf> {
    crate::paths::data_dir().map(|d| d.join("meetings")).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data folder"))
}

/// The apps that use audio and the microphones.
pub fn sources() -> io::Result<Sources> {
    let out = Command::new(helper()?).arg("sources").stderr(Stdio::null()).output()?;
    let value: Value = serde_json::from_slice(&out.stdout).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if !out.status.success() {
        return Err(io::Error::other(message(&value)));
    }
    serde_json::from_value(value).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn message(event: &Value) -> String {
    event["message"].as_str().unwrap_or("yoru-audio failed").to_string()
}

/// A recording running in `yoru-audio`.
pub struct Recording {
    meeting: Arc<Mutex<Meeting>>,
    stdin: Mutex<Option<ChildStdin>>,
    child: Arc<Mutex<Child>>,
}

impl Recording {
    /// Start recording what `request` asks for. Returns once every track
    /// runs, which may wait for the user to answer the microphone's
    /// permission prompt. `on_event` hears the levels, the errors, and the
    /// end, on a thread of the recording's own.
    pub fn start(request: &Request, on_event: impl Fn(Event) + Send + 'static) -> io::Result<Recording> {
        let now = chrono::Local::now();
        let (id, folder) = new_folder(now)?;

        let mut command = Command::new(helper()?);
        command.arg("record").arg(&folder);
        if request.system {
            command.arg("--system");
        } else if let Some(app) = &request.app {
            command.args(["--app", app]);
        }
        for mic in &request.microphones {
            command.args(["--mic", mic]);
        }
        let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
        let stdin = child.stdin.take();
        let mut lines = BufReader::new(child.stdout.take().expect("piped"));

        let started = match next_event(&mut lines) {
            Some(e) if e["event"] == "started" => e,
            other => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = fs::remove_dir_all(&folder);
                return Err(io::Error::other(other.map_or_else(|| "yoru-audio stopped".to_string(), |e| message(&e))));
            }
        };
        let tracks: Vec<Track> = serde_json::from_value(started["tracks"].clone()).unwrap_or_default();
        let meeting = Meeting {
            id,
            title: request.title.clone().filter(|t| !t.trim().is_empty()),
            started_at: now.timestamp_millis(),
            duration_ms: 0,
            state: State::Recording,
            tracks,
            transcribed_at: None,
            summarized_at: None,
            speakers: BTreeMap::new(),
        };
        save(&folder, &meeting)?;
        let meeting = Arc::new(Mutex::new(meeting));
        let child = Arc::new(Mutex::new(child));
        let (shared, process) = (meeting.clone(), child.clone());
        thread::Builder::new()
            .name("yoru-meeting".into())
            .spawn(move || follow(lines, &folder, &shared, &process, on_event))?;
        Ok(Recording { meeting, stdin: Mutex::new(stdin), child })
    }

    /// The meeting as it was when the recording started.
    pub fn meeting(&self) -> Meeting {
        self.meeting.lock().unwrap().clone()
    }

    /// Ask the helper to stop; [`Event::Done`] follows once the tracks are
    /// finalized.
    pub fn stop(&self) {
        if let Some(mut stdin) = self.stdin.lock().unwrap().take() {
            let _ = stdin.write_all(b"stop\n");
        }
    }

    /// Stop and wait up to `timeout` for the tracks to be finalized: the app
    /// is quitting.
    pub fn stop_and_wait(&self, timeout: Duration) {
        self.stop();
        let until = Instant::now() + timeout;
        while Instant::now() < until {
            if !matches!(self.child.lock().unwrap().try_wait(), Ok(None)) {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
}

/// A new meeting's folder, named after `now`.
fn new_folder(now: chrono::DateTime<chrono::Local>) -> io::Result<(String, PathBuf)> {
    let base = now.format("%Y%m%d-%H%M%S").to_string();
    let root = dir()?;
    let mut id = base.clone();
    for n in 1.. {
        if !root.join(&id).exists() {
            break;
        }
        id = format!("{base}-{n}");
    }
    let folder = root.join(&id);
    fs::create_dir_all(&folder)?;
    Ok((id, folder))
}

fn next_event(lines: &mut BufReader<ChildStdout>) -> Option<Value> {
    let mut line = String::new();
    loop {
        line.clear();
        if lines.read_line(&mut line).ok()? == 0 {
            return None;
        }
        if let Ok(value) = serde_json::from_str::<Value>(&line) {
            return Some(value);
        }
    }
}

/// The helper's events until it ends, then the meeting as kept.
fn follow(mut lines: BufReader<ChildStdout>, folder: &Path, meeting: &Mutex<Meeting>, child: &Mutex<Child>, on_event: impl Fn(Event)) {
    let mut finished = None;
    while let Some(event) = next_event(&mut lines) {
        match event["event"].as_str() {
            Some("level") => {
                let levels = serde_json::from_value(event["levels"].clone()).unwrap_or_default();
                on_event(Event::Level { levels });
            }
            Some("error") => on_event(Event::Error { message: message(&event), code: event["code"].as_str().map(str::to_string) }),
            Some("done") => finished = serde_json::from_value::<Vec<Track>>(event["tracks"].clone()).ok(),
            _ => {}
        }
    }
    let _ = child.lock().unwrap().wait();
    let mut meeting = meeting.lock().unwrap().clone();
    match finished {
        Some(tracks) => {
            meeting.tracks = tracks;
            meeting.state = State::Done;
        }
        // The helper died: finalize what it left.
        None => {
            finalize(folder, &mut meeting);
            on_event(Event::Error { message: "the recording stopped on its own".into(), code: None });
        }
    }
    meeting.duration_ms = meeting.tracks.iter().map(|t| t.duration_ms).max().unwrap_or(0);
    if let Err(e) = save(folder, &meeting) {
        on_event(Event::Error { message: format!("saving the meeting: {e}"), code: None });
    }
    on_event(Event::Done { meeting });
}

/// Turn a cut-short recording's tracks into files and mark it interrupted.
fn finalize(folder: &Path, meeting: &mut Meeting) {
    if let Ok(helper) = helper() {
        let _ = Command::new(helper).arg("finalize").arg(folder).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    for track in &mut meeting.tracks {
        track.file = ["m4a", "caf"].iter().map(|ext| format!("{}.{ext}", track.name)).find(|f| folder.join(f).exists());
    }
    meeting.state = State::Interrupted;
}

/// Meetings left "recording" by an app that quit or crashed: finalized and
/// marked interrupted. Call at start, before any recording.
pub fn recover() {
    let Ok(root) = dir() else { return };
    for mut meeting in list_in(&root).into_iter().filter(|m| m.state == State::Recording) {
        let folder = root.join(&meeting.id);
        finalize(&folder, &mut meeting);
        let _ = save(&folder, &meeting);
    }
}

fn save(folder: &Path, meeting: &Meeting) -> io::Result<()> {
    // Written aside and renamed: a crash never leaves half a file.
    let part = folder.join("meeting.json.part");
    fs::write(&part, serde_json::to_vec_pretty(meeting)?)?;
    fs::rename(part, folder.join("meeting.json"))
}

/// Every meeting, newest first.
pub fn list() -> Vec<Meeting> {
    dir().map(|d| list_in(&d)).unwrap_or_default()
}

fn list_in(root: &Path) -> Vec<Meeting> {
    let Ok(entries) = fs::read_dir(root) else { return Vec::new() };
    let mut meetings: Vec<Meeting> = entries
        .flatten()
        .filter_map(|e| serde_json::from_slice(&fs::read(e.path().join("meeting.json")).ok()?).ok())
        .collect();
    meetings.sort_by(|a, b| b.id.cmp(&a.id));
    meetings
}

fn folder(id: &str) -> io::Result<PathBuf> {
    Ok(dir()?.join(checked(id)?))
}

/// Only what [`Recording::start`] makes: no `..`, no separators.
fn checked(name: &str) -> io::Result<&str> {
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.') && !name.starts_with('.') {
        Ok(name)
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidInput, "bad name"))
    }
}

pub fn get(id: &str) -> io::Result<Meeting> {
    serde_json::from_slice(&fs::read(folder(id)?.join("meeting.json"))?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn rename(id: &str, title: &str) -> io::Result<Meeting> {
    let mut meeting = get(id)?;
    meeting.title = Some(title.trim().to_string()).filter(|t| !t.is_empty());
    save(&folder(id)?, &meeting)?;
    Ok(meeting)
}

pub fn delete(id: &str) -> io::Result<()> {
    if get(id)?.state == State::Recording {
        return Err(io::Error::new(io::ErrorKind::ResourceBusy, "still recording"));
    }
    fs::remove_dir_all(folder(id)?)
}

/// The file of track `name` of meeting `id`.
pub fn audio(id: &str, name: &str) -> io::Result<Vec<u8>> {
    let meeting = get(id)?;
    let track = meeting.tracks.iter().find(|t| t.name == name).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such track"))?;
    let file = track.file.as_deref().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "the track has no file yet"))?;
    fs::read(folder(id)?.join(checked(file)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meeting(id: &str, state: State) -> Meeting {
        Meeting {
            id: id.into(),
            title: None,
            started_at: 0,
            duration_ms: 0,
            state,
            transcribed_at: None,
            summarized_at: None,
            speakers: BTreeMap::new(),
            tracks: vec![Track { name: "mic".into(), source: "Mic".into(), kind: "mic".into(), file: None, duration_ms: 0, rate: 48000, channels: 1 }],
        }
    }

    #[test]
    fn lists_newest_first_and_skips_folders_without_a_meeting() {
        let root = tempfile::tempdir().unwrap();
        for id in ["20261029-100000", "20261029-120000"] {
            fs::create_dir_all(root.path().join(id)).unwrap();
            save(&root.path().join(id), &meeting(id, State::Done)).unwrap();
        }
        fs::create_dir_all(root.path().join("stray")).unwrap();
        let ids: Vec<_> = list_in(root.path()).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["20261029-120000", "20261029-100000"]);
    }

    #[test]
    fn a_cut_short_meeting_keeps_the_tracks_it_left() {
        let root = tempfile::tempdir().unwrap();
        let mut m = meeting("20261029-100000", State::Recording);
        fs::write(root.path().join("mic.caf"), b"x").unwrap();
        finalize(root.path(), &mut m);
        assert_eq!(m.state, State::Interrupted);
        assert!(m.tracks[0].file.as_deref().is_some_and(|f| f.starts_with("mic.")));
    }

    #[test]
    fn names_from_outside_stay_in_the_folder() {
        assert!(checked("20261029-100000").is_ok());
        assert!(checked("mic.m4a").is_ok());
        for bad in ["", "..", "../x", "a/b", ".hidden"] {
            assert!(checked(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn reads_the_helpers_tracks() {
        let tracks: Vec<Track> = serde_json::from_str(
            r#"[{"name":"others","source":"System audio","kind":"others","rate":48000,"channels":2,"file":"others.m4a","duration_ms":1200}]"#,
        )
        .unwrap();
        assert_eq!(tracks[0].file.as_deref(), Some("others.m4a"));
        assert_eq!(tracks[0].duration_ms, 1200);
    }
}
