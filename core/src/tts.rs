//! Text-to-speech with a sidecar on `127.0.0.1`: a
//! [Piper](https://github.com/OHF-Voice/piper1-gpl) `http_server`.
//!
//! Each engine is a Python package (`piper-tts`, GPL-3.0), run as a
//! separate process and installed into a virtualenv of its own in
//! the data folder with [`install`]. [`Tts`] starts the servers
//! its voices need in the background, restarts one that dies and stops them
//! when dropped ([`crate::sidecar`]). One server speaks every voice of its
//! engine: each request names the one for its language. Voices come from
//! [`crate::models`]; Piper's 22 kHz is resampled to the board's 16 kHz.
//!
//! A voice can also be a provider's ([`crate::cloud`], `groq:hannah`,
//! `elevenlabs:voice/<voice_id>`): no server here, just a request per sentence (split
//! further where the provider takes less text at once). When it fails for
//! any reason (its limits, the month's credits, the network, the key), a
//! downloaded voice of the language speaks instead, if there's one, and the
//! cloud voice is left alone for a minute ([`CLOUD_RETRY`]).
//!
//! An ElevenLabs voice acts out [`AUDIO_TAGS`] (`[laughs]`, `[whispers]`):
//! [`speakable`] keeps them, and every other voice, a fallback too, gets
//! the text without them.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File};
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::json;

pub use crate::models::Engine;
use crate::cloud::{self, CloudError};
use crate::flasher::{self, FlashEvent};
use crate::models::{self, Kind, Model};
use crate::sidecar::{self, Process};
use crate::stt::models_dir;
use crate::voice::SAMPLE_RATE;

/// What [`install`] puts in Piper's virtualenv.
pub const PIPER_REQUIREMENT: &str = "piper-tts[http]==1.8.0";
/// Written into the virtualenv once the engine is installed in it.
const READY_MARKER: &str = ".companion-ready";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a cloud voice that failed is left to its fallback before it's
/// asked again (one over its limits says how long itself).
pub const CLOUD_RETRY: Duration = Duration::from_secs(60);

/// The audio tags the assistant may write when its voice acts them out
/// ([`Tts::reads_audio_tags`]): a short list from ElevenLabs' v4 guide that
/// suits Yoru, so a voice that can't act them out is sure to lose them all.
pub const AUDIO_TAGS: &[&str] = &["laughs", "sighs", "exhales", "whispers", "curious", "excited", "happy", "sad", "sarcastic", "mischievously"];

impl Engine {
    pub const ALL: [Engine; 1] = [Engine::Piper];

    pub fn name(self) -> &'static str {
        match self {
            Engine::Piper => "Piper",
        }
    }

    /// What it's called in the data folder, commands and errors.
    pub fn key(self) -> &'static str {
        match self {
            Engine::Piper => "piper",
        }
    }

    pub fn requirement(self) -> &'static str {
        match self {
            Engine::Piper => PIPER_REQUIREMENT,
        }
    }

    /// The Python versions it installs into, as (major, minor) bounds.
    fn pythons(self) -> std::ops::RangeInclusive<u32> {
        match self {
            Engine::Piper => 9..=99,
        }
    }

    /// Where its virtualenv lives: `piper/` in the data folder.
    pub fn dir(self) -> Option<PathBuf> {
        crate::paths::data_dir().map(|d| d.join(self.key()))
    }

    /// The virtualenv's interpreter: `COMPANION_PIPER_PYTHON`, or the one
    /// [`install`] set up.
    pub fn python(self) -> Option<PathBuf> {
        if let Some(p) = std::env::var_os(format!("COMPANION_{}_PYTHON", self.key().to_uppercase())).map(PathBuf::from) {
            return p.is_file().then_some(p);
        }
        let venv = self.dir()?.join("venv");
        venv.join(READY_MARKER).is_file().then(|| flasher::venv_python(&venv)).filter(|p| p.is_file())
    }
}

/// Piper's interpreter ([`Engine::python`]).
pub fn find_python() -> Option<PathBuf> {
    Engine::Piper.python()
}

