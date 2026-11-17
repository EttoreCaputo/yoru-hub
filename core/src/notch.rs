//! The Yoru in the notch (docs/notch.md): a board made of software, so the
//! rest of the host treats it like any other. The manager starts a session
//! for [`TARGET`] when it is on ([`set_enabled`]); its [`crate::link::Link`]
//! runs over a socket pair to a thread here that answers the board's
//! protocol (docs/protocol.md in the firmware's repository): `hello`, the
//! tools, the voice's states and the speech it plays. Its ears and voice
//! are the Mac's, through `yoru-audio talk` (Apple's echo cancelling on),
//! and what it shows goes to the app as [`Event`]s, for the notch window.
//!
//! It hears only when asked: push-to-talk ([`talk`]) or the follow-up the
//! host asks for after an answer (`voice/listen`), with a simple level
//! detector telling speech from silence. No wake word, no WiFi, no firmware.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine;
use serde::Serialize;
use serde_json::{Value, json};

use crate::protocol::{self, Channel, Chunk, Decoder};

/// The manager's target for it, and the id its `hello` gives.
pub const TARGET: &str = "mac";
pub const ID: &str = "mac";
/// What its `hello` says it runs.
pub const FIRMWARE: &str = "notch";

/// The board's sample rate and frame, as the real one.
const FRAME: usize = 512;
/// How often the socket is looked at for the host going away.
const READ_WAIT: Duration = Duration::from_millis(200);
/// The microphone and the speaker go off after this long unused with the
/// notch open, and the Mac's microphone indicator with them. Closing the
/// notch lets them go at once ([`release`]).
const ENGINE_IDLE: Duration = Duration::from_secs(20);
/// How long the helper may take to put things back (the music it paused)
/// before it is killed.
const HELPER_EXIT: Duration = Duration::from_secs(2);
/// The longest utterance, as the board's `max_length`.
const MAX_UTTERANCE: Duration = Duration::from_secs(60);
/// After the last loud frame, this long still counts as speech before a
/// pause starts, as the board's VAD hangover: without it a follow-up ended
/// half a second sooner than on the board.
const HANGOVER: Duration = Duration::from_millis(500);
/// A frame this loud (dBFS, after echo cancelling) may be speech...
const SPEECH_DB: f32 = -42.0;
/// ...and this many in a row are.
const SPEECH_FRAMES: u32 = 3;

static ENABLED: AtomicBool = AtomicBool::new(false);
type Listener = Arc<dyn Fn(Event) + Send + Sync>;
static LISTENER: Mutex<Option<Listener>> = Mutex::new(None);
static BOARD: Mutex<Option<Weak<Board>>> = Mutex::new(None);

/// What the notch shows, for the app.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// The voice's state: idle, listening, thinking, speaking, error.
    State { state: String },
    /// How loud the microphone is, 0 to 1, while listening.
    Level { level: f32 },
    /// A reaction to play, by the board's name for it.
    React { name: String },
    /// An errand: a connector's tool, a search, the memory (`show_fetch`).
    Fetch { state: String, source: String, kind: String },
    /// Something went wrong with the Mac's audio.
    Error { message: String },
}

/// Whether the manager should run the Yoru in the notch.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Where the notch's [`Event`]s go.
pub fn on_event(listener: impl Fn(Event) + Send + Sync + 'static) {
    *LISTENER.lock().unwrap() = Some(Arc::new(listener));
}

fn tell(event: Event) {
    if let Event::Error { message } = &event {
        crate::hostlog::line(TARGET, &format!("error: {message}"));
    }
    let listener = LISTENER.lock().unwrap().clone();
    if let Some(listener) = listener {
        listener(event);
    }
}

fn board() -> Option<Arc<Board>> {
    BOARD.lock().unwrap().as_ref().and_then(Weak::upgrade)
}

/// Push-to-talk: listen while it's held, and send what was heard when let go.
pub fn talk(pressed: bool) {
    crate::hostlog::line(TARGET, if pressed { "talk: pressed" } else { "talk: let go" });
    if board().is_none() {
        crate::hostlog::line(TARGET, "talk: the notch's session isn't running");
    }
    if let Some(board) = board() {
        if pressed { board.begin("button", None) } else { board.end_button() }
    }
}

