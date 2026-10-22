//! The host's half of voice: what the board says after its wake word.
//!
//! The board streams an utterance on `audio_up` between an `utterance_start`
//! and an `utterance_end` notification (see `docs/protocol.md`). [`Capture`]
//! puts the frames back together, filling any it lost with silence so the
//! timing stays right, and the bridge's voice thread takes each finished
//! [`Utterance`]: it keeps a WAV copy when asked, transcribes it with
//! [`Stt`] when there's speech, hands the words to Yoru the assistant
//! ([`crate::assistant`]) when it has a key, else acts on them with the local
//! language model ([`Agent`]), or the rules of [`intents`] without one or
//! when it fails, and answers with [`Tts`] through the board's speaker
//! ([`Speaker`]), or puts the eyes back to idle. When something fails, or
//! Whisper heard no words, the board shows `error` (a red ring and a short
//! sound). After a spoken answer the board listens again for a few seconds
//! without the wake word ([`VoiceConfig::follow_up`]): longer after a
//! question, briefly after a plain answer, and not after a few plain answers
//! in a row ([`FollowUp`]). What it hears then may not be for Yoru: a
//! closing word ("grazie", "basta": a fixed list, or EmbeddingGemma with
//! [`VoiceConfig::intent`]) or words Whisper isn't sure of end the
//! conversation, and the assistant stays silent for the rest that isn't. The
//! wake word said over the answer stops it (barge-in, on the board). With
//! [`VoiceConfig::turn`], Smart Turn judges at each pause whether the
//! sentence is finished ([`turns`]).
//!
//! Speech goes to the board on `audio_down`, paced in real time a little
//! ahead of the speaker, a sentence at a time: the first plays while the
//! next is synthesized. The board says `playback_end` once it has played it.
//! The answer is streamed ([`crate::stream`]): each sentence is spoken as
//! soon as the model has written it (`Aloud`), and a voice that streams
//! (ElevenLabs) is heard before it has made the whole sentence. While the
//! model goes off to call a tool the board stops speaking and thinks; while
//! the next sentence is late, silence keeps the board's stream open.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use chrono::Local;
use serde::Serialize;
use serde_json::{Value, json};

use crate::agent::{Action, Agent, Toolbox};
use crate::intent::{IntentModel, Meaning};
use crate::turn::{self, SmartTurn};
use crate::bridge::{BridgeEvent, EventSink};
use crate::intents::{self, Context};
use crate::link::{Link, Tool};
use crate::profile::BoardKind;
use crate::protocol::Channel;
use crate::stream::Delta;
use crate::stt::Stt;
use crate::timers::Timers;
use crate::tts::{self, Tts};

/// The board's audio: 16 kHz, mono, s16le.
pub const SAMPLE_RATE: u32 = 16_000;
/// Bytes before the PCM in each `audio_up` frame: utterance id, flags, sequence (u16 LE).
const HEADER: usize = 4;
/// A gap longer than this is a broken stream, not a few lost frames.
const MAX_GAP_FRAMES: u16 = 64;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Utterance {
    /// The board's id for it; wraps at 256.
    pub id: u8,
    /// What started it: `wake` (the wake word) or `host` (`voice/listen`).
    pub trigger: String,
    /// Why it ended: `end_of_speech`, `no_speech`, `max_length`, `host` or `muted`.
    pub reason: String,
    /// The board's VAD heard speech in it.
    pub speech: bool,
    /// Frames that never arrived (the board couldn't send them, or they came
    /// in corrupt), each replaced by silence.
    pub lost_frames: u32,
    #[serde(skip)]
    pub samples: Vec<i16>,
}

impl Utterance {
    pub fn duration_ms(&self) -> u64 {
        self.samples.len() as u64 * 1000 / SAMPLE_RATE as u64
    }

    /// Peak level in dBFS; `None` for silence.
    pub fn peak_db(&self) -> Option<f64> {
        let peak = self.samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
        (peak > 0).then(|| 20.0 * (peak as f64 / 32768.0).log10())
    }

    /// The samples as a 16 kHz mono 16-bit WAV file.
    pub fn to_wav(&self) -> Vec<u8> {
        wav(&self.samples, SAMPLE_RATE)
    }
}

