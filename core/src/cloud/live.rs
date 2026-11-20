//! Gemini Live: Google's audio-to-audio model, over a WebSocket
//! (`BidiGenerateContent`). It hears the utterance itself and answers with
//! its own voice, so one session stands in for speech-to-text, the brain
//! and the voice (docs/assistant.md, Gemini Live).
//!
//! The board already tells where a sentence ends, so Google's own voice
//! detection is off: each utterance goes out between `activityStart` and
//! `activityEnd`. The session keeps the conversation, so a follow-up is
//! sent to the same one. It answers 16-bit PCM at 24 kHz; [`Downsampler`]
//! brings it to the board's 16 kHz.

use std::io::ErrorKind;
use std::net::TcpStream;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{Value, json};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

/// The model: free on AI Studio's free tier.
pub const MODEL: &str = "gemini-3.8-live";
/// The models tried in order: one Google gives no free quota on a project
/// (a 3.8 or 3.1 one may refuse at once) is skipped for [`REFUSED_FOR`].
pub const MODELS: &[&str] = &[MODEL, "gemini-3.1-flash-live-preview", "gemini-2.5-flash-native-audio-preview-12-2025"];
const REFUSED_FOR: Duration = Duration::from_secs(3600);
static REFUSED: Mutex<Vec<(&str, Instant)>> = Mutex::new(Vec::new());
pub const URL: &str = "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";
/// What Google sends: 16-bit PCM, little-endian.
pub const OUTPUT_RATE: u32 = 24_000;
/// All of Google's prebuilt voices: name, how Google describes it, and
/// whether it sounds like a man or a woman, so the app can group them. The
/// user picks any; Yoru is male by default (docs/CONCEPT.md in the
/// firmware's repository).
pub const VOICES: &[(&str, &str, &str)] = &[
    ("Achird", "friendly", "male"),
    ("Algenib", "gravelly", "male"),
    ("Algieba", "smooth", "male"),
    ("Alnilam", "firm", "male"),
    ("Charon", "informative", "male"),
    ("Enceladus", "breathy", "male"),
    ("Fenrir", "excitable", "male"),
    ("Iapetus", "clear", "male"),
    ("Orus", "firm", "male"),
    ("Puck", "upbeat", "male"),
    ("Rasalgethi", "informative", "male"),
    ("Sadachbia", "lively", "male"),
    ("Sadaltager", "knowledgeable", "male"),
    ("Schedar", "even", "male"),
    ("Umbriel", "easy-going", "male"),
    ("Zubenelgenubi", "casual", "male"),
    ("Achernar", "soft", "female"),
    ("Aoede", "breezy", "female"),
    ("Autonoe", "bright", "female"),
    ("Callirrhoe", "easy-going", "female"),
    ("Despina", "smooth", "female"),
    ("Erinome", "clear", "female"),
    ("Gacrux", "mature", "female"),
    ("Kore", "firm", "female"),
    ("Laomedeia", "upbeat", "female"),
    ("Leda", "youthful", "female"),
    ("Pulcherrima", "forward", "female"),
    ("Sulafat", "warm", "female"),
    ("Vindemiatrix", "gentle", "female"),
    ("Zephyr", "bright", "female"),
];
pub const DEFAULT_VOICE: &str = "Puck";

/// How long a reply may keep us waiting between two messages: a tool that
/// asks the user (Allow on the board) waits up to a minute itself.
const READ_TIMEOUT: Duration = Duration::from_secs(90);
/// The utterance goes out in pieces of this many samples (one second).
const SEND_CHUNK: usize = 16_000;

/// How a session is set up: once, when it opens.
#[derive(Debug, Clone, Default)]
pub struct Setup {
    /// [`MODEL`] when empty.
    pub model: String,
    pub system: String,
    /// Anthropic's shape: `{name, description, input_schema}`.
    pub tools: Vec<Value>,
    /// One of [`VOICES`] (any Google voice name works).
    pub voice: String,
    /// Google Search grounding.
    pub web: bool,
}