/// Create `engine`'s virtualenv with `python` and install its
/// [`Engine::requirement`] into it (about 60 MB for Piper, from PyPI), passing on pip's output line by line. Returns the
/// virtualenv's interpreter.
pub fn install(engine: Engine, python: &Path, mut on_line: impl FnMut(String)) -> io::Result<PathBuf> {
    let dir = engine.dir().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data folder"))?;
    let venv = dir.join("venv");
    fs::create_dir_all(&dir)?;
    let _ = fs::remove_file(venv.join(READY_MARKER));
    let mut forward = |e: FlashEvent| {
        if let FlashEvent::Log { line } = e {
            on_line(line);
        }
    };
    let mut cmd = flasher::command(python);
    cmd.arg("-m").arg("venv").arg("--clear").arg(&venv);
    flasher::run_streaming(cmd, &mut forward).map_err(|e| io::Error::new(e.kind(), format!("creating the virtualenv: {e}")))?;
    let venv_python = flasher::venv_python(&venv);
    let mut cmd = flasher::command(&venv_python);
    cmd.args(["-m", "pip", "install", "--disable-pip-version-check", "--no-input", "--retries", "10", engine.requirement()]);
    flasher::run_streaming(cmd, &mut forward).map_err(|e| io::Error::new(e.kind(), format!("installing {}: {e}", engine.name())))?;
    File::create(venv.join(READY_MARKER))?;
    Ok(venv_python)
}

/// A Python 3 on the PATH that `engine` installs into, to create its
/// virtualenv with (not macOS's `/usr/bin/python3` stub, which offers to
/// install the Xcode tools): `python3`, else `python3.13` down to `3.9`.
pub fn system_python(engine: Engine) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let exe = if cfg!(windows) { ".exe" } else { "" };
    let mut names: Vec<String> = if cfg!(windows) { vec!["python.exe".into(), "python3.exe".into()] } else { vec!["python3".into()] };
    names.extend((9..=13).rev().map(|minor| format!("python3.{minor}{exe}")));
    let dirs: Vec<PathBuf> = std::env::split_paths(&path)
        .chain(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from))
        .filter(|dir| !(cfg!(target_os = "macos") && dir == Path::new("/usr/bin")))
        .collect();
    names.iter().flat_map(|n| dirs.iter().map(move |d| d.join(n))).find(|p| p.is_file() && python_minor(p).is_some_and(|m| engine.pythons().contains(&m)))
}

/// The minor version of a Python 3.
fn python_minor(python: &Path) -> Option<u32> {
    let out = flasher::command(python).args(["-c", "import sys; print(sys.version_info[0], sys.version_info[1])"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut parts = text.split_whitespace();
    (parts.next()? == "3").then_some(())?;
    parts.next()?.parse().ok()
}

#[derive(Debug, Clone, PartialEq)]
pub struct TtsConfig {
    /// Each engine's virtualenv interpreter: those of `voices` must be here.
    pub pythons: BTreeMap<Engine, PathBuf>,
    /// Voice by language (`it`, `en`): ids from [`crate::models`],
    /// downloaded, or of [`crate::cloud`] models.
    pub voices: BTreeMap<String, String>,
    /// The cloud voices among `voices`, by id, with their provider's key.
    pub clients: BTreeMap<String, cloud::Client>,
    /// For each language with a cloud voice, where there's one: the
    /// downloaded voice that speaks when it fails.
    pub fallbacks: BTreeMap<String, String>,
}

impl TtsConfig {
    /// The default voice of each language, where it's downloaded, with the
    /// engines that are installed.
    pub fn with_default_voices() -> Self {
        let voices = ["it", "en"]
            .into_iter()
            .filter_map(|lang| models::default_voice(lang).map(|v| (lang.to_string(), v.to_string())))
            .filter(|(_, v)| Model::by_id(v).is_some_and(Model::is_installed))
            .collect();
        Self { pythons: installed_engines(), voices, clients: BTreeMap::new(), fallbacks: BTreeMap::new() }
    }

    /// `voices`, local or cloud, with the engines in `pythons`. Fails on a
    /// cloud voice whose provider has no key.
    pub fn new(pythons: BTreeMap<Engine, PathBuf>, voices: BTreeMap<String, String>) -> Result<Self, String> {
        let clients = voices
            .values()
            .filter(|v| cloud::is_cloud_id(v))
            .map(|v| cloud::Client::new(v, Kind::Voice).map(|c| (v.clone(), c)))
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let fallbacks = voices
            .iter()
            .filter(|(_, v)| clients.contains_key(*v))
            .filter_map(|(lang, _)| Some((lang.clone(), local_voice(&pythons, lang)?.to_string())))
            .collect();
        Ok(Self { pythons, voices, clients, fallbacks })
    }


    /// The engines its voices need.
    pub fn engines(&self) -> impl Iterator<Item = Engine> + '_ {
        Engine::ALL.into_iter().filter(|e| self.voices.values().any(|v| Model::by_id(v).and_then(Model::engine) == Some(*e)))
    }
}

/// A downloaded voice of `language` that an engine in `pythons` speaks: the
/// default one if it can, else any.
fn local_voice(pythons: &BTreeMap<Engine, PathBuf>, language: &str) -> Option<&'static str> {
    let usable = |m: &&Model| m.language == Some(language) && m.is_installed() && m.engine().is_some_and(|e| pythons.contains_key(&e));
    let voices = || Model::of_kind(Kind::Voice).filter(usable);
    voices().find(|m| Some(m.id) == models::default_voice(language)).or_else(|| voices().next()).map(|m| m.id)
}