/// A PCM WAV file: RIFF header, then the samples little-endian.
pub fn wav(samples: &[i16], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

struct Building {
    id: u8,
    trigger: String,
    next_seq: u16,
    frame_samples: usize,
    lost: u32,
    samples: Vec<i16>,
}

/// Puts an utterance back together from the board's notifications and frames.
#[derive(Default)]
pub struct Capture {
    current: Option<Building>,
}

impl Capture {
    /// `utterance_start`. One still open is dropped: its end got lost.
    pub fn start(&mut self, params: &Value) {
        let Some(id) = params["id"].as_u64() else {
            return;
        };
        self.current = Some(Building {
            id: id as u8,
            trigger: params["trigger"].as_str().unwrap_or("wake").to_string(),
            next_seq: 0,
            frame_samples: 0,
            lost: 0,
            samples: Vec::with_capacity(SAMPLE_RATE as usize * 4),
        });
    }

    /// One `audio_up` frame. Frames of another utterance are ignored.
    pub fn audio(&mut self, payload: &[u8]) {
        let Some(b) = self.current.as_mut() else {
            return;
        };
        if payload.len() < HEADER || payload[0] != b.id || !(payload.len() - HEADER).is_multiple_of(2) {
            return;
        }
        let seq = u16::from_le_bytes([payload[2], payload[3]]);
        let pcm = &payload[HEADER..];
        let gap = seq.wrapping_sub(b.next_seq);
        if gap >= MAX_GAP_FRAMES {
            // A repeat or something from long ago.
            return;
        }
        let frame_samples = pcm.len() / 2;
        b.samples.resize(b.samples.len() + gap as usize * frame_samples.max(b.frame_samples), 0);
        b.lost += gap as u32;
        b.frame_samples = frame_samples;
        b.samples.extend(pcm.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)));
        b.next_seq = seq.wrapping_add(1);
    }

    /// The samples of utterance `id` so far, while it's the one being built.
    pub fn so_far(&self, id: u8) -> Option<Vec<i16>> {
        self.current.as_ref().filter(|b| b.id == id).map(|b| b.samples.clone())
    }

    /// `utterance_end`: the finished utterance, if it's the one being built.
    pub fn end(&mut self, params: &Value) -> Option<Utterance> {
        let id = params["id"].as_u64()? as u8;
        if self.current.as_ref()?.id != id {
            return None;
        }
        let mut b = self.current.take()?;
        // Frames lost at the end, which no later frame revealed.
        if let Some(frames) = params["frames"].as_u64() {
            let missing = (frames as u16).wrapping_sub(b.next_seq);
            if missing < MAX_GAP_FRAMES {
                b.samples.resize(b.samples.len() + missing as usize * b.frame_samples, 0);
                b.lost += missing as u32;
            }
        }
        Some(Utterance {
            id,
            trigger: b.trigger,
            reason: params["reason"].as_str().unwrap_or("unknown").to_string(),
            speech: params["speech"].as_bool().unwrap_or(true),
            lost_frames: b.lost,
            samples: b.samples,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct VoiceConfig {
    /// Keep each utterance as a WAV file here (a debugging aid; nothing is
    /// written without it).
    pub dump_dir: Option<PathBuf>,
    /// Transcribe what the board hears. Shared: the sidecar outlives a
    /// reconnect.
    pub stt: Option<Arc<Stt>>,
    /// Answer out loud through the board's speaker; without it, answers
    /// are only reported ([`BridgeEvent::Reply`]).
    pub tts: Option<Arc<Tts>>,
    /// Understand what was said with a local language model; without it,
    /// with the rules of [`intents`].
    pub agent: Option<Arc<Agent>>,
    /// After a spoken answer, listen for a few seconds more without the
    /// wake word.
    pub follow_up: bool,
    /// Tell a goodbye in a follow-up with EmbeddingGemma; without it, with a
    /// fixed list of phrases.
    pub intent: Option<Arc<IntentModel>>,
    /// Judge at each pause whether the sentence is finished with Smart
    /// Turn; without it, the board's own pause ends it.
    pub turn: Option<Arc<SmartTurn>>,
    /// The language every answer is in (`it`, `en`), the one picked in the
    /// app; `None` answers in the one heard.
    pub language: Option<String>,
}

/// Whisper's output for silence or noise rather than words.
fn is_non_speech(text: &str) -> bool {
    let t = text.trim();
    t.is_empty() || (t.starts_with('[') && t.ends_with(']')) || (t.starts_with('(') && t.ends_with(')'))
}

/// Where WAV dumps go by default: `voice/` in the data folder.
pub fn default_dump_dir() -> Option<PathBuf> {
    crate::paths::data_dir().map(|d| d.join("voice"))
}

const STATE_TIMEOUT: Duration = Duration::from_secs(1);

/// What the eyes show next. `error` needs firmware 1.0; an older board
/// just goes back to idle.
fn set_state(link: &Link, state: &str) {
    if link.call("voice/state", json!({"state": state}), STATE_TIMEOUT).is_err() && state != "idle" {
        let _ = link.call("voice/state", json!({"state": "idle"}), STATE_TIMEOUT);
    }
}

/// The board drops a `thinking` nobody renews within 30 s (a host that went
/// away); a slow model (Ollama) can take longer, so it's renewed this often.
const THINKING_EVERY: Duration = Duration::from_secs(10);

/// Renews the board's `thinking` until stopped or dropped.
struct StillThinking {
    done: Arc<(Mutex<bool>, Condvar)>,
    thread: Option<thread::JoinHandle<()>>,
}

impl StillThinking {
    fn start(session: &Session) -> Self {
        let done = Arc::new((Mutex::new(false), Condvar::new()));
        let (link, state) = (session.link.clone(), session.voice_state.clone());
        let thread = {
            let done = done.clone();
            thread::Builder::new().name("yoru-thinking".into()).spawn(move || loop {
                let (lock, cv) = &*done;
                let over = cv.wait_timeout_while(lock.lock().unwrap(), THINKING_EVERY, |over| !*over).unwrap().0;
                if *over {
                    return;
                }
                drop(over);
                // Only while it still is: not over the wake word's listening.
                if *state.lock().unwrap() != "thinking" {
                    continue;
                }
                let Some(link) = link.upgrade() else { return };
                let _ = link.call("voice/state", json!({"state": "thinking"}), STATE_TIMEOUT);
            })
        };
        Self { done, thread: thread.ok() }
    }

    /// Before the answer plays, so a renewal never lands over `speaking`.
    fn stop(&mut self) {
        *self.done.0.lock().unwrap() = true;
        self.done.1.notify_all();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for StillThinking {
    fn drop(&mut self) {
        self.stop();
    }
}

const TOOL_TIMEOUT: Duration = Duration::from_secs(2);

/// What the voice thread shares with the bridge's session.
pub(crate) struct Session {
    pub link: Weak<Link>,
    pub speaker: Arc<Speaker>,
    /// Set after a tool changed faces or rotation: the bridge reads them back.
    pub settings_changed: Arc<AtomicBool>,
    /// The board's tools for the agent, listed once per connection.
    pub tools: Mutex<Option<Vec<Tool>>>,
    /// Which kind of board, once its `hello` said: its body in the agent's
    /// prompt, and its tools kept on disk.
    pub kind: Arc<Mutex<BoardKind>>,
    /// The board's id, once its `hello` said: the timers it sets ring on it.
    pub board: Arc<Mutex<Option<String>>>,
    /// The conversation is over: the board gives up the voice floor.
    pub conversation_over: Box<dyn Fn() + Send + Sync>,
    /// The host's timers, which the voice sets and silences.
    pub timers: Arc<Timers>,
    /// Yoru the assistant, which answers when it can.
    pub assistant: Option<Arc<crate::Assistant>>,
    /// Where the assistant's actions are allowed or denied.
    pub confirmations: Arc<crate::assistant::Confirmations>,
    /// What the board's eyes show, from its `voice_state`.
    pub voice_state: Arc<Mutex<String>>,
}

/// What the link's reader hands the pipeline.
pub enum Hearing {
    /// The board started streaming (`wake`, `host` or `follow_up`): the
    /// language model can read its prompt meanwhile.
    Started { trigger: String },
    Utterance(Utterance),
    /// The board started streaming while another board had the voice
    /// floor: it's told to stop and go back to idle.
    Refused,
}

/// The pipeline: handles each finished utterance, on its own thread so the
/// link's reader never waits on it, until the sender is dropped. The link is
/// weak because the link's reader holds that sender.
pub(crate) fn run(config: &Mutex<VoiceConfig>, session: &Session, sink: &EventSink, rx: Receiver<Hearing>) {
    let mut chain = FollowUp::default();
    for heard in rx {
        let config = config.lock().unwrap().clone();
        let utterance = match heard {
            Hearing::Utterance(u) => u,
            Hearing::Refused => {
                if let Some(link) = session.link.upgrade() {
                    let _ = link.call("voice/stop", json!({}), STATE_TIMEOUT);
                    set_state(&link, "idle");
                }
                sink(BridgeEvent::BoardLog { line: "host: another board is answering, this one stopped listening".into() });
                continue;
            }
            Hearing::Started { .. } => {
                if let (Some(agent), Some(link)) = (&config.agent, session.link.upgrade())
                    && let Err(e) = agent.prime(&BoardToolbox { link: &link, session, language: "en" })
                {
                    sink(BridgeEvent::VoiceError { message: e.to_string() });
                }
                continue;
            }
        };
        let wav = config.dump_dir.as_deref().and_then(|dir| match dump(dir, &utterance) {
            Ok(path) => Some(path),
            Err(e) => {
                sink(BridgeEvent::BoardLog { line: format!("host: could not save the utterance: {e}") });
                None
            }
        });
        sink(BridgeEvent::Utterance {
            duration_ms: utterance.duration_ms(),
            peak_db: utterance.peak_db(),
            wav: wav.map(|p| p.display().to_string()),
            utterance: utterance.clone(),
        });
        if !utterance.speech {
            (session.conversation_over)();
            continue;
        }
        let Some(link) = session.link.upgrade() else { break };
        // The board shows thinking meanwhile, however long the model takes.
        let mut thinking = StillThinking::start(session);
        let follow_up = utterance.trigger == "follow_up";
        if !follow_up {
            chain = FollowUp::default();
        }
        let transcript = match &config.stt {
            // Voice is off or its model isn't usable: say so rather than drop the words.
            None => {
                sink(BridgeEvent::VoiceError { message: "nothing transcribes what Yoru hears: turn voice on and pick a speech-to-text model".into() });
                Err(if follow_up { "idle" } else { "error" })
            }
            Some(stt) => match stt.transcribe(&utterance.samples) {
                Ok(t) if is_non_speech(&t.text) => {
                    sink(BridgeEvent::Transcript { id: utterance.id, transcript: None });
                    // Nothing said after an answer is no mistake.
                    Err(if follow_up { "idle" } else { "error" })
                }
                Ok(t) => {
                    sink(BridgeEvent::Transcript { id: utterance.id, transcript: Some(t.clone()) });
                    if follow_up && t.logprob < FOLLOW_UP_MIN_LOGPROB {
                        // Unsure words without the wake word: the room, not the user.
                        sink(BridgeEvent::BoardLog { line: format!("host: follow-up ignored, unsure transcript ({:.2})", t.logprob) });
                        Err("idle")
                    } else if follow_up && !alarm_near(&session.timers) && closes(&config, sink, &chain, &t.text) {
                        sink(BridgeEvent::BoardLog { line: "host: the user closed the conversation".into() });
                        Err("idle")
                    } else {
                        Ok(t)
                    }
                }
                Err(e) => {
                    sink(BridgeEvent::VoiceError { message: e.to_string() });
                    Err("error")
                }
            },
        };
        let next = match transcript {
            Ok(t) => respond(&link, session, &config, sink, utterance.id, &t.text, &t.language, follow_up, &mut thinking),
            Err(state) => Next::State(state),
        };
        thinking.stop();
        let over = match next {
            Next::State(state) => {
                set_state(&link, state);
                true
            }
            Next::Spoken { question } if config.follow_up => match chain.next(question) {
                Some(window) => match link.call("voice/listen", json!({"follow_up": true, "wait_ms": window.wait_ms, "pause_ms": window.pause_ms}), STATE_TIMEOUT) {
                    Ok(_) => false,
                    Err(e) => {
                        sink(BridgeEvent::BoardLog { line: format!("host: no follow-up: {e}") });
                        true
                    }
                },
                None => true,
            },
            Next::Spoken { .. } => true,
            Next::Interrupted => false,
        };
        if over {
            chain = FollowUp::default();
            (session.conversation_over)();
        }
    }
    // The board is gone mid-conversation.
    (session.conversation_over)();
}

/// What the board does once the host has answered.
enum Next {
    /// Show this state (`idle`, `error`): nothing was said.
    State(&'static str),
    /// The answer was played to the end, and the board went back to idle;
    /// `question`: it ended asking the user something.
    Spoken { question: bool },
    /// The wake word stopped it: the board is listening already.
    Interrupted,
}

/// Act on the words and answer.
#[allow(clippy::too_many_arguments)]
fn respond(link: &Link, session: &Session, config: &VoiceConfig, sink: &EventSink, id: u8, text: &str, language: &str, follow_up: bool, thinking: &mut StillThinking) -> Next {
    let started = Instant::now();
    // "Stop" or "5 more minutes" for an alarm needs no language model: the rules, at once.
    let alarm = session.timers.is_ringing() || session.timers.dismissed_within(ALARM_FOLLOW_UP);
    let quick = alarm && intents::understand(text, language, &Context { alarm, ..Context::default() }).calls.iter().any(|(t, _)| matches!(t.as_str(), "control_timer" | "update_timer"));
    // The models answer in the language picked; the rules only understand the one heard.
    let heard = language;
    let language = config.language.as_deref().unwrap_or(heard);
    let tts = config.tts.as_deref();
    let (said, to_say) = mpsc::channel();
    thread::scope(|scope| {
        // The answer is spoken as the brain writes it, a sentence at a time.
        let talker = tts.map(|tts| scope.spawn(move || talk(link, session, tts, to_say, thinking)));
        let mut aloud = Aloud::new(said, language);
        let assistant = session.assistant.as_ref().filter(|a| !quick && a.is_ready());
        let answer = assistant.and_then(|a| match a.respond_aloud(text, Some(heard), crate::assistant::Channel::Voice, follow_up, &VoiceHost { link, session }, &mut |d| aloud.add(d)) {
            Ok(reply) => Some((reply.text, reply.actions.iter().map(|x| if x.ok { format!("{}: {}", x.tool, x.summary) } else { format!("{} failed: {}", x.tool, x.summary) }).collect(), true, "assistant", language)),
            // Its free limits ran out: say so and when, rather than a worse answer.
            Err(e) if a.limited().is_some() => {
                sink(BridgeEvent::VoiceError { message: e });
                a.limited().map(|limit| (crate::assistant::limit_line(&limit, language), Vec::new(), true, "assistant", language))
            }
            // Half an answer was already said: no second one over it.
            Err(e) if aloud.has_spoken() => {
                sink(BridgeEvent::VoiceError { message: e });
                Some((aloud.text().to_string(), Vec::new(), true, "assistant", language))
            }
            Err(e) => {
                sink(BridgeEvent::VoiceError { message: format!("{e}; answering without the assistant") });
                None
            }
        });
        let agent = config.agent.as_ref().filter(|_| !quick && answer.is_none());
        let answer = answer.or_else(|| {
            agent.and_then(|agent| match agent.respond_aloud(text, language, &BoardToolbox { link, session, language }, &mut |d| aloud.add(d)) {
                Ok(turn) => Some((turn.reply, turn.actions.iter().map(action_line).collect(), true, "llm", language)),
                Err(e) if aloud.has_spoken() => {
                    sink(BridgeEvent::VoiceError { message: e.to_string() });
                    Some((aloud.text().to_string(), Vec::new(), true, "llm", language))
                }
                Err(e) => {
                    sink(BridgeEvent::VoiceError { message: format!("{e}; answering with the rules instead") });
                    None
                }
            })
        });
        let (reply, actions, understood, by, language) = answer.unwrap_or_else(|| {
            let (reply, actions, understood) = rules(link, session, text, heard);
            (reply, actions, understood, "rules", heard)
        });
        // A reply that wasn't written as it came (the rules', "Done.", a
        // limit), or whose last words weren't: said whole now.
        if !aloud.finish() {
            aloud.say_all(&reply, language);
        }
        drop(aloud);
        let elapsed_ms = started.elapsed().as_millis() as u64;
        sink(BridgeEvent::Reply { id, text: reply.clone(), language: language.to_string(), actions, understood, by: by.into(), elapsed_ms });
        let Some(talker) = talker else { return Next::State("idle") };
        match talker.join().unwrap_or_else(|_| Err("the voice stopped".into())) {
            Ok(Some(spoken)) => {
                let next = match spoken.reason.as_str() {
                    "done" => Next::Spoken { question: asks(&spoken.text) },
                    "barge_in" => Next::Interrupted,
                    // Stopped or replaced by someone else: they say what's next.
                    _ => Next::State("idle"),
                };
                sink(BridgeEvent::Spoken { id, spoken });
                next
            }
            Ok(None) => Next::State("idle"),
            Err(e) => {
                sink(BridgeEvent::VoiceError { message: e });
                Next::State("error")
            }
        }
    })
}

/// What the voice is to say, in order.
pub(crate) enum Said {
    /// A sentence, and the language it's in.
    Sentence(String, String),
    /// The brain went off to call tools: the board stops speaking and
    /// shows it thinks until the next sentence.
    Pause,
}

/// Speak what comes on `said` until it ends, a stream on the board for each
/// stretch between pauses; the board thinks during a pause. The reply as
/// it was heard (`None`: nothing to say). Stops at a barge-in.
fn talk(link: &Link, session: &Session, tts: &Tts, said: Receiver<Said>, thinking: &mut StillThinking) -> Result<Option<Spoken>, String> {
    let said = Mutex::new(said);
    let mut whole: Option<Spoken> = None;
    loop {
        let first = match said.lock().unwrap().recv() {
            Ok(Said::Sentence(s, l)) => (s, l),
            Ok(Said::Pause) => continue,
            Err(_) => return Ok(whole),
        };
        thinking.stop();
        let (spoken, paused) = session.speaker.speak_segment(link, tts, first, &said)?;
        let spoken = match whole.take() {
            Some(before) => Spoken {
                text: format!("{} {}", before.text, spoken.text),
                first_audio_ms: before.first_audio_ms,
                played_ms: before.played_ms + spoken.played_ms,
                underruns: before.underruns + spoken.underruns,
                lost: before.lost + spoken.lost,
                reason: spoken.reason,
            },
            None => spoken,
        };
        let done = spoken.reason == "done";
        whole = Some(spoken);
        if !done {
            return Ok(whole);
        }
        if paused {
            set_state(link, "thinking");
            *thinking = StillThinking::start(session);
        }
    }
}

/// What a brain writes ([`Delta`]), made into sentences for [`talk`] as it
/// comes. Its thinking (`<think>…</think>`) is left out, and a round that
/// starts like a tool call written as text (`<tool_call>`, `{"name": …`) is
/// held back until it turns out to be words: then it's said at the end.
struct Aloud {
    said: Sender<Said>,
    language: String,
    /// The round's words so far, raw.
    round: String,
    /// Bytes of the round's [`visible`] words already said.
    sent: usize,
    /// All that was said.
    text: String,
    /// Something was said since the last tool call.
    fresh: bool,
}

impl Aloud {
    fn new(said: Sender<Said>, language: &str) -> Self {
        Self { said, language: language.to_string(), round: String::new(), sent: 0, text: String::new(), fresh: false }
    }

    fn add(&mut self, delta: Delta) {
        match delta {
            Delta::Text(t) => {
                self.round.push_str(t);
                self.say_sentences(false);
            }
            Delta::Tools => {
                // What came before the call is said, unless it was the call.
                if !held(&visible(&self.round, true)) {
                    self.say_sentences(true);
                }
                self.round.clear();
                self.sent = 0;
                self.fresh = false;
                let _ = self.said.send(Said::Pause);
            }
        }
    }

    /// The brain is done: the rest is said. Whether the last round said anything.
    fn finish(&mut self) -> bool {
        self.say_sentences(true);
        self.fresh
    }

    fn has_spoken(&self) -> bool {
        !self.text.is_empty()
    }

    fn text(&self) -> &str {
        &self.text
    }

    /// `text`, written all at once, in `language`.
    fn say_all(&mut self, text: &str, language: &str) {
        // Nothing to say ("…", the assistant staying silent) is not said.
        for s in tts::sentences(&tts::speakable(text, language)).into_iter().filter(|s| s.chars().any(char::is_alphanumeric)) {
            self.push(s, language.to_string());
        }
    }

    /// The round's whole sentences not said yet, or, when `all`, everything left.
    fn say_sentences(&mut self, all: bool) {
        let visible = visible(&self.round, all);
        if !all && held(&visible) {
            return;
        }
        while self.sent < visible.len() {
            let rest = &visible[self.sent..];
            let end = match tts::sentence_end(rest) {
                Some(end) => end,
                None if all => rest.len(),
                None => return,
            };
            self.sent += end;
            let sentence = tts::speakable(&rest[..end], &self.language);
            if sentence.chars().any(char::is_alphanumeric) {
                self.push(sentence, self.language.clone());
            }
        }
    }

    fn push(&mut self, sentence: String, language: String) {
        // The audio tags go to the voice, not to what was said.
        let plain = tts::without_audio_tags(&sentence);
        if !self.text.is_empty() && !plain.is_empty() {
            self.text.push(' ');
        }
        self.text.push_str(&plain);
        self.fresh = true;
        let _ = self.said.send(Said::Sentence(sentence, language));
    }
}

/// The words of `raw` a voice may say: no thinking (one not closed yet
/// hides the rest) and, unless the text is `done`, no tag still being
/// written at its end, which may open a thought. What it returns only
/// grows as `raw` does.
fn visible(raw: &str, done: bool) -> String {
    const OPEN: &str = "<think>";
    const CLOSE: &str = "</think>";
    let mut out = String::new();
    let mut rest = raw;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        match rest[start..].find(CLOSE) {
            Some(end) => rest = &rest[start + end + CLOSE.len()..],
            None => return out,
        }
    }
    out.push_str(rest);
    if !done && let Some(lt) = out.rfind('<').filter(|&lt| !out[lt..].contains('>')) {
        out.truncate(lt);
    }
    out
}

/// How long the board listens after an answer, and how long a pause ends
/// what's said then (`voice/listen`, firmware 0.11.6).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Window {
    wait_ms: u32,
    pause_ms: u32,
}

/// After a question the user needs time to think, and may stop mid-answer.
const AFTER_QUESTION: Window = Window { wait_ms: 8000, pause_ms: 700 };
/// After a plain answer, a short chance to add something.
const AFTER_ANSWER: Window = Window { wait_ms: 3500, pause_ms: 400 };
/// Plain answers in a row after which the board stops listening on its own.
const MAX_PLAIN_ANSWERS: u32 = 3;
/// Whisper's own line for a decoding that failed: a follow-up below it
/// was most likely not the user talking to Yoru.
const FOLLOW_UP_MIN_LOGPROB: f64 = -1.0;

/// The follow-ups of one conversation.
#[derive(Debug, Default)]
struct FollowUp {
    /// The last answer ended with a question.
    asked: bool,
    /// Plain answers since the wake word or the last question.
    plain: u32,
}

impl FollowUp {
    /// After an answer: how long to listen, or `None` to stop.
    fn next(&mut self, question: bool) -> Option<Window> {
        self.asked = question;
        if question {
            self.plain = 0;
            return Some(AFTER_QUESTION);
        }
        self.plain += 1;
        (self.plain < MAX_PLAIN_ANSWERS).then_some(AFTER_ANSWER)
    }
}

/// Whether what was said ends with a question.
fn asks(said: &str) -> bool {
    said.trim_end_matches(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '»' | '”' | ')')).ends_with(['?', '？'])
}

/// Whether a follow-up ends the conversation: EmbeddingGemma's say when
/// it's on and works, else the fixed phrases. An acknowledgement ("ok")
/// closes it after a plain answer; after a question it's the answer.
fn closes(config: &VoiceConfig, sink: &EventSink, chain: &FollowUp, text: &str) -> bool {
    if let Some(intent) = &config.intent {
        let started = Instant::now();
        match intent.classify(text) {
            Ok(meaning) => {
                sink(BridgeEvent::BoardLog { line: format!("host: follow-up means {meaning:?} ({} ms)", started.elapsed().as_millis()) });
                return meaning == Meaning::Close || (meaning == Meaning::Ack && !chain.asked);
            }
            Err(e) => sink(BridgeEvent::BoardLog { line: format!("host: intent model failed, using the phrases: {e}") }),
        }
    }
    !chain.asked && is_closer(text)
}

/// An alarm rings or was just silenced: "stop" and "basta" are about it.
fn alarm_near(timers: &Timers) -> bool {
    timers.is_ringing() || timers.dismissed_within(ALARM_FOLLOW_UP)
}

/// Words that end a conversation rather than go on with it, in Italian and
/// English: the whole utterance, not a part of it.
fn is_closer(text: &str) -> bool {
    const CLOSERS: &[&str] = &[
        "grazie", "grazie mille", "ok grazie", "okay grazie", "va bene grazie", "perfetto grazie", "ottimo grazie", "no grazie",
        "basta", "basta così", "basta cosi", "è tutto", "e tutto", "niente", "nient'altro", "nulla", "a posto", "a posto così",
        "a posto cosi", "tutto qui", "ok", "okay", "va bene", "perfetto", "ottimo", "ciao", "a dopo", "lascia stare", "lascia perdere",
        "fine", "stop", "thanks", "thank you", "thanks a lot", "ok thanks", "okay thanks", "no thanks", "that's all", "thats all",
        "that's it", "nothing", "never mind", "nevermind", "all good", "great thanks", "perfect thanks", "bye", "enough",
    ];
    let words: String = text.to_lowercase().chars().map(|c| if c.is_alphanumeric() || c == '\'' || c.is_whitespace() { c } else { ' ' }).collect();
    let words = words.split_whitespace().collect::<Vec<_>>().join(" ");
    let words = words.strip_prefix("yoru ").unwrap_or(&words);
    let words = words.strip_suffix(" yoru").unwrap_or(words);
    CLOSERS.contains(&words)
}

/// Words that start like a tool call written as text rather than an answer.
fn held(visible: &str) -> bool {
    visible.trim_start().starts_with(['<', '{'])
}

/// "set_face: left: rings", "set_face failed: …"
fn action_line(a: &Action) -> String {
    if a.ok { format!("{}: {}", a.tool, a.result) } else { format!("{} failed: {}", a.tool, a.result.trim_start_matches("error: ")) }
}

/// The M5 rules: the reply, the actions taken, and whether anything matched.
fn rules(link: &Link, session: &Session, text: &str, language: &str) -> (String, Vec<String>, bool) {
    let volume = || {
        let state = link.call_tool("get_state", json!({}), TOOL_TIMEOUT).ok()?;
        Some(state.structured_content?.pointer("/audio/volume")?.as_u64()? as u8)
    };
    let timers = &session.timers;
    let alarm = timers.is_ringing() || timers.dismissed_within(ALARM_FOLLOW_UP);
    let ctx = Context {
        volume: volume(),
        timers: timers.list(),
        alarm,
    };
    let plan = intents::understand(text, language, &ctx);
    let toolbox = BoardToolbox { link, session, language };
    let mut actions = Vec::new();
    for (tool, args) in &plan.calls {
        let result = toolbox.call(tool, args);
        let ok = result.is_ok();
        actions.push(action_line(&Action { tool: tool.clone(), arguments: args.clone(), result: result.unwrap_or_else(|e| e), ok }));
        if !ok {
            return (plan.failure, actions, plan.understood);
        }
    }
    (plan.reply, actions, plan.understood)
}

/// After the wake word silenced an alarm, "stop" is still about it this long.
const ALARM_FOLLOW_UP: Duration = Duration::from_secs(20);

/// Board tools the voice agent doesn't get: muted by voice, the board
/// couldn't be unmuted by voice; the sounds are toys, not worth a tool; and
/// the host's own (the assistant's cards, the inbox).
fn not_by_voice(name: &str) -> bool {
    matches!(name, "set_mic" | "play_sound") || BoardKind::hub_only(name)
}

/// The board's tools as the voice agent gets them: without those
/// [`not_by_voice`] keeps out, plus the host's timers ([`Timers::tools`]).
pub fn voice_tools(board: Vec<Tool>) -> Vec<Tool> {
    board.into_iter().filter(|t| !not_by_voice(&t.name)).chain(Timers::tools()).collect()
}

/// `get_state` without what only a developer wants (memory, link and UI
/// statistics): fewer tokens for the model to read.
pub fn trim_state(state: &Value) -> Value {
    let mut out = json!({"brightness": state["brightness"], "activity": state["activity"], "battery": state["battery"], "voice": {}, "audio": {"volume": state["audio"]["volume"]}});
    for key in ["wake_word", "wake_words", "muted"] {
        out["voice"][key] = state["voice"][key].clone();
    }
    out
}

/// The board's tools, run over the link, and the host's.
struct BoardToolbox<'a> {
    link: &'a Link,
    session: &'a Session,
    /// What a timer says when it ends in.
    language: &'a str,
}

impl Toolbox for BoardToolbox<'_> {
    fn kind(&self) -> BoardKind {
        *self.session.kind.lock().unwrap()
    }

    fn tools(&self) -> Vec<Tool> {
        let mut tools = self.session.tools.lock().unwrap();
        if let Some(tools) = &*tools {
            return tools.clone();
        }
        match self.link.list_tools(TOOL_TIMEOUT) {
            Ok(board) => {
                crate::link::save_tools_cache(self.kind(), &board);
                tools.insert(voice_tools(board)).clone()
            }
            // The board didn't answer in time (busy with the audio): its
            // tools as last seen, asked for again next time. Without any the
            // model would only have get_time, and call it over and over.
            Err(_) => {
                voice_tools(crate::link::load_tools_cache(self.kind()).unwrap_or_default())
            }
        }
    }

    fn call(&self, name: &str, arguments: &Value) -> Result<String, String> {
        let board = self.session.board.lock().unwrap().clone();
        if let Some(result) = self.session.timers.call_tool_from(name, arguments, self.language, board.as_deref()) {
            return result;
        }
        if not_by_voice(name) {
            return Err(format!("{name} can't be used by voice"));
        }
        let result = self.link.call_tool(name, arguments.clone(), TOOL_TIMEOUT).map_err(|e| e.to_string())?;
        if result.is_error {
            return Err(result.text());
        }
        if name == "set_setting" {
            self.session.settings_changed.store(true, Ordering::Relaxed);
        }
        Ok(match (name, &result.structured_content) {
            ("get_state", Some(state)) => trim_state(state).to_string(),
            _ => result.text(),
        })
    }
}

/// What the assistant does through the board the voice came from.
struct VoiceHost<'a> {
    link: &'a Link,
    session: &'a Session,
}