/// The first message: the model, its voice, its prompt and tools, no voice
/// detection of Google's, and transcripts of both sides.
pub fn setup_message(setup: &Setup) -> Value {
    let declarations: Vec<Value> = setup.tools.iter().map(declaration).collect();
    let mut tools = Vec::new();
    if !declarations.is_empty() {
        tools.push(json!({"functionDeclarations": declarations}));
    }
    if setup.web {
        tools.push(json!({"googleSearch": {}}));
    }
    let model = if setup.model.is_empty() { MODEL } else { &setup.model };
    let mut generation = json!({
        "responseModalities": ["AUDIO"],
        "speechConfig": {"voiceConfig": {"prebuiltVoiceConfig": {"voiceName": setup.voice}}},
    });
    // 2.5 thinks before it speaks unless told not to; 3.8 has no such setting.
    if model.starts_with("gemini-2.5") {
        generation["thinkingConfig"] = json!({"thinkingBudget": 0});
    }
    json!({"setup": {
        "model": format!("models/{model}"),
        "generationConfig": generation,
        "systemInstruction": {"parts": [{"text": setup.system}]},
        "tools": tools,
        "realtimeInputConfig": {"automaticActivityDetection": {"disabled": true}},
        "inputAudioTranscription": {},
        "outputAudioTranscription": {},
    }})
}

/// One of Yoru's tools as a Live function. Blocking: the model waits for
/// its result before it goes on talking, as the other brains do.
fn declaration(tool: &Value) -> Value {
    let mut schema = tool["input_schema"].clone();
    if !schema.is_object() {
        schema = json!({"type": "object", "properties": {}});
    }
    if let Some(map) = schema.as_object_mut() {
        map.remove("$schema");
    }
    json!({
        "name": tool["name"],
        "description": tool["description"],
        "parametersJsonSchema": schema,
        "behavior": "BLOCKING",
    })
}

/// What a session asks of whoever runs it during a turn.
pub trait Turner {
    /// Speech, 24 kHz.
    fn audio(&mut self, samples: Vec<i16>);
    /// Run a tool: what it said, or why it failed.
    fn call(&mut self, name: &str, arguments: &Value) -> Result<String, String>;
}

/// What came of a turn.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Turn {
    /// What Google heard.
    pub heard: String,
    /// The language it heard it in (`it`, `en`), when it said.
    pub language: Option<String>,
    /// What it said.
    pub said: String,
    /// Samples of speech it sent.
    pub samples: usize,
    /// It was cut off by new input.
    pub interrupted: bool,
}

/// A Live session.
pub struct Session {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    /// Google said it will close soon (`goAway`): open another next time.
    going: bool,
}

impl Session {
    /// Open a session with the AI Studio `key` on the first of [`MODELS`]
    /// that has quota left; the model it runs.
    pub fn open_any(key: &str, setup: &Setup) -> Result<(Self, &'static str), String> {
        let mut last = None;
        for &model in MODELS {
            let refused = REFUSED.lock().unwrap().iter().any(|(m, at)| *m == model && at.elapsed() < REFUSED_FOR);
            if refused {
                continue;
            }
            match Self::open(key, &Setup { model: model.into(), ..setup.clone() }) {
                Ok(session) => return Ok((session, model)),
                Err(e) if is_limit(&e) => {
                    crate::hostlog::line("live", &format!("Gemini Live: {model} refused, trying the next model: {e}"));
                    REFUSED.lock().unwrap().retain(|(m, _)| *m != model);
                    REFUSED.lock().unwrap().push((model, Instant::now()));
                    last = Some(e);
                }
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| "Gemini Live's free limits were all reached; they come back within the hour".into()))
    }

    /// Open a session with the AI Studio `key`.
    pub fn open(key: &str, setup: &Setup) -> Result<Self, String> {
        Self::open_at(&format!("{URL}?key={key}"), setup)
    }

    /// Open a session at `url` (a test's server).
    pub fn open_at(url: &str, setup: &Setup) -> Result<Self, String> {
        Self::connect(url, &setup_message(setup))
    }

    /// Connect and send `setup`, until Google says it's ready.
    fn connect(url: &str, setup: &Value) -> Result<Self, String> {
        tls_provider();
        let (socket, _) = tungstenite::connect(url).map_err(|e| match e {
            tungstenite::Error::Http(response) => {
                let body = response.body().as_deref().map(String::from_utf8_lossy).unwrap_or_default();
                format!("Gemini Live refused the connection ({}): {body}", response.status())
            }
            e => format!("could not reach Gemini Live: {e}"),
        })?;
        let mut session = Self { socket, going: false };
        session.set_timeout(READ_TIMEOUT);
        session.send(setup)?;
        loop {
            let message = session.receive()?;
            if message.get("setupComplete").is_some() {
                return Ok(session);
            }
        }
    }

    /// Still good for another turn.
    pub fn is_open(&self) -> bool {
        !self.going && self.socket.can_write()
    }