/// The interpreter of every engine that's installed.
pub fn installed_engines() -> BTreeMap<Engine, PathBuf> {
    Engine::ALL.into_iter().filter_map(|e| e.python().map(|p| (e, p))).collect()
}

/// Says which engine failed: `piper: ...`.
#[derive(Debug)]
pub struct TtsError(String);

impl TtsError {
    fn new(engine: Engine, why: impl fmt::Display) -> Self {
        TtsError(format!("{}: {why}", engine.key()))
    }

    fn cloud(e: CloudError) -> Self {
        TtsError(e.to_string())
    }
}

impl fmt::Display for TtsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TtsError {}

/// The sidecars. Shared between connections: they outlive a board reconnect.
pub struct Tts {
    config: TtsConfig,
    servers: Mutex<BTreeMap<Engine, Process>>,
    /// The cloud voices that failed and are left to their fallbacks: why,
    /// and until when.
    failed: Mutex<BTreeMap<String, (String, Instant)>>,
    /// Keep the servers running with every downloaded voice loaded, cloud
    /// voices' fallbacks too.
    warm: AtomicBool,
}

impl fmt::Debug for Tts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tts").field("config", &self.config).finish_non_exhaustive()
    }
}

impl Tts {
    pub fn new(config: TtsConfig) -> Self {
        Self { config, servers: Mutex::new(BTreeMap::new()), failed: Mutex::new(BTreeMap::new()), warm: AtomicBool::new(false) }
    }

    /// Keep the downloaded voices loaded: the fallbacks of cloud voices
    /// too by [`Tts::warm_up`], and again by [`Tts::rewarm`] if their server died.
    pub fn kept_warm(self, on: bool) -> Self {
        self.warm.store(on, Ordering::Relaxed);
        self
    }

    pub fn is_kept_warm(&self) -> bool {
        self.warm.load(Ordering::Relaxed)
    }

    pub fn config(&self) -> &TtsConfig {
        &self.config
    }

    /// Stop the servers now, even while others still hold this [`Tts`] (a
    /// reply being spoken); the next request would start them again (but
    /// not [`keep_warm`](crate::keep_warm)).
    pub fn shutdown(&self) {
        self.warm.store(false, Ordering::Relaxed);
        self.servers.lock().unwrap().clear();
    }

    /// The voice `language` speaks with: its own, or else any there is.
    pub fn voice_for(&self, language: &str) -> Option<&str> {
        self.config.voices.get(language).or_else(|| self.config.voices.values().next()).map(String::as_str)
    }

    /// Start the servers and load every voice (Piper loads one on its first
    /// request, which takes most of a second). A cloud voice's key is only
    /// checked: each sentence spoken counts against a free plan.
    pub fn warm_up(&self) -> Result<(), TtsError> {
        for (language, voice) in &self.config.voices {
            match self.config.clients.get(voice) {
                Some(client) => {
                    if let Err(e) = client.check() {
                        // Not working now: have its fallback ready instead, if it has one.
                        let fallback = self.config.fallbacks.get(language).ok_or_else(|| TtsError::cloud(e.clone()))?;
                        self.cloud_failed(voice, &e);
                        drop(self.request("Ok.", fallback)?);
                    } else if let Some(fallback) = self.config.fallbacks.get(language).filter(|_| self.is_kept_warm()) {
                        // Ready for when the cloud voice fails; if it can't be, the cloud voice still speaks.
                        let _ = self.request("Ok.", fallback);
                    }
                }
                None => drop(self.request("Ok.", voice)?),
            }
        }
        Ok(())
    }

    /// When [`Tts::kept_warm`], start a server that isn't running and load
    /// its voices again.
    pub fn rewarm(&self) -> Result<(), TtsError> {
        if !self.is_kept_warm() {
            return Ok(());
        }
        let local: Vec<&String> = self.config.voices.values().filter(|v| !self.config.clients.contains_key(*v)).chain(self.config.fallbacks.values()).collect();
        let engine = |v: &str| Model::by_id(v).and_then(Model::engine);
        let stopped: Vec<Engine> = {
            let mut servers = self.servers.lock().unwrap();
            local.iter().filter_map(|v| engine(v)).filter(|e| !servers.get_mut(e).is_some_and(|s| s.is_running())).collect()
        };
        // The first request starts the server, each one loads its voice.
        for voice in local.into_iter().filter(|v| engine(v).is_some_and(|e| stopped.contains(&e))) {
            drop(self.request("Ok.", voice)?);
        }
        Ok(())
    }