impl crate::assistant::Host for VoiceHost<'_> {
    fn board_tools(&self) -> Vec<Tool> {
        // The assistant has the timers itself.
        let board = match self.link.list_tools(TOOL_TIMEOUT) {
            Ok(tools) => tools,
            Err(_) => crate::link::load_tools_cache(*self.session.kind.lock().unwrap()).unwrap_or_default(),
        };
        board.into_iter().filter(|t| !not_by_voice(&t.name)).collect()
    }

    fn call_board(&self, name: &str, arguments: &Value) -> Result<String, String> {
        if not_by_voice(name) {
            return Err(format!("{name} can't be used by the assistant"));
        }
        let result = self.link.call_tool(name, arguments.clone(), TOOL_TIMEOUT).map_err(|e| e.to_string())?;
        match (result.is_error, name, &result.structured_content) {
            (true, ..) => Err(result.text()),
            (false, "get_state", Some(state)) => Ok(trim_state(state).to_string()),
            _ => Ok(result.text()),
        }
    }

    fn confirm(&self, title: &str, text: &str, detail: &str) -> Option<bool> {
        self.session.confirmations.ask(title, text, Some(detail), crate::assistant::CONFIRM_WAIT)
    }
}

/// Samples per `audio_down` frame: 64 ms, 2 KB.
const PLAY_FRAME: usize = 1024;
/// How far ahead of the speaker the host keeps the board's buffer.
const PLAY_LEAD: Duration = Duration::from_millis(500);
/// Sound the board has left when silence goes out to fill a wait: the
/// board ends a stream that goes dry for 1.5 s (`STARVED_MS`).
const KEEP_FED: Duration = Duration::from_millis(150);
/// Past the end of the audio, how long to wait for `playback_end`.
const PLAY_GRACE: Duration = Duration::from_secs(3);