/// The pointer is on the notch: the user may be about to talk, so Gemini
/// Live, if on, opens its session ahead. The Mac's audio isn't touched:
/// it starts only when the user talks, since holding the microphone puts
/// Bluetooth headphones in their call mode and pauses the music.
pub fn pointer_on() {
    let Some(board) = board() else { return };
    if board.state.lock().unwrap().utterance.is_none() {
        board.notify("about_to_talk", json!({}));
    }
}

/// The notch closed: let go of the Mac's audio now, unless Yoru is still
/// hearing or talking.
pub fn release() {
    if let Some(board) = board() {
        let busy = {
            let state = board.state.lock().unwrap();
            state.utterance.is_some() || state.playing.is_some()
        };
        if !busy {
            board.audio_off(false);
        }
    }
}

/// Stop talking: what Yoru is saying, or what he's hearing.
pub fn hush() {
    if let Some(board) = board() {
        board.hush();
    }
}

/// Whether Yoru may use the Mac's microphone: `granted`, `denied`,
/// `restricted`, or `unknown` while the user was never asked. With `ask`,
/// macOS asks the user first if they never answered (it blocks until they
/// do). The permission is the app's: `yoru-audio` runs as part of it.
pub fn microphone(ask: bool) -> io::Result<String> {
    let program = crate::sidecar::find_program("yoru-audio", "YORU_AUDIO")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "yoru-audio isn't installed with the app"))?;
    let mut command = Command::new(program);
    command.arg("microphone");
    if ask {
        command.arg("--ask");
    }
    let out = command.stdin(Stdio::null()).stderr(Stdio::null()).output()?;
    let event: Value = serde_json::from_slice(&out.stdout).map_err(|e| io::Error::other(format!("yoru-audio microphone: {e}")))?;
    event["status"].as_str().map(str::to_string).ok_or_else(|| io::Error::other("yoru-audio microphone: no status"))
}

/// Start the board's thread; the host's end of the socket is returned.
pub(crate) fn open() -> io::Result<UnixStream> {
    let (host, ours) = UnixStream::pair()?;
    ours.set_read_timeout(Some(READ_WAIT))?;
    let board = Arc::new(Board { out: Mutex::new(ours.try_clone()?), state: Mutex::default(), audio: Mutex::default() });
    *BOARD.lock().unwrap() = Some(Arc::downgrade(&board));
    thread::Builder::new().name("yoru-notch".into()).spawn(move || {
        board.serve(ours);
        board.audio_off(true);
    })?;
    Ok(host)
}

/// What is being heard.
struct Utterance {
    id: u8,
    trigger: String,
    seq: u16,
    started: Instant,
    /// Frames in a row loud enough, and whether speech was heard.
    loud: u32,
    speech: bool,
    /// The last loud frame, for the pause that ends a follow-up.
    last_loud: Instant,
    /// A follow-up: how long to wait for speech, and the pause that ends it
    /// (after the [`HANGOVER`]; `voice/pause` changes it, for Smart Turn).
    listen: Option<(Duration, Duration)>,
    /// `utterance_pause` was sent for the pause going on: once per pause.
    paused: bool,
    /// The microphone is on and the host was told: false while the helper
    /// is still starting (about a second, for Apple's echo cancelling).
    ready: bool,
}

/// What is being played.
struct Playing {
    id: u8,
    samples: u64,
    started: Instant,
    ended: bool,
}

#[derive(Default)]
struct State {
    next_id: u8,
    utterance: Option<Utterance>,
    playing: Option<Playing>,
    volume: u8,
    voice: String,
    used: Option<Instant>,
}

/// `yoru-audio talk` and its commands.
struct Audio {
    child: Child,
    stdin: ChildStdin,
    running: bool,
    /// The helper said its engine runs.
    ready: bool,
}

struct Board {
    out: Mutex<UnixStream>,
    state: Mutex<State>,
    audio: Mutex<Option<Audio>>,
}

impl Board {
    /// Answer the host until it goes away.
    fn serve(self: &Arc<Self>, mut input: UnixStream) {
        self.state.lock().unwrap().volume = 70;
        let mut decoder = Decoder::new();
        let mut buf = [0u8; 8192];
        let mut last_tick = Instant::now();
        loop {
            match input.read(&mut buf) {
                Ok(0) => return,
                Ok(n) => {
                    let mut frames = Vec::new();
                    decoder.push(&buf[..n], |c| {
                        if let Chunk::Frame(f) = c {
                            frames.push(f);
                        }
                    });
                    for f in frames {
                        match f.channel {
                            c if c == Channel::Ctrl as u8 => self.control(&f.payload),
                            c if c == Channel::AudioDown as u8 => self.play(&f.payload),
                            _ => {}
                        }
                    }
                }
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => {}
                Err(_) => return,
            }
            if last_tick.elapsed() >= READ_WAIT {
                last_tick = Instant::now();
                self.tick();
            }
        }
    }