    /// Whether `language`'s voice acts out [`AUDIO_TAGS`]. Its fallback
    /// doesn't, but it gets the text without them.
    pub fn reads_audio_tags(&self, language: &str) -> bool {
        self.voice_for(language).and_then(|v| self.config.clients.get(v)).is_some_and(cloud::Client::reads_audio_tags)
    }

    /// Why a cloud voice is being stood in for by a downloaded one, while it is.
    pub fn cloud_trouble(&self) -> Option<String> {
        let mut failed = self.failed.lock().unwrap();
        failed.retain(|_, (_, until)| *until > Instant::now());
        failed.values().next().map(|(why, _)| why.clone())
    }

    fn cloud_failed(&self, voice: &str, e: &CloudError) {
        // One over its limits fails at once until it may ask again (crate::cloud): a minute here is enough.
        self.failed.lock().unwrap().insert(voice.to_string(), (e.to_string(), Instant::now() + CLOUD_RETRY));
    }

    /// Whether a cloud voice is left to its fallback for now.
    fn resting(&self, voice: &str) -> bool {
        self.failed.lock().unwrap().get(voice).is_some_and(|(_, until)| *until > Instant::now())
    }

    /// Speak `text` in `language`: 16 kHz mono samples.
    pub fn synthesize(&self, text: &str, language: &str) -> Result<Vec<i16>, TtsError> {
        let mut all = Vec::new();
        self.synthesize_into(text, language, &mut |samples| all.extend(samples))?;
        Ok(all)
    }

    /// [`Tts::synthesize`], the samples to `out` as they come: a voice that
    /// streams (ElevenLabs) is heard before it has said the whole of `text`.
    /// Once some sound went out, a cloud voice that fails isn't stood in for:
    /// the downloaded voice would start the sentence over.
    pub fn synthesize_into(&self, text: &str, language: &str, out: &mut dyn FnMut(Vec<i16>)) -> Result<(), TtsError> {
        // Piper fails on text it makes no sound for.
        if !text.chars().any(char::is_alphanumeric) {
            return Ok(());
        }
        let plain = without_audio_tags(text);
        let mut voice = self.voice_for(language).ok_or_else(|| TtsError::new(Engine::Piper, "no voice downloaded"))?.to_string();
        if let Some(client) = self.config.clients.get(&voice) {
            let fallback = self.config.fallbacks.get(language);
            match fallback {
                Some(f) if self.resting(&voice) => voice = f.clone(),
                _ => {
                    let said = if client.reads_audio_tags() { text } else { plain.as_str() };
                    if !said.chars().any(char::is_alphanumeric) {
                        return Ok(());
                    }
                    let mut heard = false;
                    match self.speak_cloud(client, said, language, &mut |samples| {
                        heard |= !samples.is_empty();
                        out(samples);
                    }) {
                        Ok(()) => {
                            self.failed.lock().unwrap().remove(&voice);
                            return Ok(());
                        }
                        // Whatever went wrong, the downloaded voice answers.
                        Err(e) => {
                            let Some(f) = fallback else { return Err(TtsError::cloud(e)) };
                            self.cloud_failed(&voice, &e);
                            if heard {
                                return Err(TtsError::cloud(e));
                            }
                            voice = f.clone();
                        }
                    }
                }
            }
        }
        // A sentence that was only a tag ("[laughs]") has nothing left to say.
        if !plain.chars().any(char::is_alphanumeric) {
            return Ok(());
        }
        let wav = self.request(&plain, &voice)?;
        let engine = Model::by_id(&voice).and_then(Model::engine).unwrap_or(Engine::Piper);
        let (rate, samples) = parse_wav(&wav).ok_or_else(|| TtsError::new(engine, "not a 16-bit mono WAV"))?;
        out(resample(&samples, rate, SAMPLE_RATE));
        Ok(())
    }