/// How a reply went out.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Spoken {
    pub text: String,
    /// From the request to the first frame sent: synthesizing the first sentence.
    pub first_audio_ms: u64,
    /// What the board played, from its `playback_end`.
    pub played_ms: u64,
    /// Why it ended: `done`, `stopped`, `replaced`, `starved` or `barge_in`
    /// (the wake word was said over it).
    pub reason: String,
    /// Times the board ran out of audio mid-stream.
    pub underruns: u32,
    /// Frames the board never got.
    pub lost: u32,
}

/// Plays speech on the board, one reply at a time. Shared by the voice
/// pipeline and the hub (`host/say`); the link's reader hands it the board's
/// `playback_end` notifications.
#[derive(Default)]
pub struct Speaker {
    /// Held while a reply plays; the id of the last stream.
    playing: Mutex<u8>,
    ended: Mutex<Option<Value>>,
    ended_cv: Condvar,
}

impl Speaker {
    /// `playback_end` from the board.
    pub fn playback_end(&self, params: Value) {
        *self.ended.lock().unwrap() = Some(params);
        self.ended_cv.notify_all();
    }

    /// The board said stream `id` ended (it may do so before the last frame:
    /// stopped, barge-in).
    fn has_ended(&self, id: u8) -> bool {
        self.ended.lock().unwrap().as_ref().is_some_and(|p| p["id"].as_u64() == Some(id as u64))
    }