    fn send(&self, channel: Channel, payload: &[u8]) {
        let _ = self.out.lock().unwrap().write_all(&protocol::encode(channel, payload));
    }

    fn reply(&self, id: &Value, result: Value) {
        self.send(Channel::Ctrl, json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string().as_bytes());
    }

    fn refuse(&self, id: &Value, message: &str) {
        self.send(Channel::Ctrl, json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": message}}).to_string().as_bytes());
    }

    fn notify(&self, method: &str, params: Value) {
        self.send(Channel::Ctrl, json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string().as_bytes());
    }

    /// The board's own voice state, said to the host and shown.
    fn set_voice(&self, state: &str) {
        self.state.lock().unwrap().voice = state.to_string();
        self.notify("voice_state", json!({"state": state}));
        tell(Event::State { state: state.to_string() });
    }

    fn control(self: &Arc<Self>, payload: &[u8]) {
        let Ok(message) = serde_json::from_slice::<Value>(payload) else { return };
        let Some(id) = message.get("id").filter(|i| !i.is_null()) else { return };
        let params = &message["params"];
        match message["method"].as_str().unwrap_or_default() {
            "hello" => self.reply(id, hello()),
            "tools/list" => self.reply(id, json!({"tools": tools()})),
            "tools/call" => {
                let result = self.tool(params["name"].as_str().unwrap_or_default(), &params["arguments"]);
                self.reply(id, result);
            }
            "voice/state" => {
                // Said back, as the board does: the host's idea of the state
                // (alerts wait for "idle") follows what it set.
                self.reply(id, json!({}));
                self.set_voice(params["state"].as_str().unwrap_or("idle"));
            }
            "voice/listen" => {
                self.reply(id, json!({}));
                let wait = Duration::from_millis(params["wait_ms"].as_u64().unwrap_or(8000));
                let pause = Duration::from_millis(params["pause_ms"].as_u64().unwrap_or(700));
                self.begin("follow_up", Some((wait, pause)));
            }
            "voice/stop" => {
                self.reply(id, json!({}));
                self.end("host");
            }
            "voice/pause" => {
                // Smart Turn holds the pause long and stops the utterance
                // itself when the sentence sounds finished.
                let pause = Duration::from_millis(params["pause_ms"].as_u64().unwrap_or(700).clamp(200, 3000));
                if let Some(u) = self.state.lock().unwrap().utterance.as_mut().filter(|u| params["id"].as_u64() == Some(u.id as u64)) {
                    let wait = u.listen.map_or(Duration::from_secs(8), |(w, _)| w);
                    if u.trigger != "button" {
                        u.listen = Some((wait, pause));
                    }
                }
                self.reply(id, json!({}));
            }
            other => self.refuse(id, &format!("the notch has no {other}")),
        }
    }

    /// A board tool. Cards and notifications are shown by the app from its
    /// own state, so here they only need an answer.
    fn tool(&self, name: &str, args: &Value) -> Value {
        let ok = |text: &str| json!({"content": [{"type": "text", "text": text}], "isError": false});
        match name {
            "react" => {
                // As the board: no reaction over a conversation.
                if self.state.lock().unwrap().voice != "idle" {
                    return json!({"content": [{"type": "text", "text": "not now: a conversation is on"}], "isError": true});
                }
                let name = args["name"].as_str().unwrap_or_default().to_string();
                tell(Event::React { name: name.clone() });
                ok(&format!("reacting: {name}"))
            }
            "get_state" => {
                let state = self.state.lock().unwrap();
                let s = json!({
                    "activity": "awake",
                    "brightness": 100,
                    "audio": {"volume": state.volume},
                    "voice": {"wake_word": "push-to-talk", "wake_words": [], "muted": false, "state": state.voice},
                });
                json!({"content": [{"type": "text", "text": s.to_string()}], "structuredContent": s, "isError": false})
            }
            "set_volume" => {
                let percent = args["percent"].as_u64().unwrap_or(70).min(100) as u8;
                self.state.lock().unwrap().volume = percent;
                ok(&format!("volume {percent}%"))
            }
            "show_fetch" => {
                let s = |k: &str| args[k].as_str().unwrap_or_default().to_string();
                tell(Event::Fetch { state: s("state"), source: s("source"), kind: s("kind") });
                ok("ok")
            }
            "show_approval" | "hide_approval" | "show_notification" | "hide_notification" | "clear_notifications" => ok("ok"),
            _ => json!({"content": [{"type": "text", "text": format!("the notch has no {name}")}], "isError": true}),
        }
    }

    /// Start hearing: push-to-talk, or a follow-up the host asked for.
    fn begin(self: &Arc<Self>, trigger: &str, listen: Option<(Duration, Duration)>) {
        // Talking over Yoru stops him, as on the board.
        self.stop_playing("barge_in");
        let id = {
            let mut state = self.state.lock().unwrap();
            if let Some(u) = state.utterance.as_mut() {
                // Already listening for a follow-up: holding the key turns it
                // into push-to-talk, ending when the key is let go.
                if trigger == "button" {
                    u.trigger = "button".into();
                    u.listen = None;
                }
                return;
            }
            state.next_id = state.next_id.wrapping_add(1).max(1);
            let now = Instant::now();
            state.utterance =
                Some(Utterance { id: state.next_id, trigger: trigger.to_string(), seq: 0, started: now, loud: 0, speech: false, last_loud: now, listen, paused: false, ready: false });
            state.used = Some(now);
            state.next_id
        };
        if !self.audio_on() {
            crate::hostlog::line(TARGET, "the microphone wouldn't start");
            self.state.lock().unwrap().utterance = None;
            return;
        }
        if self.audio.lock().unwrap().as_ref().is_some_and(|a| a.ready) {
            self.listen();
        } else {
            // Shown only: the host hears of the utterance once it starts.
            crate::hostlog::line(TARGET, &format!("utterance #{id} waits for the microphone"));
            tell(Event::State { state: "starting".into() });
        }
    }

    /// The microphone is ready: start the utterance for real. Once only,
    /// whether the helper was ready already or said so later.
    fn listen(&self) {
        let (id, trigger) = {
            let mut state = self.state.lock().unwrap();
            let Some(u) = state.utterance.as_mut().filter(|u| !u.ready) else { return };
            let now = Instant::now();
            u.ready = true;
            // A follow-up's wait counts from when the microphone hears.
            u.started = now;
            u.last_loud = now;
            let started = (u.id, u.trigger.clone());
            state.used = Some(now);
            started
        };
        crate::hostlog::line(TARGET, &format!("utterance #{id} starts ({trigger})"));
        self.notify("utterance_start", json!({"id": id, "trigger": trigger, "rate": 16000, "format": "s16le"}));
        self.set_voice("listening");
        self.command(json!({"cmd": "mic", "on": true}));
    }

    /// The helper's engine runs.
    fn audio_ready(&self) {
        if let Some(a) = self.audio.lock().unwrap().as_mut() {
            a.ready = true;
        }
        self.listen();
    }

    /// The helper's engine wouldn't start: an utterance waiting for it ends.
    fn audio_failed(&self) {
        if let Some(a) = self.audio.lock().unwrap().as_mut() {
            a.running = false;
            a.ready = false;
        }
        let waiting = self.state.lock().unwrap().utterance.as_ref().is_some_and(|u| !u.ready);
        if waiting {
            self.end("error");
        }
    }

    /// The push-to-talk key let go.
    fn end_button(&self) {
        let button = self.state.lock().unwrap().utterance.as_ref().is_some_and(|u| u.trigger == "button");
        if button {
            self.end("button");
        }
    }

    /// Stop hearing and tell the host what came of it.
    fn end(&self, reason: &str) {
        let Some(u) = self.state.lock().unwrap().utterance.take() else { return };
        if !u.ready {
            // Let go before the microphone was on: the host never heard of it.
            crate::hostlog::line(TARGET, &format!("utterance #{} ends before the microphone was ready: {reason}", u.id));
            self.set_voice(if reason == "error" { "error" } else { "idle" });
            return;
        }
        self.command(json!({"cmd": "mic", "on": false}));
        // Push-to-talk sends what it has: a short word may be quiet.
        let speech = u.speech || (u.trigger == "button" && u.seq > 8);
        let reason = if !speech && reason != "host" { "no_speech" } else { reason };
        crate::hostlog::line(TARGET, &format!("utterance #{} ends: {reason}, speech {speech}, {} frames", u.id, u.seq));
        self.notify("utterance_end", json!({"id": u.id, "reason": reason, "speech": speech, "frames": u.seq}));
        tell(Event::Level { level: 0.0 });
        self.set_voice(if speech { "thinking" } else { "idle" });
    }

    fn hush(&self) {
        self.stop_playing("stopped");
        if self.state.lock().unwrap().utterance.is_some() {
            self.end("host");
        }
    }

    /// 32 ms the microphone heard, while listening.
    fn heard(&self, samples: &[i16]) {
        let db = level_db(samples);
        let (frame, end, pause) = {
            let mut state = self.state.lock().unwrap();
            let Some(u) = state.utterance.as_mut().filter(|u| u.ready) else { return };
            let now = Instant::now();
            if db > SPEECH_DB {
                u.loud += 1;
                u.last_loud = now;
                u.paused = false;
                if u.loud >= SPEECH_FRAMES {
                    u.speech = true;
                }
            } else {
                u.loud = 0;
            }
            let frame = crate::voice::audio_frame(u.id, u.seq, samples);
            u.seq = u.seq.wrapping_add(1);
            let quiet = u.last_loud.elapsed();
            // A pause has begun: the host may judge the sentence (Smart Turn).
            let pause = (u.speech && u.listen.is_some() && !u.paused && quiet > HANGOVER).then(|| {
                u.paused = true;
                json!({"id": u.id, "ms": u.seq as u64 * 32})
            });
            let end = match u.listen {
                _ if u.started.elapsed() > MAX_UTTERANCE => Some("too_long"),
                Some((wait, _)) if !u.speech && u.started.elapsed() > wait => Some("no_speech"),
                Some((_, pause)) if u.speech && quiet > HANGOVER + pause => Some("pause"),
                _ => None,
            };
            state.used = Some(now);
            (frame, end, pause)
        };
        self.send(Channel::AudioUp, &frame);
        if let Some(params) = pause {
            self.notify("utterance_pause", params);
        }
        tell(Event::Level { level: ((db + 60.0) / 50.0).clamp(0.0, 1.0) });
        if let Some(reason) = end {
            self.end(reason);
        }
    }

    /// An `audio_down` frame: Yoru's speech.
    fn play(self: &Arc<Self>, payload: &[u8]) {
        if payload.len() < 4 {
            return;
        }
        let (id, last) = (payload[0], payload[1] & 1 == 1);
        let pcm = &payload[4..];
        let replaced = {
            let state = self.state.lock().unwrap();
            state.playing.as_ref().is_some_and(|p| p.id != id && !p.ended)
        };
        if replaced {
            self.stop_playing("replaced");
        }
        let fresh = {
            let mut state = self.state.lock().unwrap();
            let fresh = state.playing.as_ref().is_none_or(|p| p.id != id);
            if fresh {
                state.playing = Some(Playing { id, samples: 0, started: Instant::now(), ended: false });
            }
            state.used = Some(Instant::now());
            fresh
        };
        if fresh {
            if !self.audio_on() {
                self.notify("playback_end", json!({"id": id, "reason": "starved", "ms": 0, "underruns": 0, "lost": 0}));
                self.state.lock().unwrap().playing = None;
                return;
            }
            self.set_voice("speaking");
        }
        if !pcm.is_empty() {
            let volume = self.state.lock().unwrap().volume as f32 / 100.0;
            let gain = volume * volume * 1.6;
            let scaled: Vec<u8> = pcm
                .chunks_exact(2)
                .flat_map(|b| ((i16::from_le_bytes([b[0], b[1]]) as f32 * gain).clamp(-32768.0, 32767.0) as i16).to_le_bytes())
                .collect();
            if let Some(p) = self.state.lock().unwrap().playing.as_mut() {
                p.samples += (pcm.len() / 2) as u64;
            }
            self.command(json!({"cmd": "play", "pcm": base64::engine::general_purpose::STANDARD.encode(scaled)}));
        }
        if last {
            self.command(json!({"cmd": "mark", "id": id}));
        }
    }

    /// The helper played a stream to its end.
    fn played(&self, id: u8) {
        let done = {
            let mut state = self.state.lock().unwrap();
            match state.playing.take() {
                Some(p) if p.id == id && !p.ended => Some(p),
                other => {
                    state.playing = other;
                    None
                }
            }
        };
        if let Some(p) = done {
            self.notify("playback_end", json!({"id": id, "reason": "done", "ms": p.started.elapsed().as_millis() as u64, "underruns": 0, "lost": 0}));
            self.state.lock().unwrap().used = Some(Instant::now());
            if self.state.lock().unwrap().utterance.is_none() {
                self.set_voice("idle");
            }
        }
    }

    /// Stop what is playing, telling the host why.
    fn stop_playing(&self, reason: &str) {
        let playing = self.state.lock().unwrap().playing.take();
        if let Some(p) = playing.filter(|p| !p.ended) {
            self.command(json!({"cmd": "flush"}));
            let ms = (p.samples * 1000 / 16000).min(p.started.elapsed().as_millis() as u64);
            self.notify("playback_end", json!({"id": p.id, "reason": reason, "ms": ms, "underruns": 0, "lost": 0}));
            if reason != "barge_in" && reason != "replaced" {
                self.set_voice("idle");
            }
        }
    }

    /// Turn the microphone off once nothing has used it for a while.
    fn tick(&self) {
        let idle = {
            let state = self.state.lock().unwrap();
            state.utterance.is_none() && state.playing.is_none() && state.used.is_some_and(|t| t.elapsed() > ENGINE_IDLE)
        };
        if idle {
            self.audio_off(false);
        }
        // A follow-up whose microphone never started still has to end.
        let stale = self.state.lock().unwrap().utterance.as_ref().is_some_and(|u| u.started.elapsed() > MAX_UTTERANCE + Duration::from_secs(2));
        if stale {
            self.end("too_long");
        }
    }

    /// Make sure the helper runs with its engine on. False when it can't.
    fn audio_on(self: &Arc<Self>) -> bool {
        let mut audio = self.audio.lock().unwrap();
        if audio.as_mut().is_some_and(|a| matches!(a.child.try_wait(), Ok(Some(_)))) {
            *audio = None;
        }
        if audio.is_none() {
            match self.spawn() {
                Ok(a) => *audio = Some(a),
                Err(e) => {
                    drop(audio);
                    tell(Event::Error { message: e.to_string() });
                    self.set_voice("error");
                    return false;
                }
            }
        }
        let a = audio.as_mut().unwrap();
        if !a.running {
            a.running = writeln!(a.stdin, "{}", json!({"cmd": "start"})).is_ok();
        }
        a.running
    }

    /// Let go of the Mac's audio by ending the helper; `quit` also ends the
    /// notch's session. Stopping only the engine isn't enough: with voice
    /// processing set up, the helper kept Bluetooth headphones in their
    /// call mode (16 kHz mono) until it exited. The next use starts it again.
    /// Its stdin closed, the helper restarts the music it paused and exits.
    fn audio_off(&self, quit: bool) {
        let audio = self.audio.lock().unwrap().take();
        if let Some(Audio { mut child, stdin, .. }) = audio {
            drop(stdin);
            let deadline = Instant::now() + HELPER_EXIT;
            while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
            let _ = child.kill();
            let _ = child.wait();
            if !quit {
                crate::hostlog::line(TARGET, "the Mac's audio is let go");
            }
        }
        if quit {
            BOARD.lock().unwrap().take();
        }
    }

    fn command(&self, command: Value) {
        if let Some(a) = self.audio.lock().unwrap().as_mut() {
            let _ = writeln!(a.stdin, "{command}");
        }
    }

    fn spawn(self: &Arc<Self>) -> io::Result<Audio> {
        let program = crate::sidecar::find_program("yoru-audio", "YORU_AUDIO")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "yoru-audio isn't installed with the app"))?;
        let mut child = Command::new(program).arg("talk").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| io::Error::other("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| io::Error::other("no stdout"))?;
        let board = Arc::downgrade(self);
        thread::Builder::new().name("yoru-notch-audio".into()).spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Some(board) = board.upgrade() else { break };
                let Ok(event) = serde_json::from_str::<Value>(&line) else { continue };
                match event["event"].as_str() {
                    Some("audio") => {
                        let Some(bytes) = event["pcm"].as_str().and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok()) else { continue };
                        let samples: Vec<i16> = bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
                        for frame in samples.chunks(FRAME) {
                            board.heard(frame);
                        }
                    }
                    Some("played") => board.played(event["id"].as_u64().unwrap_or(0) as u8),
                    Some("ready") => board.audio_ready(),
                    Some("error") => {
                        tell(Event::Error { message: event["message"].as_str().unwrap_or("yoru-audio failed").to_string() });
                        if event["fatal"].as_bool() == Some(true) {
                            board.audio_failed();
                        }
                    }
                    _ => {}
                }
            }
        })?;
        Ok(Audio { child, stdin, running: false, ready: false })
    }
}