    /// Say `samples` (16 kHz) and hear the answer to the end of the turn,
    /// the speech and the tool calls going to `turner` as they come.
    /// Google listens in real time, so an utterance sent whole is answered
    /// about its length later: [`Session::begin`] and [`Session::audio`] while
    /// the user speaks are quicker.
    pub fn turn(&mut self, samples: &[i16], turner: &mut dyn Turner) -> Result<Turn, String> {
        self.begin()?;
        for chunk in samples.chunks(SEND_CHUNK) {
            self.audio(chunk)?;
        }
        self.finish(turner)
    }

    /// The user starts speaking.
    pub fn begin(&mut self) -> Result<(), String> {
        self.send(&json!({"realtimeInput": {"activityStart": {}}}))
    }

    /// What they said so far, 16 kHz, as it comes.
    pub fn audio(&mut self, samples: &[i16]) -> Result<(), String> {
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        self.send(&json!({"realtimeInput": {"audio": {"data": data, "mimeType": "audio/pcm;rate=16000"}}}))
    }

    /// They're done: hear the answer to the end of the turn, as [`Session::turn`].
    pub fn finish(&mut self, turner: &mut dyn Turner) -> Result<Turn, String> {
        self.send(&json!({"realtimeInput": {"activityEnd": {}}}))?;
        let mut turn = Turn::default();
        loop {
            let message = self.receive()?;
            if let Some(calls) = message.pointer("/toolCall/functionCalls").and_then(Value::as_array) {
                let responses: Vec<Value> = calls
                    .iter()
                    .map(|call| {
                        let name = call["name"].as_str().unwrap_or_default();
                        let args = if call["args"].is_object() { call["args"].clone() } else { json!({}) };
                        let response = match turner.call(name, &args) {
                            Ok(text) => json!({"result": text}),
                            Err(e) => json!({"error": e}),
                        };
                        json!({"id": call["id"], "name": name, "response": response})
                    })
                    .collect();
                self.send(&json!({"toolResponse": {"functionResponses": responses}}))?;
            }
            if message.get("goAway").is_some() {
                self.going = true;
            }
            let Some(content) = message.get("serverContent") else { continue };
            for part in content.pointer("/modelTurn/parts").and_then(Value::as_array).into_iter().flatten() {
                if let Some(data) = part.pointer("/inlineData/data").and_then(Value::as_str) {
                    let pcm = pcm_of(data)?;
                    turn.samples += pcm.len();
                    turner.audio(pcm);
                }
            }
            if let Some(t) = content.pointer("/inputTranscription/text").and_then(Value::as_str) {
                turn.heard.push_str(t);
            }
            if let Some(code) = content.pointer("/inputTranscription/languageCode").and_then(Value::as_str) {
                turn.language = code.split(['-', '_']).next().filter(|c| !c.is_empty()).map(str::to_lowercase);
            }
            if let Some(t) = content.pointer("/outputTranscription/text").and_then(Value::as_str) {
                turn.said.push_str(t);
            }
            turn.interrupted |= content["interrupted"].as_bool().unwrap_or(false);
            if content["turnComplete"].as_bool().unwrap_or(false) {
                turn.heard = tidy(&turn.heard);
                turn.said = tidy(&turn.said);
                return Ok(turn);
            }
        }
    }

    fn set_timeout(&mut self, timeout: Duration) {
        let tcp = match self.socket.get_mut() {
            MaybeTlsStream::Plain(s) => s,
            MaybeTlsStream::Rustls(s) => &mut s.sock,
            _ => return,
        };
        let _ = tcp.set_read_timeout(Some(timeout));
    }

    fn send(&mut self, message: &Value) -> Result<(), String> {
        self.socket.send(Message::text(message.to_string())).map_err(|e| format!("Gemini Live: {e}"))
    }

    /// The next message from Google, as JSON (it comes as text or binary).
    fn receive(&mut self) -> Result<Value, String> {
        self.poll()?.ok_or_else(|| "Gemini Live stopped answering".to_string())
    }