    /// Synthesize `text` in `language` and play it, a sentence at a time.
    /// Returns once the board has played it.
    pub fn speak(&self, link: &Link, tts: &Tts, text: &str, language: &str) -> Result<Spoken, String> {
        let mut sentences = tts::sentences(&tts::speakable(text, language)).into_iter();
        let Some(first) = sentences.next() else { return Ok(Spoken { text: text.to_string(), first_audio_ms: 0, played_ms: 0, reason: "done".into(), underruns: 0, lost: 0 }) };
        let (tx, rx) = mpsc::channel();
        for s in sentences {
            let _ = tx.send(Said::Sentence(s, language.to_string()));
        }
        drop(tx);
        let (spoken, _) = self.speak_segment(link, tts, (first, language.to_string()), &Mutex::new(rx))?;
        Ok(Spoken { text: text.to_string(), ..spoken })
    }

    /// Play `first` and the sentences after it in `said` as one stream on the
    /// board, each synthesized while the one before plays, until a
    /// [`Said::Pause`] (`true`) or the end. Returns once the board played it.
    fn speak_segment(&self, link: &Link, tts: &Tts, first: (String, String), said: &Mutex<Receiver<Said>>) -> Result<(Spoken, bool), String> {
        let started = Instant::now();
        let (tx, rx) = mpsc::sync_channel::<Result<Vec<i16>, String>>(8);
        thread::scope(|scope| {
            let synth = scope.spawn(move || {
                let (mut text, mut next) = (String::new(), Some(first));
                loop {
                    let (sentence, language) = match next.take() {
                        Some(s) => s,
                        None => match said.lock().unwrap().recv() {
                            Ok(Said::Sentence(s, l)) => (s, l),
                            Ok(Said::Pause) => return (text, true),
                            Err(_) => return (text, false),
                        },
                    };
                    let mut gone = false;
                    let audio = tts.synthesize_into(&sentence, &language, &mut |samples| gone |= tx.send(Ok(samples)).is_err());
                    let plain = tts::without_audio_tags(&sentence);
                    if !text.is_empty() && !plain.is_empty() {
                        text.push(' ');
                    }
                    text.push_str(&plain);
                    // The board stopped playing it: say no more.
                    if gone {
                        return (text, false);
                    }
                    if let Err(e) = audio {
                        let _ = tx.send(Err(e.to_string()));
                        return (text, false);
                    }
                }
            });
            let mut first_audio_ms = None;
            let played = self.play_from(link, rx, &mut || {
                first_audio_ms.get_or_insert(started.elapsed().as_millis() as u64);
            });
            let (text, paused) = synth.join().unwrap_or_default();
            Ok((Spoken { text, first_audio_ms: first_audio_ms.unwrap_or(0), ..played? }, paused))
        })
    }