    /// `text` spoken by a cloud voice, a piece at a time, streamed when the voice can.
    fn speak_cloud(&self, client: &cloud::Client, text: &str, language: &str, out: &mut dyn FnMut(Vec<i16>)) -> Result<(), CloudError> {
        for piece in cloud::chunks(text, client.model.model.max_chars) {
            if client.streams_speech() {
                client.speak_stream(&piece, Some(language), out)?;
                continue;
            }
            let wav = client.speak(&piece, Some(language))?;
            let (rate, samples) = parse_wav(&wav).ok_or_else(|| CloudError::Invalid { provider: client.model.provider.name, why: "not a 16-bit mono WAV".into() })?;
            out(resample(&samples, rate, SAMPLE_RATE));
        }
        Ok(())
    }

    fn request(&self, text: &str, voice: &str) -> Result<Vec<u8>, TtsError> {
        let model = Model::by_id(voice).filter(|m| m.kind == Kind::Voice);
        let engine = model.and_then(Model::engine).unwrap_or(Engine::Piper);
        let model = model.ok_or_else(|| TtsError::new(engine, format!("{voice} isn't a voice")))?;
        let body = json!({"text": text, "voice": model.id}).to_string();
        // A server that died since the last request gets one restart.
        for attempt in 0..2 {
            let addr = self.addr(engine)?;
            match sidecar::post(addr, "/synthesize", "application/json", body.as_bytes(), REQUEST_TIMEOUT) {
                Ok(wav) => return Ok(wav),
                Err(e) if attempt == 0 && matches!(e.kind(), io::ErrorKind::ConnectionRefused | io::ErrorKind::ConnectionReset) => {
                    self.servers.lock().unwrap().remove(&engine);
                }
                Err(e) => return Err(TtsError::new(engine, e)),
            }
        }
        unreachable!()
    }

    fn addr(&self, engine: Engine) -> Result<SocketAddr, TtsError> {
        let mut servers = self.servers.lock().unwrap();
        if let Some(s) = servers.get_mut(&engine) {
            if s.is_running() {
                return Ok(s.addr);
            }
            servers.remove(&engine);
        }
        let s = self.start(engine).map_err(|why| TtsError::new(engine, why))?;
        let addr = s.addr;
        servers.insert(engine, s);
        Ok(addr)
    }

    fn start(&self, engine: Engine) -> Result<Process, String> {
        let dir = models_dir().ok_or("no data folder for the voices")?;
        // The fallbacks of cloud voices too: they may be all it speaks.
        let ids = self.config.voices.values().chain(self.config.fallbacks.values());
        let mut voices: Vec<&Model> = Vec::new();
        for m in ids.filter_map(|v| Model::by_id(v)).filter(|m| m.engine() == Some(engine)) {
            if !voices.iter().any(|v| v.id == m.id) {
                voices.push(m);
            }
        }
        let first = voices.first().ok_or("no voice downloaded")?;
        if let Some(missing) = voices.iter().find(|m| !m.is_installed()) {
            return Err(format!("the voice {} isn't downloaded", missing.id));
        }
        let python = self.config.pythons.get(&engine).ok_or_else(|| format!("{} isn't installed yet", engine.name()))?;
        let home = engine.dir().ok_or("no data folder")?;
        let mut cmd = flasher::command(python);
        let marker = match engine {
            Engine::Piper => {
                cmd.args(["-m", "piper.http_server", "-m", first.id]).arg("--data-dir").arg(&dir);
                "piper.http_server"
            }
        };
        Process::start(cmd, marker, Some(home.join("server.pid")), Some(home.join("server.log")), STARTUP_TIMEOUT)
    }
}

/// Rate and samples of a 16-bit mono PCM WAV.
pub fn parse_wav(wav: &[u8]) -> Option<(u32, Vec<i16>)> {
    if wav.len() < 12 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return None;
    }
    let (mut rate, mut pos) = (None, 12);
    while pos + 8 <= wav.len() {
        let id = &wav[pos..pos + 4];
        let len = u32::from_le_bytes(wav[pos + 4..pos + 8].try_into().ok()?) as usize;
        let body = wav.get(pos + 8..(pos + 8 + len).min(wav.len()))?;
        match id {
            b"fmt " if body.len() >= 16 => {
                let format = u16::from_le_bytes([body[0], body[1]]);
                let channels = u16::from_le_bytes([body[2], body[3]]);
                let bits = u16::from_le_bytes([body[14], body[15]]);
                if format != 1 || channels != 1 || bits != 16 {
                    return None;
                }
                rate = Some(u32::from_le_bytes(body[4..8].try_into().ok()?));
            }
            b"data" => return Some((rate?, body.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)).collect())),
            _ => {}
        }
        pos += 8 + len + (len & 1);
    }
    None
}