    /// The next message, or `None` when none came within the read timeout.
    fn poll(&mut self) -> Result<Option<Value>, String> {
        loop {
            let message = match self.socket.read() {
                Ok(m) => m,
                Err(tungstenite::Error::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => return Ok(None),
                Err(e) => return Err(format!("Gemini Live: {e}")),
            };
            let bytes = match message {
                Message::Text(t) => t.as_bytes().to_vec(),
                Message::Binary(b) => b.to_vec(),
                Message::Close(frame) => {
                    self.going = true;
                    return Err(closed(frame.as_ref().map(|f| f.reason.as_str()).unwrap_or_default()));
                }
                _ => continue,
            };
            return serde_json::from_slice(&bytes).map(Some).map_err(|e| format!("Gemini Live sent something unreadable: {e}"));
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.socket.close(None);
        let _ = self.socket.flush();
    }
}

/// Why Google closed the session, said plainly.
fn closed(reason: &str) -> String {
    let lower = reason.to_lowercase();
    if lower.contains("api key") {
        "Google refused the API key: check it in Settings, Google AI Studio".into()
    } else if lower.contains("quota") || lower.contains("exhausted") || lower.contains("rate") {
        format!("Gemini Live's free limit was reached: {reason}")
    } else if reason.is_empty() {
        "Gemini Live closed the session".into()
    } else {
        format!("Gemini Live closed the session: {reason}")
    }
}

/// The error is a quota Google refused.
pub fn is_limit(error: &str) -> bool {
    error.starts_with("Gemini Live's free limit")
}

/// Transcripts come in pieces, with odd spacing.
fn tidy(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Base64 PCM to samples.
fn pcm_of(data: &str) -> Result<Vec<i16>, String> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| format!("Gemini Live sent bad audio: {e}"))?;
    Ok(bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect())
}

/// 24 kHz to 16 kHz, a chunk at a time: two samples out of every three in,
/// lightly smoothed first so what's above 8 kHz doesn't fold back down.
#[derive(Default)]
pub struct Downsampler {
    carry: Vec<i16>,
    last: i16,
}

impl Downsampler {
    pub fn push(&mut self, input: &[i16]) -> Vec<i16> {
        self.carry.extend_from_slice(input);
        let whole = self.carry.len() / 3 * 3;
        let mut out = Vec::with_capacity(whole / 3 * 2);
        for t in self.carry[..whole].chunks_exact(3) {
            let (a, b, c) = (t[0] as i32, t[1] as i32, t[2] as i32);
            // At input positions 0 and 1.5 of each three.
            out.push(((self.last as i32 + 2 * a + b) / 4) as i16);
            out.push(((b + c) / 2) as i16);
            self.last = t[2];
        }
        self.carry.drain(..whole);
        out
    }
}

/// Google's speech-to-text on the Live API: free with no daily limit (20,000
/// tokens a minute), much better at a short Italian sentence than a Live
/// session's own transcript of the user.
pub const TRANSCRIBE_MODEL: &str = "gemini-3.5-transcribe-live";

/// Write down `samples` (16 kHz) with [`TRANSCRIBE_MODEL`]: `languages` are
/// BCP-47 codes to expect (none: any), `words` names to spell as given.
pub fn transcribe(key: &str, samples: &[i16], languages: &[&str], words: &[&str]) -> Result<String, String> {
    transcribe_with(key, TRANSCRIBE_MODEL, samples, languages, words)
}

/// [`transcribe`] with another of Google's Live transcription models.
pub fn transcribe_with(key: &str, model: &str, samples: &[i16], languages: &[&str], words: &[&str]) -> Result<String, String> {
    transcribe_at(&format!("{URL}?key={key}"), model, samples, languages, words)
}

/// [`transcribe_with`] at `url` (a test's server).
pub fn transcribe_at(url: &str, model: &str, samples: &[i16], languages: &[&str], words: &[&str]) -> Result<String, String> {
    let setup = json!({"setup": {
        "model": format!("models/{model}"),
        "generationConfig": {"responseModalities": ["TEXT"]},
        "inputAudioTranscription": {"languageCodes": languages, "customVocabulary": words},
        "realtimeInputConfig": {"automaticActivityDetection": {"disabled": true}},
    }});
    let mut session = Session::connect(url, &setup)?;
    session.set_timeout(TRANSCRIBE_TIMEOUT);
    session.begin()?;
    for chunk in samples.chunks(SEND_CHUNK) {
        session.audio(chunk)?;
    }
    session.send(&json!({"realtimeInput": {"activityEnd": {}}}))?;
    let mut text = String::new();
    loop {
        let message = session.receive()?;
        let Some(content) = message.get("serverContent") else { continue };
        if let Some(t) = content.pointer("/inputTranscription/text").and_then(Value::as_str) {
            text.push_str(t);
        }
        if content["turnComplete"].as_bool().unwrap_or(false) || content["generationComplete"].as_bool().unwrap_or(false) {
            return Ok(tidy(&text));
        }
    }
}

/// How long a transcription may take: it runs beside the answer.
const TRANSCRIBE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a [`Scribe`] waits for Google before sending what came meanwhile.
const SCRIBE_POLL: Duration = Duration::from_millis(40);

enum ScribeInput {
    Audio(Vec<i16>),
    End,
}

/// [`TRANSCRIBE_MODEL`] while the user speaks: the audio goes in as it
/// comes, and the words written so far go to `on_words` as Google writes
/// them, so they can be shown before the user has finished. It runs on its
/// own thread, which also opens the connection: nothing waits for it.
pub struct Scribe {
    tx: std::sync::mpsc::Sender<ScribeInput>,
    done: std::sync::mpsc::Receiver<Result<String, String>>,
}

impl Scribe {
    /// Start writing down what comes, in `languages` (BCP-47; none: any),
    /// spelling `words` as given.
    pub fn start(key: &str, languages: &[&str], words: &[&str], on_words: impl FnMut(&str) + Send + 'static) -> Scribe {
        Self::start_at(format!("{URL}?key={key}"), languages, words, on_words)
    }

