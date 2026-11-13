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
/// Google's prebuilt voices that sound like a man, and how Google describes
/// them. Yoru is male (docs/CONCEPT.md in the firmware's repository).
pub const VOICES: &[(&str, &str)] = &[
    ("Puck", "upbeat"),
    ("Fenrir", "excitable"),
    ("Charon", "informative"),
    ("Orus", "firm"),
    ("Achird", "friendly"),
    ("Sadachbia", "lively"),
    ("Umbriel", "easy-going"),
    ("Iapetus", "clear"),
    ("Algieba", "smooth"),
    ("Enceladus", "breathy"),
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
        let (socket, _) = tungstenite::connect(url).map_err(|e| match e {
            tungstenite::Error::Http(response) => {
                let body = response.body().as_deref().map(String::from_utf8_lossy).unwrap_or_default();
                format!("Gemini Live refused the connection ({}): {body}", response.status())
            }
            e => format!("could not reach Gemini Live: {e}"),
        })?;
        let mut session = Self { socket, going: false };
        session.set_timeout(READ_TIMEOUT);
        session.send(&setup_message(setup))?;
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
        loop {
            let message = self.socket.read().map_err(|e| match e {
                tungstenite::Error::Io(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => "Gemini Live stopped answering".to_string(),
                e => format!("Gemini Live: {e}"),
            })?;
            let bytes = match message {
                Message::Text(t) => t.as_bytes().to_vec(),
                Message::Binary(b) => b.to_vec(),
                Message::Close(frame) => {
                    self.going = true;
                    return Err(closed(frame.as_ref().map(|f| f.reason.as_str()).unwrap_or_default()));
                }
                _ => continue,
            };
            return serde_json::from_slice(&bytes).map_err(|e| format!("Gemini Live sent something unreadable: {e}"));
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
            system: "You are Yoru, a small desk companion. Answer in one short sentence. To know the time, call get_time.".into(),
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

    #[test]
    fn a_refused_key_reads_plainly() {
        assert!(closed("API key not valid. Please pass a valid API key.").contains("refused the API key"));
        assert!(is_limit(&closed("Resource has been exhausted (e.g. check quota).")));
        assert!(is_limit(&closed("You exceeded your current quota, please check your plan and billing details.")));
    }
}