    /// Stream audio to the board as it comes, paced in real time, then wait
    /// for the board to finish playing it.
    pub fn play(&self, link: &Link, audio: impl IntoIterator<Item = Result<Vec<i16>, String>>) -> Result<Spoken, String> {
        let (tx, rx) = mpsc::channel();
        for chunk in audio {
            let _ = tx.send(chunk);
        }
        drop(tx);
        self.play_from(link, rx, &mut || {})
    }

    /// [`Speaker::play`] from `audio`, a channel that may be slow to fill:
    /// while it waits, silence keeps the board's buffer from running dry,
    /// or the board would end the stream as starved. `first` is called with
    /// the first sound.
    fn play_from(&self, link: &Link, audio: Receiver<Result<Vec<i16>, String>>, first: &mut dyn FnMut()) -> Result<Spoken, String> {
        let mut id = self.playing.lock().unwrap();
        *id = id.wrapping_add(1);
        let id = *id;
        *self.ended.lock().unwrap() = None;
        let mut seq = 0u16;
        let mut pending: Vec<i16> = Vec::new();
        // Where the board's speaker is: `sent` samples are due at `clock + sent / rate`.
        let mut clock: Option<Instant> = None;
        let mut sent = 0u64;
        let mut failure = None;
        let mut send = |samples: &[i16], last: bool, clock: &mut Option<Instant>, sent: &mut u64| -> io::Result<()> {
            let start = *clock.get_or_insert_with(Instant::now);
            let due = |n: u64| start + Duration::from_micros(n * 1_000_000 / SAMPLE_RATE as u64);
            // Keep at most PLAY_LEAD in the board's buffer.
            let now = Instant::now();
            if due(*sent) < now {
                // We fell behind (a slow sentence): the board played
                // silence meanwhile, so its clock moved on.
                *clock = Some(now - Duration::from_micros(*sent * 1_000_000 / SAMPLE_RATE as u64));
            } else if let Some(wait) = due(*sent).checked_duration_since(now + PLAY_LEAD) {
                thread::sleep(wait);
            }
            let mut frame = Vec::with_capacity(4 + samples.len() * 2);
            frame.extend_from_slice(&[id, u8::from(last)]);
            frame.extend_from_slice(&seq.to_le_bytes());
            for s in samples {
                frame.extend_from_slice(&s.to_le_bytes());
            }
            seq = seq.wrapping_add(1);
            *sent += samples.len() as u64;
            link.send(Channel::AudioDown, &frame)
        };
        let mut cut_short = false;
        loop {
            if self.has_ended(id) {
                cut_short = true;
                break;
            }
            // Once it plays, wait only until the board is about to run dry.
            let dry = clock.map(|c| c + Duration::from_micros(sent * 1_000_000 / SAMPLE_RATE as u64));
            let chunk = match dry {
                None => audio.recv().map_err(|_| RecvTimeoutError::Disconnected),
                Some(dry) => audio.recv_timeout(dry.saturating_duration_since(Instant::now()).saturating_sub(KEEP_FED)),
            };
            match chunk {
                Ok(Ok(samples)) => {
                    if !samples.is_empty() && clock.is_none() {
                        first();
                    }
                    pending.extend(samples);
                }
                Ok(Err(e)) => {
                    failure = Some(e);
                    break;
                }
                Err(RecvTimeoutError::Timeout) => {
                    // The next sentence is late: a pause, not an end.
                    pending.resize(pending.len().max(PLAY_FRAME), 0);
                    send(&pending, false, &mut clock, &mut sent).map_err(|e| e.to_string())?;
                    pending.clear();
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
            while pending.len() > PLAY_FRAME && !cut_short {
                let rest = pending.split_off(PLAY_FRAME);
                send(&pending, false, &mut clock, &mut sent).map_err(|e| e.to_string())?;
                pending = rest;
                cut_short = self.has_ended(id);
            }
            if cut_short {
                break;
            }
        }
        if !cut_short {
            send(&pending, true, &mut clock, &mut sent).map_err(|e| e.to_string())?;
        }
        let deadline = clock.unwrap_or_else(Instant::now) + Duration::from_micros(sent * 1_000_000 / SAMPLE_RATE as u64) + PLAY_GRACE;
        let mut ended = self.ended.lock().unwrap();
        loop {
            if let Some(p) = ended.as_ref().filter(|p| p["id"].as_u64() == Some(id as u64)) {
                if let Some(e) = failure {
                    return Err(e);
                }
                let n = |k: &str| p[k].as_u64().unwrap_or(0);
                return Ok(Spoken {
                    text: String::new(),
                    first_audio_ms: 0,
                    played_ms: n("ms"),
                    reason: p["reason"].as_str().unwrap_or("done").to_string(),
                    underruns: n("underruns") as u32,
                    lost: n("lost") as u32,
                });
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(failure.unwrap_or_else(|| "the board didn't say it had played the reply".into()));
            }
            ended = self.ended_cv.wait_timeout(ended, deadline - now).unwrap().0;
        }
    }
}

fn dump(dir: &Path, utterance: &Utterance) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}-{:03}.wav", Local::now().format("%Y%m%d-%H%M%S"), utterance.id));
    fs::write(&path, utterance.to_wav())?;
    Ok(path)
}