    /// [`Scribe::start`] at `url` (a test's server).
    pub fn start_at(url: String, languages: &[&str], words: &[&str], mut on_words: impl FnMut(&str) + Send + 'static) -> Scribe {
        let (tx, rx) = std::sync::mpsc::channel();
        let (done_tx, done) = std::sync::mpsc::channel();
        let setup = json!({"setup": {
            "model": format!("models/{TRANSCRIBE_MODEL}"),
            "generationConfig": {"responseModalities": ["TEXT"]},
            "inputAudioTranscription": {"languageCodes": languages, "customVocabulary": words},
            "realtimeInputConfig": {"automaticActivityDetection": {"disabled": true}},
        }});
        let _ = std::thread::Builder::new().name("yoru-scribe".into()).spawn(move || {
            let _ = done_tx.send(Self::write(&url, &setup, &rx, &mut on_words));
        });
        Scribe { tx, done }
    }

    /// More of what the user says.
    pub fn audio(&self, samples: &[i16]) {
        let _ = self.tx.send(ScribeInput::Audio(samples.to_vec()));
    }

    /// The user is done: everything they said, written.
    pub fn finish(self) -> Result<String, String> {
        let _ = self.tx.send(ScribeInput::End);
        self.done.recv_timeout(TRANSCRIBE_TIMEOUT + Duration::from_secs(5)).map_err(|_| "the transcription took too long".to_string())?
    }

    fn write(url: &str, setup: &Value, rx: &std::sync::mpsc::Receiver<ScribeInput>, on_words: &mut dyn FnMut(&str)) -> Result<String, String> {
        let mut session = Session::connect(url, setup)?;
        session.set_timeout(SCRIBE_POLL);
        session.begin()?;
        let (mut text, mut shown) = (String::new(), String::new());
        let mut show = |words: String| {
            if words != shown {
                on_words(&words);
                shown = words;
            }
        };
        let mut ended: Option<Instant> = None;
        loop {
            // What came while connecting or waiting goes out first.
            loop {
                match rx.try_recv() {
                    Ok(ScribeInput::Audio(samples)) => {
                        for chunk in samples.chunks(SEND_CHUNK) {
                            session.audio(chunk)?;
                        }
                    }
                    Ok(ScribeInput::End) => {
                        session.send(&json!({"realtimeInput": {"activityEnd": {}}}))?;
                        ended = Some(Instant::now());
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    // Nobody wants the words any more.
                    Err(std::sync::mpsc::TryRecvError::Disconnected) if ended.is_none() => return Err("the utterance was dropped".into()),
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                }
            }
            if let Some(content) = session.poll()?.as_ref().and_then(|m| m.get("serverContent")) {
                // A draft of everything so far, rewritten as more is heard.
                if let Some(t) = content.pointer("/interimInputTranscription/text").and_then(Value::as_str) {
                    if text.is_empty() {
                        show(tidy(t));
                    }
                }
                if let Some(t) = content.pointer("/inputTranscription/text").and_then(Value::as_str) {
                    text.push_str(t);
                    show(tidy(&text));
                }
                let complete = content["turnComplete"].as_bool().unwrap_or(false) || content["generationComplete"].as_bool().unwrap_or(false);
                if complete && ended.is_some() {
                    return Ok(tidy(&text));
                }
            }
            if ended.is_some_and(|e| e.elapsed() > TRANSCRIBE_TIMEOUT) {
                return Err("the transcription took too long".into());
            }
        }
    }
}

/// Install rustls' process-wide crypto provider once. The app links both ring
/// and aws-lc-rs (the connectors' reqwest), and then rustls can't choose and
/// panics on the first TLS handshake, killing the voice thread.
fn tls_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // Err means another provider is already installed, which is fine.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn the_setup_has_the_voice_tools_and_no_voice_detection() {
        let setup = Setup {
            system: "You are Yoru.".into(),
            tools: vec![json!({"name": "react", "description": "React.", "input_schema": {"$schema": "x", "type": "object", "properties": {"scene": {"type": "string"}}}})],
            voice: "Puck".into(),
            web: true,
            ..Setup::default()
        };
        let m = setup_message(&setup);
        assert_eq!(m["setup"]["model"], "models/gemini-3.8-live");
        assert_eq!(m.pointer("/setup/generationConfig/speechConfig/voiceConfig/prebuiltVoiceConfig/voiceName").unwrap(), "Puck");
        assert_eq!(m.pointer("/setup/realtimeInputConfig/automaticActivityDetection/disabled").unwrap(), true);
        let f = &m["setup"]["tools"][0]["functionDeclarations"][0];
        assert_eq!(f["name"], "react");
        assert_eq!(f["behavior"], "BLOCKING");
        assert!(f["parametersJsonSchema"].get("$schema").is_none());
        assert_eq!(m["setup"]["tools"][1], json!({"googleSearch": {}}));
    }