/// What the notch says about itself.
fn hello() -> Value {
    json!({
        "protocol": protocol::PROTOCOL,
        "firmware": FIRMWARE,
        "board": "notch",
        "max_payload": protocol::MAX_PAYLOAD,
        "channels": ["ctrl", "audio_up", "audio_down"],
        "capabilities": [],
        "id": ID,
        "transport": "local",
    })
}

/// The tools it offers: the real board's `react` when its list is known
/// (its names are the ones the notch plays too), and a few of its own.
/// The notch's tools, for a request typed in its field: it acts on the
/// notch, as one said to it does, not on the board on the desk.
pub fn typed_tools() -> Vec<crate::link::Tool> {
    tools().into_iter().filter_map(|t| serde_json::from_value(t).ok()).collect()
}

/// Run one of [`typed_tools`].
pub fn call_typed(name: &str, arguments: &Value) -> Result<String, String> {
    let board = board().ok_or("the notch isn't on")?;
    let result = board.tool(name, arguments);
    let text = result["content"][0]["text"].as_str().unwrap_or_default().to_string();
    if result["isError"].as_bool().unwrap_or(false) { Err(text) } else { Ok(text) }
}

fn tools() -> Vec<Value> {
    let known = crate::link::load_tools_cache(crate::profile::BoardKind::Yoru).unwrap_or_default();
    let react = known
        .iter()
        .find(|t| t.name == "react")
        .and_then(|t| serde_json::to_value(t).ok())
        .unwrap_or_else(|| json!({"name": "react", "description": "Play a short reaction on Yoru's face.", "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]}}));
    let object = |props: Value| json!({"type": "object", "properties": props});
    vec![
        react,
        json!({"name": "get_state", "description": "Yoru's state: volume, voice.", "inputSchema": object(json!({}))}),
        json!({"name": "set_volume", "description": "Set the speaking volume, 0-100.", "inputSchema": object(json!({"percent": {"type": "integer", "minimum": 0, "maximum": 100}}))}),
    ]
}

/// A frame's loudness in dBFS.
fn level_db(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return -100.0;
    }
    let sum: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    let rms = (sum / samples.len() as f64).sqrt() / 32768.0;
    (20.0 * rms.max(1e-6).log10()) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_and_speech_levels() {
        assert!(level_db(&[0; 512]) < -90.0);
        let loud: Vec<i16> = (0..512).map(|i| if i % 2 == 0 { 8000 } else { -8000 }).collect();
        assert!(level_db(&loud) > SPEECH_DB);
    }

    #[test]
    fn it_says_hello_as_a_board() {
        let hello: crate::link::Hello = serde_json::from_value(hello()).unwrap();
        assert_eq!((hello.protocol, hello.id.as_deref()), (protocol::PROTOCOL, Some(ID)));
        assert!(hello.capabilities.is_empty(), "no cloud keys, no flashing");
    }

    #[test]
    fn the_host_talks_to_it_like_a_board() {
        let link = crate::link::Link::local(open().unwrap(), |_| {}).unwrap();
        let wait = Duration::from_secs(2);
        let hello = link.hello(wait).unwrap();
        assert_eq!(hello.firmware, FIRMWARE);
        let tools = link.list_tools(wait).unwrap();
        assert!(["react", "get_state", "set_volume"].iter().all(|n| tools.iter().any(|t| t.name == *n)));
        link.call_tool("set_volume", json!({"percent": 40}), wait).unwrap();
        let state = link.call_tool("get_state", json!({}), wait).unwrap();
        assert_eq!(state.structured_content.unwrap()["audio"]["volume"], 40);
        assert!(link.call_tool("set_icon", json!({}), wait).unwrap().is_error);
        assert!(link.call("no/such", json!({}), wait).is_err());
        assert!(link.call("voice/pause", json!({}), wait).is_ok());
    }
}