/// May this board go on listening? The voice floor, between boards.
pub type Gate = Box<dyn Fn() -> bool + Send>;

/// Keeps the capture and the pipeline's sender together for the link's reader.
pub struct Receiving {
    capture: Capture,
    pipeline: Sender<Hearing>,
    gate: Gate,
}

impl Receiving {
    pub fn new(pipeline: Sender<Hearing>) -> Self {
        Self::gated(pipeline, Box::new(|| true))
    }

    /// Each utterance asks `gate` first; one it refuses isn't captured.
    pub fn gated(pipeline: Sender<Hearing>, gate: Gate) -> Self {
        Self { capture: Capture::default(), pipeline, gate }
    }

    pub fn start(&mut self, params: &Value) {
        if !(self.gate)() {
            // Its frames find no utterance to join.
            self.capture = Capture::default();
            let _ = self.pipeline.send(Hearing::Refused);
            return;
        }
        self.capture.start(params);
        let trigger = params["trigger"].as_str().unwrap_or("wake").to_string();
        let _ = self.pipeline.send(Hearing::Started { trigger });
    }

    pub fn audio(&mut self, payload: &[u8]) {
        self.capture.audio(payload);
    }

    pub fn end(&mut self, params: &Value) {
        if let Some(u) = self.capture.end(params) {
            let _ = self.pipeline.send(Hearing::Utterance(u));
        }
    }

    pub fn so_far(&self, id: u8) -> Option<Vec<i16>> {
        self.capture.so_far(id)
    }
}

/// What the link's reader hands [`turns`].
pub enum TurnJob {
    /// An utterance started (not push-to-talk): the board is to wait longer.
    Start(u8),
    /// A pause in it, with what was heard so far.
    Pause(u8, Vec<i16>),
}

/// With Smart Turn the board ends an utterance after this long a pause
/// unless the host stops it sooner: long enough to think mid-sentence.
const TURN_FALLBACK_MS: u32 = 1500;

/// Smart Turn's half of a conversation, on its own thread so the link's
/// reader never waits on the model: at each pause, if the sentence sounds
/// finished, the board stops listening at once (`voice/stop`); otherwise it
/// waits up to [`TURN_FALLBACK_MS`] for more.
pub(crate) fn turns(config: &Mutex<VoiceConfig>, link: &std::sync::OnceLock<Weak<Link>>, sink: &EventSink, rx: Receiver<TurnJob>) {
    while let Ok(first) = rx.recv() {
        let Some(turn) = config.lock().unwrap().turn.clone() else { continue };
        let Some(link) = link.get().and_then(Weak::upgrade) else { continue };
        for job in latest(std::iter::once(first).chain(rx.try_iter()).collect()) {
            match job {
            TurnJob::Start(id) => {
                if let Err(e) = link.call("voice/pause", json!({"id": id, "pause_ms": TURN_FALLBACK_MS}), STATE_TIMEOUT) {
                    // A board before 0.11.7: its own pause ends the utterance.
                    sink(BridgeEvent::BoardLog { line: format!("host: Smart Turn can't hold the pause: {e}") });
                }
            }
            TurnJob::Pause(id, samples) => {
                let started = Instant::now();
                match turn.finished(&samples) {
                    Ok(p) => {
                        let done = p > turn::FINISHED;
                        let ms = started.elapsed().as_millis();
                        sink(BridgeEvent::BoardLog { line: format!("host: utterance {id} {} ({p:.2}, {ms} ms)", if done { "finished" } else { "goes on" }) });
                        if done {
                            let _ = link.call("voice/stop", json!({}), STATE_TIMEOUT);
                        }
                    }
                    Err(e) => sink(BridgeEvent::BoardLog { line: format!("host: Smart Turn failed: {e}") }),
                }
            }
            }
        }
    }
}