    #[test]
    fn downsamples_three_to_two_across_chunks() {
        let mut d = Downsampler::default();
        let constant = vec![1000i16; 7];
        let first = d.push(&constant);
        assert_eq!(first.len(), 4);
        let second = d.push(&[1000, 1000]);
        assert_eq!(second.len(), 2);
        // A steady level stays (the first sample eases in from silence).
        assert!(second.iter().all(|&s| s == 1000));
        assert_eq!(first[1], 1000);
    }

    struct Recorder {
        audio: Vec<i16>,
        calls: Vec<String>,
    }

    impl Turner for Recorder {
        fn audio(&mut self, samples: Vec<i16>) {
            self.audio.extend(samples);
        }
        fn call(&mut self, name: &str, arguments: &Value) -> Result<String, String> {
            self.calls.push(format!("{name} {arguments}"));
            Ok("it is 10:00".into())
        }
    }

    /// A Live server that hears an utterance, calls a tool, then answers.
    fn fake_google(listener: TcpListener) {
        let (stream, _) = listener.accept().unwrap();
        let mut ws = tungstenite::accept(stream).unwrap();
        let mut read = || -> Value {
            loop {
                match ws.read().unwrap() {
                    Message::Text(t) => return serde_json::from_str(&t).unwrap(),
                    _ => continue,
                }
            }
        };
        let setup = read();
        assert_eq!(setup["setup"]["model"], "models/gemini-3.8-live");
        drop(read);
        // Google sends JSON as binary frames.
        ws.send(Message::binary(json!({"setupComplete": {}}).to_string().into_bytes())).unwrap();
        let mut got = Vec::new();
        loop {
            let Message::Text(t) = ws.read().unwrap() else { continue };
            let m: Value = serde_json::from_str(&t).unwrap();
            let end = m.pointer("/realtimeInput/activityEnd").is_some();
            got.push(m);
            if end {
                break;
            }
        }
        assert!(got[0].pointer("/realtimeInput/activityStart").is_some());
        assert_eq!(got[1].pointer("/realtimeInput/audio/mimeType").unwrap(), "audio/pcm;rate=16000");
        ws.send(Message::text(json!({"serverContent": {"inputTranscription": {"text": "che ore "}}}).to_string())).unwrap();
        ws.send(Message::text(json!({"serverContent": {"inputTranscription": {"text": "sono?", "languageCode": "it-IT"}}}).to_string())).unwrap();
        ws.send(Message::text(json!({"toolCall": {"functionCalls": [{"id": "c1", "name": "get_time", "args": {}}]}}).to_string())).unwrap();
        let Message::Text(t) = ws.read().unwrap() else { panic!("no tool response") };
        let response: Value = serde_json::from_str(&t).unwrap();
        assert_eq!(response.pointer("/toolResponse/functionResponses/0/id").unwrap(), "c1");
        assert_eq!(response.pointer("/toolResponse/functionResponses/0/response/result").unwrap(), "it is 10:00");
        let pcm: Vec<u8> = [1i16, 2, 3].iter().flat_map(|s| s.to_le_bytes()).collect();
        let data = base64::engine::general_purpose::STANDARD.encode(pcm);
        ws.send(Message::text(
            json!({"serverContent": {"modelTurn": {"parts": [{"inlineData": {"mimeType": "audio/pcm;rate=24000", "data": data}}]}, "outputTranscription": {"text": "Sono le "}}}).to_string(),
        ))
        .unwrap();
        ws.send(Message::text(json!({"serverContent": {"outputTranscription": {"text": "dieci."}}}).to_string())).unwrap();
        ws.send(Message::text(json!({"serverContent": {"generationComplete": true}}).to_string())).unwrap();
        ws.send(Message::text(json!({"serverContent": {"turnComplete": true}}).to_string())).unwrap();
        let _ = ws.read();
    }