/// Band-limited resampling: a windowed sinc (Blackman, 16 zero crossings
/// each side), cut off just below the lower Nyquist frequency.
pub fn resample(input: &[i16], from: u32, to: u32) -> Vec<i16> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    const ZEROS: f64 = 16.0;
    let ratio = to as f64 / from as f64;
    // Cutoff as a fraction of the input's Nyquist frequency.
    let cutoff = ratio.min(1.0) * 0.95;
    let half = ZEROS / cutoff;
    let out_len = (input.len() as f64 * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for n in 0..out_len {
        let t = n as f64 / ratio;
        let (lo, hi) = (((t - half).ceil() as i64).max(0), ((t + half).floor() as i64).min(input.len() as i64 - 1));
        let (mut acc, mut norm) = (0.0, 0.0);
        for k in lo..=hi {
            let x = k as f64 - t;
            let sinc = if x == 0.0 { 1.0 } else { (std::f64::consts::PI * cutoff * x).sin() / (std::f64::consts::PI * cutoff * x) };
            let w = 0.42 + 0.5 * (std::f64::consts::PI * x / half).cos() + 0.08 * (2.0 * std::f64::consts::PI * x / half).cos();
            let h = sinc * w;
            acc += input[k as usize] as f64 * h;
            norm += h;
        }
        out.push((acc / norm).round().clamp(-32768.0, 32767.0) as i16);
    }
    out
}

/// `text` in sentences, so the first can be spoken while the next is
/// synthesized. Very short pieces stay with the one before.
pub fn sentences(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        current.push(c);
        let ends = ends_sentence(c) && chars.peek().is_none_or(|n| n.is_whitespace());
        if ends {
            push_sentence(&mut out, &mut current, MIN_SENTENCE_CHARS);
        }
    }
    push_sentence(&mut out, &mut current, 0);
    out
}

/// Shorter than this, a piece of text isn't a sentence of its own.
const MIN_SENTENCE_CHARS: usize = 12;

fn ends_sentence(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | ';' | '\n')
}

/// Where the first sentence of `text`, which is still being written, ends
/// (a byte offset): only once what follows has begun, so "3." of "3.5"
/// isn't taken for an end. Like [`sentences`], a very short one goes on to
/// the next end.
pub fn sentence_end(text: &str) -> Option<usize> {
    let mut chars = text.char_indices().peekable();
    let mut count = 0;
    while let Some((i, c)) = chars.next() {
        count += 1;
        if ends_sentence(c) && count >= MIN_SENTENCE_CHARS && chars.peek().is_some_and(|(_, n)| n.is_whitespace()) {
            return Some(i + c.len_utf8());
        }
    }
    None
}

/// `text` as a voice can read it. The model is asked for plain sentences
/// when it is heard, but Markdown, links, emoji and symbols still slip
/// through, and a voice spells them out or stumbles on them. The
/// [`AUDIO_TAGS`] stay, for the voice that acts them out.
pub fn speakable(text: &str, language: &str) -> String {
    let mut out = String::new();
    for line in text.lines().map(|l| speakable_line(l, language)).filter(|l| !l.is_empty()) {
        // A list item or a heading has no full stop: each line is a sentence.
        if !out.is_empty() {
            if !out.ends_with(is_mark) {
                out.push('.');
            }
            out.push(' ');
        }
        out.push_str(&line);
    }
    out
}

fn speakable_line(line: &str, language: &str) -> String {
    let line = without_urls(&link_texts(without_markers(line)));
    let mut out = String::new();
    let mut chars = line.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            c if c.is_alphanumeric() || c.is_whitespace() || is_mark(c) => out.push(c),
            '[' => match audio_tag_at(&line[i..]) {
                Some((tag, len)) => {
                    out.push_str(&format!(" [{tag}] "));
                    while chars.next_if(|(j, _)| *j < i + len).is_some() {}
                }
                None => out.push(' '),
            },
            '\'' | '’' | '(' | ')' | '%' | '€' | '$' | '£' | '-' => out.push(c),
            '–' | '—' => out.push(','),
            '…' => out.push('.'),
            '&' => out.push_str(&format!(" {} ", word(language, "and"))),
            '°' => {
                // "20°C" is said "20 degrees".
                if chars.peek().is_some_and(|(_, n)| matches!(n, 'C' | 'F')) {
                    chars.next();
                }
                out.push_str(&format!(" {}", word(language, "degrees")));
            }
            // * _ # ` ~ | / \ < > ] { } = + @ ", arrows, emoji.
            _ => out.push(' '),
        }
    }
    tidy(&out.replace(" - ", ", "))
}