/// Of the jobs waiting, the last start and the last pause after it: an
/// older pause was overtaken by more words.
fn latest(jobs: Vec<TurnJob>) -> Vec<TurnJob> {
    let start = jobs.iter().rposition(|j| matches!(j, TurnJob::Start(_)));
    let pause = jobs.iter().rposition(|j| matches!(j, TurnJob::Pause(..))).filter(|&p| start.is_none_or(|s| p > s));
    jobs.into_iter().enumerate().filter(|(i, _)| Some(*i) == start || Some(*i) == pause).map(|(_, j)| j).collect()
}

/// For tests and tools: an `audio_up` frame as the board sends it.
pub fn audio_frame(id: u8, seq: u16, samples: &[i16]) -> Vec<u8> {
    let mut out = vec![id, 0];
    out.extend_from_slice(&seq.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod follow_up_tests {
    use super::*;

    #[test]
    fn listens_longer_after_a_question_and_stops_after_plain_answers() {
        let mut chain = FollowUp::default();
        assert_eq!(chain.next(true), Some(AFTER_QUESTION));
        assert_eq!(chain.next(false), Some(AFTER_ANSWER));
        assert_eq!(chain.next(false), Some(AFTER_ANSWER));
        assert_eq!(chain.next(false), None);
        assert_eq!(chain.next(true), Some(AFTER_QUESTION));
    }

    #[test]
    fn tells_a_question() {
        assert!(asks("Vuoi che lo aggiunga al calendario?"));
        assert!(asks("A che ora? "));
        assert!(asks("Shall I send it?\""));
        assert!(!asks("Timer impostato per dieci minuti."));
        assert!(!asks("Fatto"));
    }

    #[test]
    fn closers_are_the_whole_utterance() {
        assert!(is_closer("Grazie!"));
        assert!(is_closer(" Ok, grazie. "));
        assert!(is_closer("Basta così, Yoru."));
        assert!(is_closer("That's all."));
        assert!(!is_closer("Grazie, e domani che tempo fa?"));
        assert!(!is_closer("Niente di speciale per stasera?"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `deltas` make `Aloud` say, and whether its last round said anything.
    fn said(deltas: &[Delta]) -> (Vec<String>, bool) {
        let (tx, rx) = mpsc::channel();
        let mut aloud = Aloud::new(tx, "it");
        for d in deltas {
            aloud.add(*d);
        }
        let fresh = aloud.finish();
        drop(aloud);
        let lines = rx
            .into_iter()
            .map(|s| match s {
                Said::Sentence(s, _) => s,
                Said::Pause => "|".into(),
            })
            .collect();
        (lines, fresh)
    }

    #[test]
    fn sentences_are_said_as_they_are_written() {
        let (tx, rx) = mpsc::channel();
        let mut aloud = Aloud::new(tx, "it");
        aloud.add(Delta::Text("La protesta è sospesa"));
        assert!(rx.try_recv().is_err());
        aloud.add(Delta::Text(" da oggi. Il confronto"));
        assert!(matches!(rx.try_recv(), Ok(Said::Sentence(s, _)) if s == "La protesta è sospesa da oggi."));
        aloud.add(Delta::Text(" è il 27."));
        assert!(rx.try_recv().is_err());
        assert!(aloud.finish());
        assert!(matches!(rx.try_recv(), Ok(Said::Sentence(s, _)) if s == "Il confronto è il 27."));
    }

    #[test]
    fn thinking_is_not_said() {
        let deltas = [Delta::Text("<thi"), Delta::Text("nk>Devo cercare. Poi rispondo.</th"), Delta::Text("ink>\n\nEcco la risposta di oggi.")];
        assert_eq!(said(&deltas).0, ["Ecco la risposta di oggi."]);
        assert!(said(&[Delta::Text("Ok, ci penso su. <think>E se invece")]).0 == ["Ok, ci penso su."]);
    }

    #[test]
    fn a_call_written_as_text_is_not_said() {
        let call = "<tool_call><function=set_face><parameter=face>rings</parameter></function></tool_call>";
        let (lines, fresh) = said(&[Delta::Text(call), Delta::Tools]);
        assert_eq!((lines, fresh), (vec!["|".to_string()], false));
        // Words that only looked like one are said at the end.
        assert_eq!(said(&[Delta::Text("{ok} va bene. Fatto per oggi.")]).0, ["ok va bene.", "Fatto per oggi."]);
    }

    #[test]
    fn words_before_a_tool_are_said_then_a_pause() {
        let (lines, fresh) = said(&[Delta::Text("Cerco le notizie"), Delta::Tools, Delta::Text("In Calabria la protesta è sospesa.")]);
        assert_eq!(lines, ["Cerco le notizie", "|", "In Calabria la protesta è sospesa."]);
        assert!(fresh);
        // Nothing after the last tool: the reply is said whole instead.
        assert!(!said(&[Delta::Text("Un attimo che controllo."), Delta::Tools]).1);
    }

    #[test]
    fn a_refused_utterance_is_not_captured() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut r = Receiving::gated(tx, Box::new(|| false));
        r.start(&json!({"id": 3, "trigger": "wake"}));
        r.audio(&audio_frame(3, 0, &[1, 2]));
        r.end(&json!({"id": 3, "reason": "host", "speech": false}));
        drop(r);
        let heard: Vec<Hearing> = rx.iter().collect();
        assert!(matches!(heard.as_slice(), [Hearing::Refused]));
    }

    #[test]
    fn reassembles_and_fills_gaps() {
        let mut c = Capture::default();
        c.start(&json!({"id": 3, "trigger": "wake"}));
        c.audio(&audio_frame(3, 0, &[1, 2]));
        // Frame 1 lost.
        c.audio(&audio_frame(3, 2, &[5, 6]));
        // Another utterance's frame and a repeat are ignored.
        c.audio(&audio_frame(4, 3, &[9, 9]));
        c.audio(&audio_frame(3, 2, &[5, 6]));
        let u = c.end(&json!({"id": 3, "reason": "end_of_speech", "speech": true, "frames": 4})).unwrap();
        assert_eq!(u.samples, [1, 2, 0, 0, 5, 6, 0, 0]);
        assert_eq!(u.lost_frames, 2);
        assert_eq!(u.reason, "end_of_speech");
        assert!(c.end(&json!({"id": 3})).is_none());
    }

    #[test]
    fn end_of_another_utterance_is_ignored() {
        let mut c = Capture::default();
        c.start(&json!({"id": 1}));
        assert!(c.end(&json!({"id": 2})).is_none());
        assert!(c.end(&json!({"id": 1, "frames": 0})).is_some());
    }

    #[test]
    fn wav_header() {
        let w = wav(&[0, -1], 16_000);
        assert_eq!(w.len(), 48);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()), 4);
        assert_eq!(&w[44..], [0, 0, 0xff, 0xff]);
    }
}