    #[test]
    fn a_transcription_asks_for_the_languages_and_joins_the_pieces() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            let mut read = || loop {
                if let Message::Text(t) = ws.read().unwrap() {
                    return serde_json::from_str::<Value>(&t).unwrap();
                }
            };
            let setup = read();
            assert_eq!(setup["setup"]["model"], "models/gemini-3.5-transcribe-live");
            assert_eq!(setup["setup"]["inputAudioTranscription"]["languageCodes"], json!(["it-IT"]));
            assert_eq!(setup["setup"]["inputAudioTranscription"]["customVocabulary"], json!(["Yoru"]));
            drop(read);
            ws.send(Message::text(json!({"setupComplete": {}}).to_string())).unwrap();
            loop {
                let Message::Text(t) = ws.read().unwrap() else { continue };
                if serde_json::from_str::<Value>(&t).unwrap().pointer("/realtimeInput/activityEnd").is_some() {
                    break;
                }
            }
            ws.send(Message::text(json!({"serverContent": {"interimInputTranscription": {"text": "ciao yo"}}}).to_string())).unwrap();
            ws.send(Message::text(json!({"serverContent": {"inputTranscription": {"text": "Ciao "}}}).to_string())).unwrap();
            ws.send(Message::text(json!({"serverContent": {"inputTranscription": {"text": " Yoru."}}}).to_string())).unwrap();
            ws.send(Message::text(json!({"serverContent": {"turnComplete": true}}).to_string())).unwrap();
            let _ = ws.read();
        });
        let accepted = thread::spawn(move || transcribe_at(&url, TRANSCRIBE_MODEL, &[0; 4000], &["it-IT"], &["Yoru"]));
        server.join().unwrap();
        assert_eq!(accepted.join().unwrap().unwrap(), "Ciao Yoru.");
    }

    #[test]
    fn a_turn_hears_calls_a_tool_and_answers() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || fake_google(listener));
        let mut session = Session::open_at(&url, &Setup { voice: "Puck".into(), ..Setup::default() }).unwrap();
        let mut recorder = Recorder { audio: Vec::new(), calls: Vec::new() };
        let turn = session.turn(&vec![0i16; 20_000], &mut recorder).unwrap();
        assert_eq!(turn.heard, "che ore sono?");
        assert_eq!(turn.language.as_deref(), Some("it"));
        assert_eq!(turn.said, "Sono le dieci.");
        assert_eq!(turn.samples, 3);
        assert_eq!(recorder.audio, vec![1, 2, 3]);
        assert_eq!(recorder.calls, vec!["get_time {}"]);
        drop(session);
        server.join().unwrap();
    }

    /// The TLS handshake must not panic when the build links both rustls
    /// providers (`--features connectors`): a fake key gets a refusal back.
    #[test]
    #[ignore = "talks to Google"]
    fn the_tls_handshake_picks_a_provider() {
        let setup = Setup { system: String::new(), tools: vec![], voice: DEFAULT_VOICE.into(), web: false, model: String::new() };
        let error = Session::open_at(&format!("{URL}?key=not-a-key"), &setup).err().expect("a fake key is refused");
        assert!(!error.contains("could not reach"), "{error}");
    }

    /// Against Google, with the key in Settings: `LIVE_WAV` is a 16 kHz mono
    /// WAV of a question (`say -v Alice … -o q.wav --data-format=LEI16@16000`).
    #[test]
    #[ignore = "talks to Google"]
    fn google_answers_for_real() {
        let key = crate::cloud::GOOGLE.api_key().expect("a Google key");
        let wav = std::fs::read(std::env::var("LIVE_WAV").expect("LIVE_WAV")).unwrap();
        let start = wav.windows(4).position(|w| w == b"data").unwrap() + 8;
        let samples: Vec<i16> = wav[start..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        let setup = Setup {
            system: "You are Yoru, an AI assistant. Answer in one short sentence. To know the time, call get_time.".into(),
            tools: vec![json!({"name": "get_time", "description": "The time now.", "input_schema": {"type": "object", "properties": {}}})],
            voice: DEFAULT_VOICE.into(),
            web: true,
            model: std::env::var("LIVE_MODEL").unwrap_or_default(),
        };
        let opened = std::time::Instant::now();
        let mut session = Session::open(&key, &setup).unwrap();
        eprintln!("open: {} ms", opened.elapsed().as_millis());
        let asked = std::time::Instant::now();
        struct Timed(Recorder, std::time::Instant, Option<u128>);
        impl Turner for Timed {
            fn audio(&mut self, samples: Vec<i16>) {
                self.2.get_or_insert(self.1.elapsed().as_millis());
                self.0.audio(samples)
            }
            fn call(&mut self, name: &str, arguments: &Value) -> Result<String, String> {
                self.0.call(name, arguments)
            }
        }
        let mut t = Timed(Recorder { audio: Vec::new(), calls: Vec::new() }, asked, None);
        // As the board sends it: 64 ms frames in real time, timed from the end.
        let turn = if std::env::var("LIVE_STREAM").is_ok() {
            session.begin().unwrap();
            for frame in samples.chunks(1024) {
                session.audio(frame).unwrap();
                thread::sleep(Duration::from_millis(64));
            }
            t.1 = std::time::Instant::now();
            session.finish(&mut t).unwrap()
        } else {
            session.turn(&samples, &mut t).unwrap()
        };
        eprintln!("first sound: {:?} ms, turn: {} ms", t.2, asked.elapsed().as_millis());
        eprintln!("{turn:?}\ncalls: {:?}", t.0.calls);
        let out = std::env::var("LIVE_OUT").ok();
        if let Some(out) = out {
            let pcm: Vec<u8> = Downsampler::default().push(&t.0.audio).iter().flat_map(|s| s.to_le_bytes()).collect();
            std::fs::write(out, pcm).unwrap();
        }
        assert!(turn.samples > 0, "no speech");
        assert!(!turn.said.is_empty());
    }

    /// `LIVE_WAV=q.wav cargo test -p yoru-core google_transcribes_for_real -- --ignored --nocapture`
    #[test]
    #[ignore = "talks to Google"]
    fn google_transcribes_for_real() {
        let key = crate::cloud::GOOGLE.api_key().expect("a Google key");
        for path in std::env::var("LIVE_WAV").expect("LIVE_WAV").split(',') {
            let wav = std::fs::read(path).unwrap();
            let start = wav.windows(4).position(|w| w == b"data").unwrap() + 8;
            let samples: Vec<i16> = wav[start..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
            let asked = std::time::Instant::now();
            let text = transcribe(&key, &samples, &["it-IT", "en-US"], &["Yoru"]);
            eprintln!("{path}: {} ms: {text:?}", asked.elapsed().as_millis());
        }
    }

    /// Streams a clip at speaking pace and prints when each piece of text came.
    /// `LIVE_WAV=q.wav cargo test -p yoru-core google_writes_while_speaking -- --ignored --nocapture`
    #[test]
    #[ignore = "talks to Google"]
    fn google_writes_while_speaking() {
        let key = crate::cloud::GOOGLE.api_key().expect("a Google key");
        let wav = std::fs::read(std::env::var("LIVE_WAV").expect("LIVE_WAV")).unwrap();
        let start = wav.windows(4).position(|w| w == b"data").unwrap() + 8;
        let samples: Vec<i16> = wav[start..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        let began = Instant::now();
        let scribe = Scribe::start(&key, &["it-IT", "en-US"], &["Yoru"], move |t| eprintln!("{:>5} ms: {t}", began.elapsed().as_millis()));
        for chunk in samples.chunks(1600) {
            scribe.audio(chunk);
            thread::sleep(Duration::from_millis(100));
        }
        eprintln!("{:>5} ms: spoken", began.elapsed().as_millis());
        let text = scribe.finish();
        eprintln!("{:>5} ms: final {text:?}", began.elapsed().as_millis());
        assert!(text.is_ok_and(|t| !t.is_empty()));
    }

    #[test]
    fn a_refused_key_reads_plainly() {
        assert!(closed("API key not valid. Please pass a valid API key.").contains("refused the API key"));
        assert!(is_limit(&closed("Resource has been exhausted (e.g. check quota).")));
        assert!(is_limit(&closed("You exceeded your current quota, please check your plan and billing details.")));
    }
}