/// The [`AUDIO_TAGS`] one `text` starts with (`[Laughs]` too), and its length in bytes.
fn audio_tag_at(text: &str) -> Option<(&'static str, usize)> {
    let inner = text.strip_prefix('[')?;
    let close = inner.find(']').filter(|&c| c <= 20)?;
    let name = inner[..close].trim().to_lowercase();
    AUDIO_TAGS.iter().find(|t| **t == name).map(|t| (*t, close + 2))
}

/// `text` without its [`AUDIO_TAGS`]: for a voice that would read them out,
/// and for the screen.
pub fn without_audio_tags(text: &str) -> String {
    let line = |l: &str| {
        let mut out = String::new();
        let mut rest = l;
        while let Some(open) = rest.find('[') {
            out.push_str(&rest[..open]);
            match audio_tag_at(&rest[open..]) {
                Some((_, len)) => rest = &rest[open + len..],
                None => {
                    out.push('[');
                    rest = &rest[open + 1..];
                }
            }
        }
        out.push_str(rest);
        out.split_whitespace().collect::<Vec<_>>().join(" ")
    };
    text.lines().map(line).collect::<Vec<_>>().join("\n")
}

fn is_mark(c: char) -> bool {
    matches!(c, '.' | ',' | ';' | ':' | '!' | '?')
}

/// Heading, quote and list markers at the start of a line.
fn without_markers(line: &str) -> &str {
    let mut line = line.trim();
    loop {
        let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let rest = if line.starts_with(['#', '>']) {
            line.trim_start_matches(['#', '>'])
        } else if let Some(rest) = ["- ", "* ", "+ ", "• "].iter().find_map(|m| line.strip_prefix(m)) {
            rest
        } else if digits > 0 && (line[digits..].starts_with(". ") || line[digits..].starts_with(") ")) {
            &line[digits + 2..]
        } else {
            return line;
        };
        line = rest.trim_start();
    }
}

/// `[text](url)` as its text.
fn link_texts(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find(']').map(|i| open + i) else { break };
        // Brackets that aren't a link (an audio tag) stay as they are.
        let end = rest[close..].strip_prefix("](").and_then(|after| after.find(')')).map(|i| close + 2 + i);
        let Some(end) = end else {
            out.push_str(&rest[..=close]);
            rest = &rest[close + 1..];
            continue;
        };
        out.push_str(&rest[..open]);
        out.push_str(&rest[open + 1..close]);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

fn without_urls(line: &str) -> String {
    let is_url = |w: &str| {
        let w = w.trim_start_matches(['(', '<']);
        w.starts_with("http://") || w.starts_with("https://") || w.starts_with("www.")
    };
    line.split(' ').filter(|w| !is_url(w)).collect::<Vec<_>>().join(" ")
}

/// One space between words, none before punctuation, no punctuation twice
/// (but "..." stays).
fn tidy(text: &str) -> String {
    let mut out = String::new();
    for word in text.split_whitespace() {
        if !out.is_empty() && !word.starts_with(is_mark) {
            out.push(' ');
        }
        for c in word.chars() {
            let doubled = out.ends_with(is_mark) && !(c == '.' && out.ends_with('.'));
            if is_mark(c) && (out.is_empty() || doubled) {
                continue;
            }
            out.push(c);
        }
    }
    out.trim().to_string()
}

/// The few words [`speakable`] puts in place of a symbol.
fn word(language: &str, what: &str) -> &'static str {
    let lang = language.get(..2).unwrap_or(language);
    match (what, lang) {
        ("and", "it") => "e",
        ("and", "es") => "y",
        ("and", "fr") => "et",
        ("and", "de") => "und",
        ("and", _) => "and",
        (_, "it") => "gradi",
        (_, "es") => "grados",
        (_, "fr") => "degrés",
        (_, "de") => "Grad",
        _ => "degrees",
    }
}

fn push_sentence(out: &mut Vec<String>, current: &mut String, min: usize) {
    let s = current.trim();
    if s.is_empty() {
        current.clear();
        return;
    }
    match out.last_mut() {
        Some(last) if last.chars().count() < min => {
            last.push(' ');
            last.push_str(s);
        }
        _ => out.push(s.to_string()),
    }
    current.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::wav;

    #[test]
    fn parses_what_wav_writes() {
        assert_eq!(parse_wav(&wav(&[1, -2, 3], 22_050)), Some((22_050, vec![1, -2, 3])));
        assert_eq!(parse_wav(b"RIFF\0\0\0\0WAVE"), None);
    }

    #[test]
    fn resampling_keeps_a_tone_and_its_level() {
        let tone: Vec<i16> = (0..22_050).map(|i| (10_000.0 * (2.0 * std::f64::consts::PI * 440.0 * i as f64 / 22_050.0).sin()) as i16).collect();
        let out = resample(&tone, 22_050, 16_000);
        assert_eq!(out.len(), 16_000);
        let want: Vec<f64> = (0..16_000).map(|i| 10_000.0 * (2.0 * std::f64::consts::PI * 440.0 * i as f64 / 16_000.0).sin()).collect();
        // Away from the edges, within a small fraction of the amplitude.
        let worst = out[200..15_800].iter().zip(&want[200..15_800]).map(|(a, b)| (*a as f64 - b).abs()).fold(0.0, f64::max);
        assert!(worst < 150.0, "error {worst}");
    }

    #[test]
    fn resampling_removes_what_16k_cannot_carry() {
        // 10 kHz is above 16 kHz's Nyquist frequency: it must not alias down.
        let tone: Vec<i16> = (0..22_050).map(|i| (10_000.0 * (2.0 * std::f64::consts::PI * 10_000.0 * i as f64 / 22_050.0).sin()) as i16).collect();
        let out = resample(&tone, 22_050, 16_000);
        let rms = (out[200..15_800].iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / 15_600.0).sqrt();
        assert!(rms < 300.0, "rms {rms}");
    }

    #[test]
    fn a_failed_cloud_voice_rests() {
        let tts = Tts::new(TtsConfig { pythons: BTreeMap::new(), voices: BTreeMap::new(), clients: BTreeMap::new(), fallbacks: BTreeMap::new() });
        assert!(!tts.resting("elevenlabs:voice/x") && tts.cloud_trouble().is_none());
        tts.cloud_failed("elevenlabs:voice/x", &CloudError::Network { provider: "ElevenLabs", why: "offline".into() });
        assert!(tts.resting("elevenlabs:voice/x") && !tts.resting("groq:hannah"));
        assert_eq!(tts.cloud_trouble().as_deref(), Some("ElevenLabs: offline"));
        // Once its time is up, it's asked again.
        tts.failed.lock().unwrap().values_mut().for_each(|(_, until)| *until = Instant::now());
        assert!(!tts.resting("elevenlabs:voice/x") && tts.cloud_trouble().is_none());
    }

    #[test]
    fn splits_sentences() {
        assert_eq!(sentences("Fatto. La faccia rings è a sinistra! E poi?"), ["Fatto. La faccia rings è a sinistra!", "E poi?"]);
        assert_eq!(sentences("It's 3.5 degrees warmer. Done"), ["It's 3.5 degrees warmer.", "Done"]);
        assert!(sentences("  ").is_empty());
    }

    #[test]
    fn a_sentence_ends_once_the_next_begins() {
        assert_eq!(sentence_end("It's 3."), None);
        assert_eq!(sentence_end("It's 3.5 degrees warmer."), None);
        assert_eq!(sentence_end("It's 3.5 degrees warmer. D"), Some(24));
        // Too short on its own: it goes on to the next.
        assert_eq!(sentence_end("Fatto. La faccia è a sinistra! E"), Some(31));
    }

    #[test]
    fn audio_tags_come_off_for_other_voices() {
        assert_eq!(without_audio_tags("[sighs] Va bene. [laughs]\nOk [nota]"), "Va bene.\nOk [nota]");
        assert_eq!(without_audio_tags("[Whispers]"), "");
    }

    #[test]
    fn speakable_drops_markdown_and_symbols() {
        assert_eq!(speakable("**Fatto!** Ho messo il timer 🎉", "it"), "Fatto! Ho messo il timer");
        assert_eq!(speakable("## Oggi\n- riunione alle 14:30\n- palestra & spesa", "it"), "Oggi. riunione alle 14:30. palestra e spesa");
        assert_eq!(speakable("Fuori ci sono 20°C, vedi [il meteo](https://meteo.it) o https://x.y", "it"), "Fuori ci sono 20 gradi, vedi il meteo o");
        assert_eq!(speakable("1. `email` → Marco / Luca", "en"), "email Marco Luca");
        assert_eq!(speakable("It's 3.5 degrees — warmer... Done", "en"), "It's 3.5 degrees, warmer... Done");
        assert_eq!(speakable("L'espresso costa 1,20 €", "it"), "L'espresso costa 1,20 €");
        assert_eq!(speakable("[sighs] Leggi [il meteo](https://meteo.it)", "it"), "[sighs] Leggi il meteo");
        assert_eq!(speakable("[Laughs] Davvero? [whispers] Non lo dico a nessuno [nota]", "it"), "[laughs] Davvero? [whispers] Non lo dico a nessuno nota");
    }
}
