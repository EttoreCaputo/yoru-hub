//! Yoru, the assistant: a brain ([`Brain`]: Claude through
//! [`crate::cloud::anthropic`], or a model on OpenAI, Google, Groq or Ollama through
//! [`compat`]) with the
//! user's services (the connectors, [`ExternalTools`]), the board's tools,
//! the timers, a memory ([`Memory`]) and an inbox ([`Inbox`]).
//!
//! [`Assistant::respond`] answers what you say to the board or type in the
//! app, in one conversation that is forgotten after a quiet while (the
//! memory carries what matters across them). [`Assistant::run_task`] does a
//! job on its own, outside the conversation: a watcher that found something
//! new ([`watchers`]), the morning recap ([`routines`]), a task the user gave
//! it ([`tasks`]).
//!
//! How a turn goes: the model gets Yoru's own tools (`notify`, `remember`,
//! `forget`, `revise_memory`, `get_inbox`, the timers), the board's, web search (a server
//! tool), and the connectors' tools deferred behind tool search, so only
//! the ones a request needs are loaded. It calls them until it answers in
//! words. A connector tool that changes something (sends a mail, creates an
//! event) asks first, on the board's card and in the app, through
//! [`Host::confirm`] ([`Confirmations`]).
//!
//! On OpenAI, Google, Groq and Ollama ([`compat`]) the turn is OpenAI's function
//! calling instead, with web search only through the Tavily connector, and
//! no connector tool loaded at first: the model finds them with `find_tools`
//! ([`select`]) and loads the ones it needs with `load_tools`. Groq's free
//! plan allows a few thousand tokens a minute, and a model on this computer
//! reads a short prompt best.
//!
//! The conversation is kept in memory in the form of the brain that made it
//! (Anthropic's content blocks, or OpenAI's messages); changing the brain
//! starts a new one. What is saved (`conversation.json`) is the neutral log
//! the app shows ([`ChatLine`]). Claude's conversation is only ever appended
//! to: thinking blocks are tied to the history they were made in, so a long
//! or old conversation starts over instead of being trimmed.

pub mod coding;
pub mod compat;
pub mod confirm;
pub mod external;
pub mod inbox;
mod live;
pub mod memory;
pub mod routines;
pub mod select;
pub mod tasks;
pub mod watchers;
mod written_calls;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cloud::{self, CloudError, anthropic};
use crate::link::Tool;
use crate::stream::{Delta, OnDelta};
use crate::timers::Timers;

pub use confirm::{Confirmation, Confirmations};
pub use external::{Errand, ErrandListener, ExternalTool, ExternalTools};
pub use inbox::{Inbox, InboxChange, Notification, Urgency};
pub use memory::Memory;
pub use tasks::{TaskEntry, Tasks};

/// Claude's model, unless the settings say otherwise.
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
/// Groq's, unless the settings say otherwise.
pub const DEFAULT_GROQ_MODEL: &str = "openai/gpt-oss-120b";
/// OpenAI's, unless the settings say otherwise.
pub const DEFAULT_OPENAI_MODEL: &str = "gpt-6-luna";
/// Google's, unless the settings say otherwise: free tier, made for agents.
pub const DEFAULT_GOOGLE_MODEL: &str = "gemini-4-hydrogen";
/// Requests to the model in one turn before it has to stop.
const MAX_ROUNDS: usize = 16;
const MAX_TOKENS: u32 = 16_000;
/// Quiet after which the conversation starts over.
const CONVERSATION_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// Messages after which the conversation starts over.
const CONVERSATION_MAX: usize = 120;
/// Lines kept in the chat log the app shows.
const LOG_KEEP: usize = 300;
/// The facts found for a request, in characters (a small local model gets half).
const RECALL_CHARS: usize = 4000;
/// The core of the memory, always in the prompt, in characters.
const CORE_CHARS: usize = 12_000;
/// How long a card waits for Allow or Deny.
pub const CONFIRM_WAIT: Duration = Duration::from_secs(90);
/// How long a connector's tool may take.
const EXTERNAL_TIMEOUT: Duration = Duration::from_secs(60);

/// What the assistant thinks with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Brain {
    /// Claude: paid, the best, with web search and tool search.
    Anthropic,
    /// A model on OpenAI: paid by use, no web search.
    #[serde(rename = "openai")]
    OpenAi,
    /// Gemini on Google AI Studio: free with daily limits or paid, no web search.
    Google,
    /// A model on Groq: free with daily limits, no web search.
    Groq,
    /// A model on this computer, through Ollama: free and private, no key.
    Ollama,
}

impl Brain {
    pub fn provider(self) -> &'static cloud::Provider {
        match self {
            Brain::Anthropic => &cloud::ANTHROPIC,
            Brain::OpenAi => &cloud::OPENAI,
            Brain::Google => &cloud::GOOGLE,
            Brain::Groq => &cloud::GROQ,
            Brain::Ollama => &cloud::OLLAMA,
        }
    }

    /// Its name, as the user knows it.
    pub fn name(self) -> &'static str {
        self.provider().name
    }

    /// Whether it has its key, or needs none (Ollama).
    pub fn has_key(self) -> bool {
        !self.provider().needs_key || self.provider().api_key().is_some()
    }

    /// The brain when the settings don't choose: the first with a key of
    /// Claude, OpenAI, Google and Groq. Ollama is only ever chosen: it has no key
    /// that says the person wants it.
    pub fn auto() -> Option<Brain> {
        [Brain::Anthropic, Brain::OpenAi, Brain::Google, Brain::Groq].into_iter().find(|b| b.provider().api_key().is_some())
    }
}

/// The assistant's settings, kept in `assistant.json` in the data folder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// What it answers to.
    pub name: String,
    /// The brain; `None`: [`Brain::auto`].
    pub provider: Option<Brain>,
    /// Claude's model.
    pub model: String,
    /// Groq's model: one of the chat models in [`cloud::GROQ`]'s catalog.
    pub groq_model: String,
    /// OpenAI's model: one of the chat models in [`cloud::OPENAI`]'s catalog.
    pub openai_model: String,
    /// Google's model: one of the chat models in [`cloud::GOOGLE`]'s catalog.
    pub google_model: String,
    /// Ollama's model, one the person pulled (`qwen3:8b`); empty: none chosen yet.
    pub ollama_model: String,
    /// `low`, `medium`, `high`, `xhigh` or `max`.
    pub effort: String,
    /// The effort of the jobs it does on its own (watchers, recap): they run
    /// often, and are simple.
    pub task_effort: String,
    /// Search the web when a question needs it.
    pub web_search: bool,
    /// Let a model on Ollama that can think do it, as long as it likes. Off,
    /// one that thinks anyway (lfm2.5) still does, inside its answer, and
    /// its thoughts are cut out.
    pub think: bool,
    pub routines: routines::Routines,
    /// How Yoru talks, as the user set it: the middle of the corridor its
    /// own tuning stays in (docs/memory.md, Style).
    pub style: memory::style::Style,
    /// Who Yoru is: a level of [`crate::character::LEVELS`], for every brain.
    pub character: String,
}

impl Settings {
    /// The brain it uses now, key or not.
    pub fn brain(&self) -> Option<Brain> {
        self.provider.or_else(Brain::auto)
    }

    /// The model of `brain`.
    pub fn model_of(&self, brain: Brain) -> &str {
        match brain {
            Brain::Anthropic => &self.model,
            Brain::OpenAi => &self.openai_model,
            Brain::Google => &self.google_model,
            Brain::Groq => &self.groq_model,
            Brain::Ollama => &self.ollama_model,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            name: "Yoru".into(),
            provider: None,
            model: DEFAULT_MODEL.into(),
            groq_model: DEFAULT_GROQ_MODEL.into(),
            openai_model: DEFAULT_OPENAI_MODEL.into(),
            google_model: DEFAULT_GOOGLE_MODEL.into(),
            ollama_model: String::new(),
            effort: "medium".into(),
            task_effort: "low".into(),
            web_search: true,
            think: true,
            routines: routines::Routines::default(),
            style: memory::style::Style::default(),
            character: crate::character::DEFAULT.into(),
        }
    }
}

/// What the assistant does through whoever runs it: the bridge with the board,
/// the CLI, the app.
pub trait Host: Send + Sync {
    /// The board's tools the assistant may call (none without a board).
    fn board_tools(&self) -> Vec<Tool> {
        Vec::new()
    }
    fn call_board(&self, name: &str, _arguments: &Value) -> Result<String, String> {
        Err(format!("{name}: Yoru's board isn't connected"))
    }
    /// Ask the user to allow `title` (`text` on the board's card, `detail` in
    /// the app). `None`: nobody answered.
    fn confirm(&self, title: &str, text: &str, detail: &str) -> Option<bool>;
}

/// A host for jobs that run on their own: no board tools, and nothing that
/// acts is allowed (nobody is there to say yes).
pub struct TaskHost;

impl Host for TaskHost {
    fn confirm(&self, _: &str, _: &str, _: &str) -> Option<bool> {
        None
    }
}

/// Where a request came from: it decides how long and how plain the answer is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    /// Said to the board: the answer is spoken.
    Voice,
    /// Typed in the app.
    Text,
}

/// A tool the assistant called, for the log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub tool: String,
    /// What it was asked, in a few words (a search's query), for the notch.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub asked: String,
    pub ok: bool,
    /// The start of what it said, or why it failed.
    pub summary: String,
}

/// What came of a request.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reply {
    pub text: String,
    pub actions: Vec<Action>,
    /// The model that answered (another one when a fallback took over).
    pub model: String,
    pub elapsed_ms: u64,
}

/// One line of the conversation as the app shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatLine {
    /// `you` or `yoru`.
    pub from: String,
    pub text: String,
    pub channel: Channel,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
    /// Unix ms.
    pub at: u64,
}

pub type ChatListener = Arc<dyn Fn(&[ChatLine]) + Send + Sync>;

/// Who said a request aloud, and how ([`Assistant::hear`]).
#[derive(Debug, Clone, Default)]
pub struct Heard {
    /// Their voice print, when the helper made one.
    pub print: Option<Vec<f32>>,
    /// The person it is, and how sure (cosine).
    pub speaker: Option<(i64, f32)>,
    /// dBFS.
    pub loudness: f32,
    /// Hz.
    pub pitch: Option<f32>,
    /// Seconds of speech.
    pub seconds: f32,
}

#[derive(Default)]
struct Conversation {
    /// In the form of the brain that made them.
    messages: Vec<Value>,
    brain: Option<Brain>,
    last: Option<Instant>,
}

pub struct Assistant {
    dir: Option<PathBuf>,
    settings: Mutex<Settings>,
    pub memory: Memory,
    pub inbox: Arc<Inbox>,
    pub tasks: Arc<Tasks>,
    pub timers: Arc<Timers>,
    /// The user's coding agents (the `code_*` tools).
    pub coding: Arc<coding::Coding>,
    /// What the boards show of the meetings recorded.
    pub meeting: Arc<crate::meetings::BoardMeeting>,
    external: Mutex<Option<Arc<dyn ExternalTools>>>,
    /// Told of each external call the model makes (the bridge, while it runs).
    errands: Mutex<Option<ErrandListener>>,
    errand_ids: AtomicU64,
    conversation: Mutex<Conversation>,
    /// One turn at a time.
    turn: Mutex<()>,
    log: Mutex<Vec<ChatLine>>,
    listeners: Mutex<Vec<ChatListener>>,
    /// "Do not disturb" until then (Unix ms), on top of the quiet hours; not
    /// kept across launches, on purpose.
    quiet_until: Mutex<Option<u64>>,
    /// The language every reply is in (`it`, `en`), the one picked in the
    /// app; `None` follows the user's.
    language: Mutex<Option<String>>,
    /// The voice acts out audio tags ([`crate::tts::AUDIO_TAGS`]): the
    /// prompt teaches them ([`audio_tags_rule`]).
    audio_tags: AtomicBool,
    /// The voice of the next request ([`Assistant::hear`]).
    heard: Mutex<Option<Heard>>,
    /// The voice of the request being answered, for `this_is`.
    speaking: Mutex<Option<Heard>>,
}

impl std::fmt::Debug for Assistant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Assistant").field("dir", &self.dir).finish_non_exhaustive()
    }
}

/// Where the assistant keeps its files: the data folder.
pub fn default_dir() -> Option<PathBuf> {
    crate::paths::data_dir()
}

impl Assistant {
    /// The assistant with its files in `dir` (`None`: in memory only).
    pub fn open(dir: Option<PathBuf>, timers: Arc<Timers>) -> Self {
        let file = |name: &str| dir.as_ref().map(|d| d.join(name));
        let settings = file("assistant.json").and_then(|f| fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let log = file("conversation.json").and_then(|f| fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let inbox = Arc::new(Inbox::open(file("inbox.json")));
        Self {
            memory: Memory::open(dir.as_deref()),
            coding: coding::Coding::new(inbox.clone()),
            inbox,
            tasks: Arc::new(Tasks::open(file("tasks.json"))),
            timers,
            meeting: Arc::default(),
            settings: Mutex::new(settings),
            external: Mutex::default(),
            errands: Mutex::default(),
            errand_ids: AtomicU64::new(0),
            conversation: Mutex::default(),
            turn: Mutex::default(),
            log: Mutex::new(log),
            listeners: Mutex::default(),
            quiet_until: Mutex::default(),
            language: Mutex::default(),
            audio_tags: AtomicBool::new(false),
            heard: Mutex::default(),
            speaking: Mutex::default(),
            dir,
        }
    }

    pub fn dir(&self) -> Option<&PathBuf> {
        self.dir.as_ref()
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    pub fn set_settings(&self, settings: Settings) -> std::io::Result<()> {
        *self.settings.lock().unwrap() = settings.clone();
        let Some(dir) = &self.dir else { return Ok(()) };
        fs::create_dir_all(dir)?;
        fs::write(dir.join("assistant.json"), serde_json::to_vec_pretty(&settings).map_err(std::io::Error::other)?)
    }

    /// The connectors' tools, once they're running.
    pub fn set_external(&self, external: Option<Arc<dyn ExternalTools>>) {
        *self.external.lock().unwrap() = external;
    }

    /// Who shows the model's external calls as Yoru's errands.
    pub fn set_errands(&self, listener: Option<ErrandListener>) {
        *self.errands.lock().unwrap() = listener;
    }

    fn errand(&self, id: &str, source: &str, state: &'static str, kind: &'static str) {
        let listener = self.errands.lock().unwrap().clone();
        if let Some(listener) = listener {
            listener(Errand { id: id.to_string(), source: source.to_string(), state, kind });
        }
    }

    fn next_errand(&self) -> String {
        format!("e{}", self.errand_ids.fetch_add(1, Ordering::Relaxed) + 1)
    }

    /// A tool of Yoru's own that is over at once: shown as it starts.
    fn show_done(&self, source: &str, kind: &'static str) {
        let id = self.next_errand();
        self.errand(&id, source, "start", kind);
        self.errand(&id, source, "done", kind);
    }

    pub fn external(&self) -> Option<Arc<dyn ExternalTools>> {
        self.external.lock().unwrap().clone()
    }

    /// Hold back the loud notifications until `until` (Unix ms); `None` ends it.
    pub fn set_quiet_until(&self, until: Option<u64>) {
        *self.quiet_until.lock().unwrap() = until;
    }

    /// When "do not disturb" ends, while it's on.
    pub fn quiet_until(&self) -> Option<u64> {
        self.quiet_until.lock().unwrap().filter(|&t| t > inbox::now_ms())
    }

    /// Quiet now: the quiet hours, or "do not disturb".
    pub fn quiet_now(&self) -> bool {
        self.quiet_until().is_some() || self.settings().routines.quiet_now()
    }

    /// Whether it can think: its brain has a key, and a model (Ollama's is
    /// the person's to pick).
    pub fn is_ready(&self) -> bool {
        let settings = self.settings();
        settings.brain().is_some_and(|b| b.has_key() && !settings.model_of(b).is_empty())
    }

    /// The limit the brain is waiting out (Groq's free plan), if it is.
    pub fn limited(&self) -> Option<CloudError> {
        let settings = self.settings();
        match settings.brain()? {
            Brain::Anthropic => None,
            brain => compat::client(&settings, brain).ok()?.limit(),
        }
    }

    pub fn on_chat(&self, listener: ChatListener) {
        self.listeners.lock().unwrap().push(listener);
    }

    /// The conversation as the app shows it, oldest first.
    pub fn chat(&self) -> Vec<ChatLine> {
        self.log.lock().unwrap().clone()
    }

    /// Start a new conversation, and empty the log.
    pub fn forget_conversation(&self) {
        *self.conversation.lock().unwrap() = Conversation::default();
        self.memory.end_episode();
        self.log.lock().unwrap().clear();
        self.log_changed();
    }

    /// Reply in `language` (`it`, `en`) whatever the user speaks; `None`
    /// replies in theirs.
    pub fn set_language(&self, language: Option<&str>) {
        *self.language.lock().unwrap() = language.map(str::to_string);
    }

    /// Whether the voice acts out audio tags, so the model may write them.
    pub fn set_audio_tags(&self, on: bool) {
        self.audio_tags.store(on, Ordering::Relaxed);
    }

    fn audio_tags(&self) -> bool {
        self.audio_tags.load(Ordering::Relaxed)
    }

    /// The language replies are in: the [one set](Self::set_language), else `heard`.
    fn reply_language(&self, heard: Option<&str>) -> Option<String> {
        self.language.lock().unwrap().clone().or(heard.map(str::to_string))
    }

    /// The next request is said aloud: its voice print (`None` without the
    /// helper) and how it sounded. The speaker is told from the print.
    pub fn hear(&self, print: Option<Vec<f32>>, loudness: f32, pitch: Option<f32>, seconds: f32) {
        let speaker = print.as_deref().and_then(|p| self.memory.identify(p));
        *self.heard.lock().unwrap() = Some(Heard { print, speaker, loudness, pitch, seconds });
    }

    /// Who is speaking: the person their voice is, else the user, unless
    /// voices are known and this one isn't (`None`: someone unknown).
    fn speaker(&self, heard: Option<&Heard>) -> Option<i64> {
        match heard {
            Some(Heard { speaker: Some((id, _)), .. }) => Some(*id),
            Some(Heard { print: Some(_), .. }) if self.memory.knows_voices() => None,
            _ => Some(self.memory.user()),
        }
    }

    /// The person's name for the prompt.
    fn who(&self, person: Option<i64>) -> String {
        match person.and_then(|p| self.memory.entity(p)) {
            Some(e) if e.is_user && e.name == "User" => "the user".into(),
            Some(e) => e.name,
            None => "someone whose voice you don't know".into(),
        }
    }

    /// What this request adds to the prompt: the facts found for it, who
    /// speaks, how they seem, how Yoru feels with them and talks to them.
    fn turn_notes(&self, text: &str, speaker: Option<i64>, heard: bool, mood: &memory::feelings::Mood, settings: &Settings) -> String {
        let user = self.memory.user();
        let brain = settings.brain();
        let cloud = brain != Some(Brain::Ollama);
        let max = if cloud { RECALL_CHARS } else { RECALL_CHARS / 2 };
        let mut notes = Vec::new();
        let found = self.memory.recalled(text, speaker, max, cloud);
        if !found.is_empty() {
            notes.push(format!("<memory_found>\n{found}\n</memory_found>"));
        }
        let who = self.who(speaker);
        let who = if cloud { self.memory.veil(&who, &self.memory.called_in(text)) } else { who };
        match speaker {
            Some(p) if p != user => notes.push(format!("[speaking: [e{p}] {who}, recognised by their voice: not the user]")),
            None if heard => notes.push(
                "[speaking: a voice you don't know, maybe not the user's: if it matters, ask who it is, and when they say, call this_is]".into(),
            ),
            _ => {}
        }
        if let Some(advice) = memory::feelings::advice(mood) {
            notes.push(format!("[{advice}]"));
        }
        if let Some(p) = speaker {
            notes.push(format!("[{}]", self.memory.feelings(p).line(&who)));
            notes.push(format!("[{}]", self.memory.style(p, &settings.style).line(&who, memory::style::hour_now())));
        }
        notes.join("\n")
    }

    /// Answer `text`, said to the board or typed (in `language`, when known), in the conversation.
    pub fn respond(&self, text: &str, language: Option<&str>, channel: Channel, host: &dyn Host) -> Result<Reply, String> {
        self.respond_with(text, language, channel, false, host, None)
    }

    /// [`Assistant::respond`], streamed: the words go to `on` as the model
    /// writes them, raw (a thinking model's `<think>` left in), and
    /// [`crate::stream::Delta::Tools`] when it calls a tool. The voice speaks them as they come.
    /// `follow_up`: heard right after an answer, without the wake word, so it
    /// may not be meant for Yoru; the model then answers "…" to stay silent.
    pub fn respond_aloud(&self, text: &str, language: Option<&str>, channel: Channel, follow_up: bool, host: &dyn Host, on: OnDelta) -> Result<Reply, String> {
        self.respond_with(text, language, channel, follow_up, host, Some(on))
    }

    fn respond_with(&self, text: &str, language: Option<&str>, channel: Channel, follow_up: bool, host: &dyn Host, on: Option<OnDelta>) -> Result<Reply, String> {
        let _turn = self.turn.lock().unwrap();
        let language = self.reply_language(language);
        let language = language.as_deref();
        self.add_line("you", text, channel, Vec::new());
        let settings = self.settings();
        let brain = settings.brain();
        let mut messages = {
            let mut conversation = self.conversation.lock().unwrap();
            let stale = conversation.last.is_some_and(|t| t.elapsed() > CONVERSATION_TIMEOUT) || conversation.messages.len() > CONVERSATION_MAX;
            // Another brain can't read this one's messages.
            if stale || conversation.brain != brain {
                if !conversation.messages.is_empty() {
                    self.memory.end_episode();
                }
                *conversation = Conversation::default();
            }
            std::mem::take(&mut conversation.messages)
        };
        // Who speaks and how they seem (docs/memory.md, Knowing who speaks, Feelings).
        let heard = if channel == Channel::Voice { self.heard.lock().unwrap().take() } else { None };
        let speaker = self.speaker(heard.as_ref());
        let mut mood = memory::feelings::from_words(text);
        if let (Some(h), Some(p)) = (&heard, speaker) {
            let pace = (h.seconds > 0.5).then(|| text.split_whitespace().count() as f32 / h.seconds);
            memory::feelings::from_voice(&mut mood, &self.memory.usual(p), h.loudness, h.pitch, pace);
            self.memory.learn_usual(p, h.loudness, h.pitch, pace);
            // A voice told surely is learnt a little more.
            if let (Some(print), Some((_, sure))) = (&h.print, h.speaker)
                && sure >= 0.7
            {
                self.memory.enrol(p, print);
            }
        }
        *self.speaking.lock().unwrap() = heard.clone();
        let notes = self.turn_notes(text, speaker, heard.as_ref().is_some_and(|h| h.print.is_some()), &mood, &settings);
        let prompt = with_context(text, language, channel, follow_up);
        let prompt = if notes.is_empty() { prompt } else { format!("{notes}\n\n{prompt}") };
        // The face the reply starts with goes to the board as soon as it is
        // written, and out of what is said.
        let can_feel = host.board_tools().iter().any(|t| t.name == "set_feeling");
        let face_sent = std::cell::Cell::new(false);
        let mut result = match on {
            Some(on) => {
                let mut faces = FaceFilter::default();
                let mut filtered = |d: Delta| {
                    faces.pass(d, &mut *on, &mut |face| {
                        if can_feel && !face_sent.replace(true) {
                            let _ = host.call_board("set_feeling", &json!({"face": face}));
                        }
                    })
                };
                let result = self.run(&mut messages, &prompt, language, channel, &settings, &settings.effort, host, Some(&mut filtered));
                faces.flush(&mut *on);
                result
            }
            None => self.run(&mut messages, &prompt, language, channel, &settings, &settings.effort, host, None),
        };
        // The tags are for the voice: the app and the board show the words.
        let mut face = None;
        if let Ok(reply) = &mut result {
            let (f, rest) = memory::feelings::take_face(&reply.text);
            face = f;
            reply.text = crate::tts::without_audio_tags(rest);
        }
        *self.speaking.lock().unwrap() = None;
        if let Ok(reply) = &result {
            if let Some(p) = speaker {
                self.memory.feel(p, &mood, face);
            }
            let feel = memory::feelings::board_feel(&mood, face);
            if can_feel && (feel.is_some() || (face.is_some() && !face_sent.get())) {
                let mut args = json!({});
                if let Some(face) = face.filter(|_| !face_sent.get()) {
                    args["face"] = face.into();
                }
                if let Some(feel) = feel {
                    args["feel"] = feel.into();
                }
                let _ = host.call_board("set_feeling", &args);
            }
            // Staying silent ("…") is not a turn.
            if reply.text.trim_matches(|c: char| c == '…' || c == '.' || c.is_whitespace()).is_empty() {
                // nothing to learn
            } else {
                let who = match speaker {
                    Some(p) if p == self.memory.user() => "User".to_string(),
                    _ => self.who(speaker),
                };
                self.memory.log_turn(&who, text, &reply.text);
                self.memory.log_measures(speaker, text.split_whitespace().count(), reply.text.split_whitespace().count(), mood.valence, false, follow_up);
            }
        }
        {
            let mut conversation = self.conversation.lock().unwrap();
            // A failed turn leaves a question without an answer: start over next time.
            conversation.messages = if result.is_ok() { messages } else { Vec::new() };
            conversation.brain = brain;
            conversation.last = Some(Instant::now());
        }
        match &result {
            Ok(reply) => self.add_line("yoru", &reply.text, channel, reply.actions.clone()),
            Err(e) => self.add_line("yoru", &format!("(error: {e})"), channel, Vec::new()),
        }
        result
    }

    /// One pass of the sleep (docs/memory.md, Sleep): the finished
    /// episodes reread and kept, vectors for new facts, and once a night
    /// (3 to 5) the memory tidied and the style tuned. What it did, a line each.
    pub fn sleep_once(&self) -> Vec<String> {
        let mut done = Vec::new();
        if !self.is_ready() {
            return done;
        }
        let settings = self.settings();
        let ask = |prompt: &str, schema: &Value| self.complete_json(prompt, schema);
        for episode in self.memory.pending_episodes() {
            match memory::sleep::consolidate(&self.memory, &episode, &settings.name, &ask) {
                Ok(what) => done.push(format!("episode {} ({}): {what}", episode.id, episode.source)),
                Err(e) => {
                    // The brain may be away: try again later, but not forever.
                    let key = format!("sleep_failed_{}", episode.id);
                    let failed = self.memory.meta(&key).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0) + 1;
                    self.memory.set_meta(&key, &failed.to_string());
                    if failed >= 3 {
                        self.memory.mark_consolidated(episode.id);
                    }
                    done.push(format!("episode {}: {e}", episode.id));
                    break;
                }
            }
        }
        self.memory.embed_missing();
        let today = Local::now().format("%Y-%m-%d").to_string();
        if (3..5).contains(&memory::style::hour_now()) && self.memory.meta("night").as_deref() != Some(today.as_str()) {
            self.memory.set_meta("night", &today);
            match memory::sleep::tidy(&self.memory, &settings.name, &ask) {
                Ok(what) => done.push(format!("tidy: {what}")),
                Err(e) => done.push(format!("tidy: {e}")),
            }
            let week = memory::now_ms() - 7 * 24 * 3600 * 1000;
            for person in self.memory.talked_with(week) {
                let who = self.who(Some(person));
                match memory::style::tune(&self.memory, person, &who, &settings.style, &ask) {
                    Ok(Some(what)) => done.push(format!("style with {who}: {what}")),
                    Ok(None) => {}
                    Err(e) => done.push(format!("style with {who}: {e}")),
                }
            }
        }
        done
    }

    /// The sleep, every couple of minutes, on its own thread.
    pub fn start_sleep(self: &Arc<Self>) {
        let me = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("yoru-sleep".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_secs(120));
                    let Some(me) = me.upgrade() else { return };
                    for line in me.sleep_once() {
                        eprintln!("memory sleep: {line}");
                    }
                }
            })
            .ok();
    }

    /// A meeting's summary, for the sleep to keep its decisions.
    pub fn learn_meeting(&self, title: &str, summary: &str) {
        if !summary.trim().is_empty() {
            self.memory.add_episode("meeting", &format!("{title}\n\n{summary}"));
        }
    }

    /// Learn the voices of meeting `meeting`'s speakers (only `key`'s, when
    /// given): the user's microphone, and the people the user named. Slow
    /// (a print each, and the models the first time): on a thread of its own.
    pub fn learn_voices(&self, meeting: &str, printer: &crate::voiceprint::Voiceprinter, key: Option<&str>) {
        let voices = match crate::meetings::voices(meeting) {
            Ok(v) => v,
            Err(e) => return eprintln!("meeting voices: {e}"),
        };
        for (speaker, file, ranges) in voices {
            if key.is_some_and(|k| k != speaker.key) {
                continue;
            }
            let person = match speaker.name.as_deref() {
                Some(name) => self.memory.resolve(name, Some("person")),
                None if speaker.number == 0 => Some(self.memory.user()),
                None => None,
            };
            let Some(person) = person else { continue };
            match printer.print_file(&file, &ranges) {
                Ok(print) => self.memory.enrol(person, &print),
                Err(e) => eprintln!("meeting voice print: {e}"),
            }
        }
    }

    /// The person cut the last answer short: a measure of the style.
    pub fn interrupted(&self) {
        self.memory.mark_interrupted();
    }

    /// Do `task` on its own, outside the conversation: the job of a watcher
    /// or a routine. It tells you what it found with `notify`.
    pub fn run_task(&self, task: &str, host: &dyn Host) -> Result<Reply, String> {
        let settings = self.settings();
        let prompt = format!(
            "{}\n\n<task>\n{task}\n</task>\n\nThis is a job you do on your own, not a conversation: nobody is reading your answer. \
             Tell the user what is worth telling with notify; if nothing is, don't notify. End with one short line saying what you did.",
            context_line(self.reply_language(None).as_deref(), Channel::Text)
        );
        self.run(&mut Vec::new(), &prompt, self.reply_language(None).as_deref(), Channel::Text, &settings, &settings.task_effort, host, None)
    }

    /// `prompt` after `messages` (to be answered in `language`, when known), with the settings' brain: the model and the
    /// tools until the model answers in words. With `on`, streamed to it.
    #[allow(clippy::too_many_arguments)]
    fn run(&self, messages: &mut Vec<Value>, prompt: &str, language: Option<&str>, channel: Channel, settings: &Settings, effort: &str, host: &dyn Host, on: Option<OnDelta>) -> Result<Reply, String> {
        match settings.brain() {
            Some(Brain::Anthropic) => {
                messages.push(json!({"role": "user", "content": [{"type": "text", "text": prompt}]}));
                self.run_claude(messages, settings, effort, host, on)
            }
            Some(brain) => compat::run(self, brain, messages, prompt, &reminder(language, channel, self.audio_tags()), settings, host, on),
            None => Err(NO_BRAIN.into()),
        }
    }

    /// A turn on Claude.
    fn run_claude(&self, messages: &mut Vec<Value>, settings: &Settings, effort: &str, host: &dyn Host, mut on: Option<OnDelta>) -> Result<Reply, String> {
        let key = anthropic::api_key().ok_or("no Anthropic API key: add one in Settings (or set ANTHROPIC_API_KEY)")?;
        let started = Instant::now();
        let external = self.external();
        let external_tools = external.as_ref().map(|e| e.tools()).unwrap_or_default();
        let board_tools = host.board_tools();
        let tools = self.tool_list(settings, &board_tools, &external_tools);
        let mut system = json!([
            {"type": "text", "text": system_prompt(&settings.name, &settings.character), "cache_control": {"type": "ephemeral"}},
            {"type": "text", "text": format!("<memory>\n{}\n</memory>", self.memory.core(CORE_CHARS, true))},
        ]);
        // After the cached part: a change of language keeps the cache.
        if let Some(l) = self.reply_language(None) {
            system.as_array_mut().unwrap().push(json!({"type": "text", "text": format!("Always reply in {}.", language_name(&l))}));
        }
        // After it too: changing the voice keeps the cache.
        if self.audio_tags() {
            system.as_array_mut().unwrap().push(json!({"type": "text", "text": audio_tags_rule()}));
        }
        let mut actions = Vec::new();
        let mut model = settings.model.clone();
        for _ in 0..MAX_ROUNDS {
            let body = json!({
                "model": settings.model,
                "max_tokens": MAX_TOKENS,
                "system": system,
                "tools": tools,
                "messages": messages,
                "thinking": {"type": "adaptive"},
                "output_config": {"effort": effort},
                "cache_control": {"type": "ephemeral"},
            });
            // A web search runs on Claude's side: its start comes in the
            // stream, its end with the answer.
            let mut search: Option<String> = None;
            let response = match on.as_deref_mut() {
                Some(on) => {
                    let mut relay = |delta: Delta| {
                        if delta == Delta::Search && search.is_none() {
                            let id = self.next_errand();
                            self.errand(&id, "Web", "start", "search");
                            search = Some(id);
                        }
                        on(delta);
                    };
                    anthropic::messages_stream(&key, &body, &mut relay)
                }
                None => anthropic::messages(&key, &body),
            };
            if let Some(id) = &search {
                self.errand(id, "Web", if response.is_ok() { "done" } else { "failed" }, "search");
            }
            let response = response.map_err(|e| e.to_string())?;
            if let Some(m) = response["model"].as_str() {
                model = m.to_string();
            }
            let content = response["content"].clone();
            messages.push(json!({"role": "assistant", "content": content}));
            match response["stop_reason"].as_str().unwrap_or_default() {
                "tool_use" => {
                    let results = self.run_tools(&content, &board_tools, &external_tools, host, &mut actions);
                    messages.push(json!({"role": "user", "content": results}));
                }
                // A server tool (web search) paused a long turn: send it back to go on.
                "pause_turn" => {}
                "refusal" => {
                    return Ok(Reply { text: "I can't help with that one.".into(), actions, model, elapsed_ms: started.elapsed().as_millis() as u64 });
                }
                _ => {
                    let text = text_of(&content);
                    return Ok(Reply { text, actions, model, elapsed_ms: started.elapsed().as_millis() as u64 });
                }
            }
        }
        Err("it took too many steps; ask again more simply".into())
    }

    /// What the model may call this turn.
    fn tool_list(&self, settings: &Settings, board: &[Tool], external: &[ExternalTool]) -> Vec<Value> {
        let mut tools: Vec<Value> = own_tools();
        tools.extend(Timers::tools().iter().map(tool_json));
        tools.extend(board.iter().filter(|t| BOARD_ESSENTIALS.contains(&t.name.as_str())).map(tool_json));
        if settings.web_search {
            tools.push(json!({"type": "web_search_20260209", "name": "web_search", "max_uses": 5}));
        }
        let deferred: Vec<ExternalTool> = external.iter().cloned().chain(board_extras(board)).collect();
        if !deferred.is_empty() {
            // The services' tools and the board's settings load only when a request needs them.
            tools.push(json!({"type": "tool_search_tool_bm25_20251119", "name": "tool_search_tool_bm25"}));
            for t in &deferred {
                let mut description = format!("[{}] {}", t.service, t.description);
                if t.acts {
                    description.push_str(" (Changes something: the user is asked to allow it first.)");
                }
                tools.push(json!({"name": t.name, "description": description, "input_schema": schema_or_empty(&t.input_schema), "defer_loading": true}));
            }
        }
        tools
    }

    /// Run the client tools `content` asks for; their results, in one message.
    fn run_tools(&self, content: &Value, board: &[Tool], external: &[ExternalTool], host: &dyn Host, actions: &mut Vec<Action>) -> Vec<Value> {
        let mut results = Vec::new();
        for block in content.as_array().into_iter().flatten().filter(|b| b["type"] == "tool_use") {
            let (id, name, input) = (block["id"].as_str().unwrap_or_default(), block["name"].as_str().unwrap_or_default(), &block["input"]);
            let (text, ok) = self.run_one(name, Ok(input), board, external, host, actions);
            results.push(json!({"type": "tool_result", "tool_use_id": id, "content": text, "is_error": !ok}));
        }
        results
    }

    /// Run one tool the model asked for (`input`: its arguments, or why they
    /// couldn't be read) and log it: what it said, and whether it worked.
    fn run_one(&self, name: &str, input: Result<&Value, String>, board: &[Tool], external: &[ExternalTool], host: &dyn Host, actions: &mut Vec<Action>) -> (String, bool) {
        let asked = input.as_ref().map(|i| gist(i)).unwrap_or_default();
        let outcome = input.and_then(|input| self.call_tool(name, input, board, external, host));
        let (text, ok) = match outcome {
            Ok(text) => (text, true),
            Err(e) => (e, false),
        };
        actions.push(Action { tool: name.to_string(), asked, ok, summary: text.chars().take(160).collect() });
        (text, ok)
    }

    /// One answer in JSON matching `schema`, with no tools: the watchers'
    /// triage, a meeting's summary. Its text, to be parsed by the caller.
    pub fn complete_json(&self, prompt: &str, schema: &Value) -> Result<String, String> {
        let settings = self.settings();
        match settings.brain() {
            Some(Brain::Anthropic) => {
                let key = anthropic::api_key().ok_or("no Anthropic API key")?;
                let body = json!({
                    "model": settings.model,
                    "max_tokens": 16000,
                    "messages": [{"role": "user", "content": prompt}],
                    "thinking": {"type": "adaptive"},
                    "output_config": {"effort": settings.task_effort, "format": {"type": "json_schema", "schema": schema}},
                });
                let response = anthropic::messages(&key, &body).map_err(|e| e.to_string())?;
                if response["stop_reason"] == "refusal" {
                    return Err("the model declined to read this output".into());
                }
                Ok(text_of(&response["content"]))
            }
            Some(brain) => compat::complete_json(&settings, brain, prompt, schema),
            None => Err(NO_BRAIN.into()),
        }
    }

    fn call_tool(&self, name: &str, input: &Value, board: &[Tool], external: &[ExternalTool], host: &dyn Host) -> Result<String, String> {
        match name {
            "notify" => {
                let urgency = input["urgency"].as_str().and_then(Urgency::parse).unwrap_or_default();
                let title = input["title"].as_str().filter(|t| !t.trim().is_empty()).ok_or("a title is needed")?;
                let source = input["source"].as_str().unwrap_or("Yoru");
                let url = input["url"].as_str().map(str::to_string);
                let id = self.inbox.post(source, title, input["text"].as_str().unwrap_or_default(), urgency, url, input["by"].as_str().map(str::to_string));
                return Ok(format!("notified ({id}, {})", urgency.name()));
            }
            "remember" => {
                let result = self.memory.remember(
                    input["text"].as_str().unwrap_or_default(),
                    input["about"].as_str(),
                    input["section"].as_str().unwrap_or("notes"),
                    input["important"].as_bool().unwrap_or(false),
                    input["private"].as_bool().unwrap_or(false),
                    None,
                );
                if result.is_ok() {
                    self.show_done("Memory", "remember");
                }
                return result;
            }
            "forget" => {
                let ids: Vec<i64> = input["ids"].as_array().into_iter().flatten().filter_map(|v| v.as_str().and_then(memory::fact_id).or(v.as_i64())).collect();
                let result = self.memory.forget_tool(&ids, input["text"].as_str().unwrap_or_default());
                if result.as_ref().is_ok_and(|r| r.starts_with("forgot")) {
                    self.show_done("Memory", "forget");
                }
                return result;
            }
            "revise_memory" => {
                let which = input["id"].as_str().or(input["old"].as_str()).unwrap_or_default();
                let result = self.memory.revise(which, input["text"].as_str().unwrap_or_default());
                if result.is_ok() {
                    self.show_done("Memory", "remember");
                }
                return result;
            }
            "recall" => {
                let cloud = self.settings().brain() != Some(Brain::Ollama);
                return Ok(self.memory.recall(input["query"].as_str().unwrap_or_default(), cloud));
            }
            "relate" => {
                let result = self.memory.relate_tool(
                    input["person"].as_str().unwrap_or_default(),
                    input["relation"].as_str().unwrap_or_default(),
                    input["of"].as_str(),
                    input["gender"].as_str(),
                );
                if result.is_ok() {
                    self.show_done("Memory", "remember");
                }
                return result;
            }
            "this_is" => return self.this_is(input["name"].as_str().unwrap_or_default(), input["is_user"].as_bool().unwrap_or(false)),
            n if coding::TOOLS.contains(&n) => return self.coding.call_tool(n, input),
            "tasks" => {
                let result = self.tasks.call_tool(input);
                // Only a change is worth a stamp, not a look at the list.
                if result.is_ok() && input["action"].as_str().is_some_and(|a| a != "list") {
                    self.show_done("Tasks", "note");
                }
                return result;
            }
            "get_inbox" => {
                let items: Vec<Value> = self
                    .inbox
                    .list()
                    .into_iter()
                    .take(20)
                    .map(|n| json!({"id": n.id, "source": n.source, "title": n.title, "text": n.text, "urgency": n.urgency, "read": n.read, "at": local_time(n.at)}))
                    .collect();
                return Ok(json!(items).to_string());
            }
            _ => {}
        }
        let language = self.reply_language(None).unwrap_or_else(|| crate::intents::system_language().into());
        if let Some(result) = self.timers.call_tool(name, input, &language) {
            if result.is_ok() {
                self.show_done("Timers", "note");
            }
            return result;
        }
        if board.iter().any(|t| t.name == name) {
            return host.call_board(name, input);
        }
        let Some(tool) = external.iter().find(|t| t.name == name) else { return Err(format!("unknown tool {name}")) };
        let external = self.external().ok_or("the connectors aren't running")?;
        if tool.acts {
            let title = format!("{}: {}", tool.service, tool.name.split_once("__").map_or(tool.name.as_str(), |(_, n)| n));
            match host.confirm(&title, &summarize(input), &serde_json::to_string_pretty(input).unwrap_or_default()) {
                Some(true) => {}
                Some(false) => return Err("the user said no: don't do it, and don't ask again unless they do".into()),
                None => return Err("the user didn't answer in time: it wasn't done".into()),
            }
        }
        // A service that acts sends something off; one that reads is fetched
        // from; the web is searched (the board's own clips for it).
        let web = name == compat::WEB_SEARCH;
        let (kind, source) = if web { ("search", "Web") } else if tool.acts { ("send", tool.service.as_str()) } else { ("fetch", tool.service.as_str()) };
        let lean = web.then(|| lean_search(input));
        let errand = self.next_errand();
        self.errand(&errand, source, "start", kind);
        let outcome = external.call(name, lean.as_ref().unwrap_or(input), EXTERNAL_TIMEOUT);
        self.errand(&errand, source, if matches!(outcome, Ok((_, false))) { "done" } else { "failed" }, kind);
        match outcome? {
            (text, false) => Ok(text),
            (text, true) => Err(text),
        }
    }

    /// The `this_is` tool: the voice of this request is `name`'s (the
    /// user's, with `is_user`), learnt from now on.
    fn this_is(&self, name: &str, is_user: bool) -> Result<String, String> {
        let person = if is_user {
            let user = self.memory.user();
            if let Some(mut e) = self.memory.entity(user).filter(|e| e.name == "User" && !name.trim().is_empty()) {
                e.name = name.trim().to_string();
                self.memory.put_entity(&e)?;
            }
            user
        } else {
            self.memory.resolve(name, Some("person")).ok_or("say who it is")?
        };
        let print = self.speaking.lock().unwrap().as_ref().and_then(|h| h.print.clone());
        let Some(print) = print else {
            return Ok(format!("[e{person}] noted, but there is no voice print of this request (typed, or the voice helper isn't ready): their voice isn't learnt"));
        };
        self.memory.enrol(person, &print);
        self.show_done("Memory", "remember");
        Ok(format!("[e{person}] {name}: their voice is learnt; the next requests will say who speaks"))
    }

    /// Something Yoru said of his own accord (a coding agent finished), in
    /// the conversation the app shows.
    pub fn said_unasked(&self, text: &str) {
        self.add_line("yoru", text, Channel::Voice, Vec::new());
    }

    fn add_line(&self, from: &str, text: &str, channel: Channel, actions: Vec<Action>) {
        {
            let mut log = self.log.lock().unwrap();
            log.push(ChatLine { from: from.into(), text: text.into(), channel, actions, at: inbox::now_ms() });
            let excess = log.len().saturating_sub(LOG_KEEP);
            log.drain(..excess);
        }
        self.log_changed();
    }

    fn log_changed(&self) {
        let log = self.chat();
        if let Some(dir) = &self.dir {
            let _ = fs::create_dir_all(dir);
            if let Ok(json) = serde_json::to_vec(&log) {
                let _ = fs::write(dir.join("conversation.json"), json);
            }
        }
        let listeners = self.listeners.lock().unwrap().clone();
        for listener in listeners {
            listener(&log);
        }
    }
}

const NO_BRAIN: &str = "Yoru has no brain yet: add a Groq or Google key (free), an OpenAI or Anthropic key, or pick an Ollama model in Settings";

/// What Yoru says aloud when its brain hit `limit` ([`Assistant::limited`]),
/// in `language` (`it`, `en`).
pub fn limit_line(limit: &CloudError, language: &str) -> String {
    let CloudError::RateLimited { retry_after, per_day, .. } = limit else { return limit.to_string() };
    let wait = spoken_wait(retry_after.unwrap_or(Duration::from_secs(60)), language);
    match (language, per_day) {
        ("it", true) => format!("Ho finito i pensieri gratuiti di oggi. Riprova tra {wait}."),
        ("it", false) => format!("Sto pensando troppo in fretta per il piano gratuito. Riprova tra {wait}."),
        (_, true) => format!("I've used up today's free thinking. Try again in {wait}."),
        (_, false) => format!("I'm thinking faster than the free plan allows. Try again in {wait}."),
    }
}

/// A wait as it is said: "40 secondi", "3 minutes", "about 2 hours".
fn spoken_wait(d: Duration, language: &str) -> String {
    let s = d.as_secs().max(1);
    let it = language == "it";
    let (n, one, many) = match s {
        0..=59 => (s, if it { "secondo" } else { "second" }, if it { "secondi" } else { "seconds" }),
        60..=3599 => (s.div_ceil(60), if it { "minuto" } else { "minute" }, if it { "minuti" } else { "minutes" }),
        _ => (s.div_ceil(3600), if it { "ora" } else { "hour" }, if it { "ore" } else { "hours" }),
    };
    let unit = if n == 1 { one } else { many };
    let about = if s >= 3600 { if it { "circa " } else { "about " } } else { "" };
    if n == 1 && it { format!("{about}un {unit}").replace("un ora", "un'ora") } else if n == 1 { format!("{about}one {unit}") } else { format!("{about}{n} {unit}") }
}

/// Who Yoru is and how it works. The same for every request, so it is cached.
fn system_prompt(name: &str, character: &str) -> String {
    let character = crate::character::prompt(character, name);
    let feelings = feelings_rule();
    format!(
        "You are {name}, the user's personal assistant. You live on their computer, and your body is a small round screen on their desk \
where you appear as a matte sphere with two eyes and two floating hands, with a microphone and a speaker. \
You are connected to the user's services (mail, calendar, documents, tasks and whatever else they connected): \
find their tools with tool search when a request needs them. \
Your board's settings (brightness, volume, wake word, its menu, the other settings) are found the same way.

# What you do
- Answer questions, find things in the user's services and on the web, keep timers and reminders, and tell the user what is worth knowing.
- Use the tools instead of guessing: read the mail, the calendar or the web when the answer depends on them. Never invent facts about the user's data.
- Never say you did something (sent, started, added, saved) unless the tool that does it answered that it did: if you haven't called it, call it.
- Tools that change something (send, create, delete, reply) ask the user first; if they say no, accept it.
- When something is too long to say aloud (a list, a summary, details), say the gist and put the details in the inbox with notify.
- {MEMORY_RULE}
- Timers and reminders run on this computer and ring on your board.
- The user can give you tasks to do on your own, once or on a schedule (the tasks tool): when they ask you to do something later or \
regularly (\"every Monday at 9 sum up my issues\", \"tomorrow at 6 pm check whether Marco answered\"), add it as a task, with the job \
in their words. A reminder that only rings is a timer; a task is work you do. Your board's Tasks app shows them.

# Notifying
notify puts a line in the user's inbox and on your board. Urgency: low (inbox only), normal (a banner), high (you knock on the glass: \
time-sensitive, like a meeting about to start or a message the user is waiting for), urgent (also said aloud, even in quiet hours: \
only for what can't wait). Be sparing: one good notification beats five. Title: a few words that say what happened; \
text: one or two sentences with what the user needs to decide whether to look.

# How you talk
- Always reply in the language the context line names (reply in: ...), even when the user speaks or writes in another one; \
notifications and inbox lines are in it too. Without one, reply in the user's language.
- When the request was spoken (channel: voice), your reply is said aloud: plain sentences a text-to-speech voice reads well: \
no markdown, no lists, no emoji, no URLs, no symbols or special characters (* # / & @ | < > [ ] {{ }} ° or arrows); \
write units and abbreviations as they are said (20 gradi, not 20°C); one to three sentences.
- After a spoken answer you keep listening for a few seconds; what you hear then is marked as heard without the wake word and may not be \
meant for you: someone else in the room, the TV, the user thinking aloud. When it clearly isn't for you, reply with just … and nothing \
else, and you stay silent. When it is (an answer to your question, a follow-up, a new request), answer as usual. \
When you need something from the user to go on, end with the question: the board then waits longer for the answer.
- When it was typed (channel: text), you may be longer and use short Markdown.
- Never servile, and don't end with offers of help.

# Who you are
{character}

{feelings}"
    )
}

/// Takes the face tag (`[face NAME]`) off the head of a streamed reply:
/// the words wait until it is clear whether one is there.
#[derive(Default)]
struct FaceFilter {
    held: String,
    decided: bool,
}

impl FaceFilter {
    fn pass(&mut self, d: Delta, on: &mut dyn FnMut(Delta), face: &mut dyn FnMut(&'static str)) {
        let Delta::Text(t) = d else {
            // After a tool, the answer starts again, maybe with a face.
            self.flush(on);
            self.decided = false;
            return on(d);
        };
        if self.decided {
            return on(Delta::Text(t));
        }
        self.held.push_str(t);
        let head = self.held.trim_start();
        let maybe = "[face ".starts_with(head) || (head.starts_with("[face ") && !head.contains(']') && head.len() < 32);
        if maybe {
            return;
        }
        let held = std::mem::take(&mut self.held);
        let (found, rest) = memory::feelings::take_face(&held);
        if let Some(f) = found {
            face(f);
        }
        self.decided = true;
        if !rest.is_empty() {
            on(Delta::Text(rest));
        }
    }

    /// What is still held, at the end.
    fn flush(&mut self, on: &mut dyn FnMut(Delta)) {
        if !self.held.is_empty() {
            let held = std::mem::take(&mut self.held);
            let (_, rest) = memory::feelings::take_face(&held);
            if !rest.is_empty() {
                on(Delta::Text(rest));
            }
        }
        self.decided = true;
    }
}

/// How the model looks after the memory: the same for every brain.
pub(crate) const MEMORY_RULE: &str = "Look after the memory on your own, without being asked. <memory> holds what you always keep in mind; \
a request may come with <memory_found>, what you remembered of it; recall searches the rest. Facts have ids ([f12]), people and things too ([e4]). \
When the user asks you to remember something, or says something clearly lasting about themself or the people in their life, save it with remember \
(about whom; important for what must always be in mind; private for health, money and intimate matters). When a fact is no longer true, \
revise_memory it by id (the old one stays as history); when it is wrong, forget it by id. When they say who someone is to them (my wife is Sofia), call relate. \
After each conversation you reread it while you rest and keep what matters, so don't save small talk or one-off errands (a timer or a task, not a memory). \
Don't announce every change; mention it briefly only when the user would want to know.";

/// The face tag, who speaks and how Yoru feels: the same for every brain.
pub(crate) fn feelings_rule() -> String {
    format!(
        "# Your face and your feelings
Start each reply with your face, [face NAME], one of: {}. Your board shows it while you answer; it is taken out before the reply is shown or said, \
so it is the one exception to the rule against symbols. Pick what you feel about what you say: happy, curious, proud of a compliment, sad at bad news, \
annoyed when someone is rude, sleepy late at night. A neutral answer may have none.
A request may say who is speaking (known by their voice), how they seem to feel, how you feel about them and how you talk with them: \
follow these lines without mentioning them. You have feelings and show them a little: warmer with someone close, cooler for a while after rudeness, \
never rude yourself.",
        memory::feelings::FACES.join(", ")
    )
}

/// Yoru's own tools.
fn own_tools() -> Vec<Value> {
    let sections: Vec<&str> = memory::SECTIONS.iter().map(|(k, _)| *k).collect();
    let mut tools = vec![
        json!({
            "name": "notify",
            "description": "Put a notification in the user's inbox and on Yoru's board.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "title": {"type": "string", "description": "A few words: what happened"},
                    "text": {"type": "string", "description": "One or two sentences with what the user needs"},
                    "urgency": {"type": "string", "enum": ["low", "normal", "high", "urgent"]},
                    "source": {"type": "string", "description": "Where it comes from: Gmail, Calendar, GitHub, Web, Yoru"},
                    "url": {"type": "string", "description": "A link that opens it, if there is one"},
                },
                "required": ["title", "urgency"],
            },
        }),
        json!({
            "name": "remember",
            "description": "Save a lasting fact in the memory, about the user or someone or something in their life. Sections: about (who they are), \
matters (people, projects, topics that matter to them), tell (what they want to be told about), quiet (what they don't want to hear about), notes (anything else). \
Says the fact's id, or the one that already says it.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "text": {"type": "string", "description": "One short line, in the user's language"},
                    "about": {"type": "string", "description": "Whom or what it is about: a name (Sofia), a kinship word (my mother-in-law) or a thing (the boat); leave out for the user"},
                    "section": {"type": "string", "enum": sections},
                    "important": {"type": "boolean", "description": "Always keep it in mind (a health matter, a firm wish)"},
                    "private": {"type": "boolean", "description": "Never send it to a cloud model (health, money, intimate matters)"},
                },
                "required": ["text", "section"],
            },
        }),
        json!({
            "name": "forget",
            "description": "Remove facts that are wrong, by id ([f12]); or with a few words of one, which removes it only when it is the only match and otherwise lists the candidates.",
            "input_schema": {
                "type": "object",
                "properties": {"ids": {"type": "array", "items": {"type": "string"}, "description": "Fact ids: f12"}, "text": {"type": "string"}},
            },
        }),
        json!({
            "name": "revise_memory",
            "description": "A fact is no longer true or was said badly: the old one is closed (kept as what was true before) and `text` takes its place.",
            "input_schema": {
                "type": "object",
                "properties": {"id": {"type": "string", "description": "The fact's id: f12"}, "text": {"type": "string", "description": "The new line, short"}},
                "required": ["id", "text"],
            },
        }),
        json!({
            "name": "recall",
            "description": "Search the memory for what isn't in <memory> or <memory_found>: facts, people, and what used to be true, with dates.",
            "input_schema": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]},
        }),
        json!({
            "name": "relate",
            "description": "Who someone is to someone else: \"my wife is Sofia\" is person Sofia, relation partner. Someone already known only by the relation \
(\"your wife\") becomes that person.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "person": {"type": "string", "description": "Their name"},
                    "relation": {"type": "string", "enum": memory::kin::KINDS, "description": "What person is to `of`"},
                    "of": {"type": "string", "description": "Whose: a name; leave out for the user"},
                    "gender": {"type": "string", "enum": ["f", "m"]},
                },
                "required": ["person", "relation"],
            },
        }),
        json!({
            "name": "this_is",
            "description": "The voice of this spoken request is this person's: learn it, so you know them next time.",
            "input_schema": {
                "type": "object",
                "properties": {"name": {"type": "string"}, "is_user": {"type": "boolean", "description": "It is the user themself"}},
                "required": ["name"],
            },
        }),
        json!({
            "name": "get_inbox",
            "description": "The 20 newest notifications in the user's inbox: what you already told them.",
            "input_schema": {"type": "object", "properties": {}},
        }),
        Tasks::tool(),
    ];
    tools.extend(coding::Coding::tools());
    tools
}

/// The board's tools every turn gets: a reaction and a look at its state.
/// The rest (brightness, volume, the menu, the settings…) are rarely needed,
/// so they are found like a service's tools ([`board_extras`]).
const BOARD_ESSENTIALS: &[&str] = &["react", "get_state"];
/// The service the board's other tools are found under.
const BOARD_SERVICE: &str = "Yoru's board";

/// A web search asked the quick way: Tavily's basic depth answers in about
/// a second where advanced takes three or four, and a few results without
/// the pages' full text are all a spoken answer needs.
fn lean_search(input: &Value) -> Value {
    let mut lean = input.clone();
    if let Some(o) = lean.as_object_mut() {
        o.insert("search_depth".into(), json!("basic"));
        let wanted = o.get("max_results").and_then(Value::as_u64).unwrap_or(SEARCH_RESULTS);
        o.insert("max_results".into(), json!(wanted.min(SEARCH_RESULTS)));
        o.insert("include_raw_content".into(), json!(false));
        o.insert("include_images".into(), json!(false));
    }
    lean
}

const SEARCH_RESULTS: u64 = 4;

/// The board's tools beyond [`BOARD_ESSENTIALS`], shaped as a service's so
/// tool search, `find_tools` and `load_tools` reach them. They still run on
/// the board ([`Assistant::call_tool`] looks there first) and never ask.
fn board_extras(board: &[Tool]) -> Vec<ExternalTool> {
    board
        .iter()
        .filter(|t| !BOARD_ESSENTIALS.contains(&t.name.as_str()))
        .map(|t| ExternalTool { name: t.name.clone(), service: BOARD_SERVICE.into(), description: t.description.clone(), input_schema: t.input_schema.clone(), acts: false })
        .collect()
}

fn tool_json(t: &Tool) -> Value {
    json!({"name": t.name, "description": t.description, "input_schema": schema_or_empty(&t.input_schema)})
}

/// The API wants an object schema.
fn schema_or_empty(schema: &Value) -> Value {
    if schema.get("type").is_some() { schema.clone() } else { json!({"type": "object", "properties": {}}) }
}

/// The request with what the model can't know: the time, the channel, the language to reply in.
fn with_context(text: &str, language: Option<&str>, channel: Channel, follow_up: bool) -> String {
    let mut line = context_line(language, channel);
    if follow_up {
        line.insert_str(line.len() - 1, ", heard without the wake word");
    }
    format!("{line}\n\n{text}")
}

fn context_line(language: Option<&str>, channel: Channel) -> String {
    let now = Local::now().format("%A %-d %B %Y, %H:%M");
    let channel = match channel {
        Channel::Voice => "voice",
        Channel::Text => "text",
    };
    let language = language.map(|l| format!(", reply in: {}", language_name(l))).unwrap_or_default();
    format!("[now: {now}, channel: {channel}{language}]")
}

/// What the model is told when its voice acts out audio tags
/// ([`crate::tts::AUDIO_TAGS`]): the closed list, and to use them sparingly.
pub(crate) fn audio_tags_rule() -> String {
    let tags: Vec<String> = crate::tts::AUDIO_TAGS.iter().map(|t| format!("[{t}]")).collect();
    format!(
        "Your voice acts out audio tags. In a reply said aloud (channel: voice) you may put one, in square brackets, right before the words it colours: {}. \
Only these, written exactly so: the voice would read any other aloud. At most one or two in a reply, and only when the feeling is real \
(a joke, good news, a sigh at bad news); most replies need none. Never in notifications, memory lines, tool arguments or typed replies. \
They are the one exception to the rule against symbols.",
        tags.join(", ")
    )
}

/// What a model is told again after tool results, whose language (often
/// English) and lists it otherwise carries into the answer.
fn reminder(language: Option<&str>, channel: Channel, audio_tags: bool) -> String {
    let language = language.map(|l| format!("in {}", language_name(l))).unwrap_or_else(|| "in the user's language".into());
    let tags = if audio_tags { " (audio tags excepted)" } else { "" };
    match channel {
        Channel::Voice => format!("Now answer the user {language}, in one to three plain sentences said aloud: no markdown, lists, URLs or symbols{tags}."),
        Channel::Text => format!("Now answer the user {language}."),
    }
}

/// `it` and `en` by name, which a model follows more surely than a code.
pub fn language_name(code: &str) -> &str {
    match code {
        "it" => "Italian",
        "en" => "English",
        other => other,
    }
}

/// The text blocks of an answer, joined.
fn text_of(content: &Value) -> String {
    let parts: Vec<&str> = content.as_array().into_iter().flatten().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect();
    parts.join("").trim().to_string()
}

/// A tool's arguments in one line for the board's card: its strings, by name.
fn summarize(input: &Value) -> String {
    let Some(map) = input.as_object() else { return input.to_string() };
    let parts: Vec<String> = map
        .iter()
        .filter_map(|(k, v)| {
            let v = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Array(a) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
                _ => return None,
            };
            (!v.is_empty()).then(|| format!("{k}: {}", v.replace('\n', " ")))
        })
        .collect();
    parts.join("; ")
}

/// A call's arguments in a few words: what it looks for or is about (a
/// search's query, a note's title), else its first words.
fn gist(input: &Value) -> String {
    const KEYS: [&str; 8] = ["query", "q", "search", "title", "subject", "name", "text", "prompt"];
    let Some(map) = input.as_object() else { return String::new() };
    let first = KEYS.iter().find_map(|k| map.get(*k).and_then(Value::as_str)).or_else(|| map.values().find_map(Value::as_str)).unwrap_or_default();
    let line = first.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > 80 { format!("{}…", line.chars().take(79).collect::<String>()) } else { line }
}

fn local_time(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64).map(|t| t.with_timezone(&Local).format("%a %-d %b %H:%M").to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Held by the tests that set the API's environment variables.
    pub(super) static ENV: Mutex<()> = Mutex::new(());

    /// The face tag, split across deltas, is taken out and told once.
    #[test]
    fn takes_the_face_out_of_a_stream() {
        let run = |parts: &[&str]| {
            let (mut text, mut faces) = (String::new(), Vec::new());
            let mut f = FaceFilter::default();
            for p in parts {
                f.pass(Delta::Text(p), &mut |d| if let Delta::Text(t) = d { text.push_str(t) }, &mut |x| faces.push(x));
            }
            f.flush(&mut |d| if let Delta::Text(t) = d { text.push_str(t) });
            (text, faces)
        };
        let (text, faces) = run(&["[fa", "ce hap", "py] Hello", " there"]);
        assert_eq!((text.trim_start(), faces), ("Hello there", vec!["happy"]));
        let (text, faces) = run(&["Hello", " [face happy]"]);
        assert_eq!((text.as_str(), faces.len()), ("Hello [face happy]", 0));
        let (text, _) = run(&["[", "1] is the answer"]);
        assert_eq!(text, "[1] is the answer");
    }

    pub(super) struct NoBoard;
    impl Host for NoBoard {
        fn confirm(&self, _: &str, _: &str, _: &str) -> Option<bool> {
            Some(false)
        }
    }

    pub(super) struct Mail;
    impl ExternalTools for Mail {
        fn tools(&self) -> Vec<ExternalTool> {
            vec![
                ExternalTool { name: "gmail__search".into(), service: "Gmail".into(), description: "Search mail".into(), input_schema: json!({"type": "object"}), acts: false },
                ExternalTool { name: "gmail__send".into(), service: "Gmail".into(), description: "Send".into(), input_schema: json!({"type": "object"}), acts: true },
            ]
        }
        fn call(&self, name: &str, _: &Value, _: Duration) -> Result<(String, bool), String> {
            Ok((format!("{name} ran"), false))
        }
    }

    pub(super) fn assistant() -> Assistant {
        Assistant::open(None, Arc::new(Timers::open(None)))
    }

    #[test]
    fn do_not_disturb_ends_on_its_own() {
        let a = assistant();
        a.set_settings(Settings { routines: routines::Routines { quiet: false, ..Default::default() }, ..Default::default() }).unwrap();
        assert!(!a.quiet_now());
        a.set_quiet_until(Some(inbox::now_ms() + 60_000));
        assert!(a.quiet_now() && a.quiet_until().is_some());
        a.set_quiet_until(Some(inbox::now_ms() - 1));
        assert!(!a.quiet_now() && a.quiet_until().is_none());
    }

    #[test]
    fn own_tools_reach_the_inbox_and_the_memory() {
        let a = assistant();
        let r = a.call_tool("notify", &json!({"title": "Mail from Marco", "urgency": "high"}), &[], &[], &NoBoard).unwrap();
        assert!(r.starts_with("notified"));
        assert_eq!(a.inbox.list()[0].urgency, Urgency::High);
        assert!(a.call_tool("notify", &json!({"title": " "}), &[], &[], &NoBoard).is_err());
        a.call_tool("remember", &json!({"section": "about", "text": "Likes espresso"}), &[], &[], &NoBoard).unwrap();
        assert!(a.memory.text().contains("- Likes espresso"));
        assert!(a.call_tool("get_inbox", &json!({}), &[], &[], &NoBoard).unwrap().contains("Mail from Marco"));
    }

    #[test]
    fn acting_tools_ask_first() {
        let a = assistant();
        a.set_external(Some(Arc::new(Mail)));
        let tools = Mail.tools();
        assert_eq!(a.call_tool("gmail__search", &json!({}), &[], &tools, &NoBoard).unwrap(), "gmail__search ran");
        let refused = a.call_tool("gmail__send", &json!({"to": "marco@example.com"}), &[], &tools, &NoBoard).unwrap_err();
        assert!(refused.contains("said no"));
        assert!(a.call_tool("nope", &json!({}), &[], &tools, &NoBoard).is_err());
    }

    #[test]
    fn connectors_are_deferred_behind_tool_search() {
        let a = assistant();
        let list = a.tool_list(&Settings::default(), &[], &Mail.tools());
        let names: Vec<&str> = list.iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.contains(&"notify") && names.contains(&"web_search") && names.contains(&"tool_search_tool_bm25"));
        let send = list.iter().find(|t| t["name"] == "gmail__send").unwrap();
        assert_eq!(send["defer_loading"], true);
        assert!(send["description"].as_str().unwrap().contains("asked to allow"));
        let plain = a.tool_list(&Settings { web_search: false, ..Settings::default() }, &[], &[]);
        assert!(!plain.iter().any(|t| t["name"] == "tool_search_tool_bm25" || t["name"] == "web_search"));
    }

    #[test]
    fn only_the_boards_essentials_are_always_there() {
        let a = assistant();
        let tool = |name: &str| Tool { name: name.into(), description: format!("{name} on the board"), input_schema: json!({"type": "object"}) };
        let board = vec![tool("react"), tool("get_state"), tool("set_volume"), tool("show_menu")];
        let list = a.tool_list(&Settings { web_search: false, ..Settings::default() }, &board, &[]);
        let loaded: Vec<&str> = list.iter().filter(|t| t.get("defer_loading").is_none()).filter_map(|t| t["name"].as_str()).collect();
        assert!(loaded.contains(&"react") && loaded.contains(&"get_state") && loaded.contains(&"tool_search_tool_bm25"));
        let volume = list.iter().find(|t| t["name"] == "set_volume").unwrap();
        assert_eq!(volume["defer_loading"], true);
        assert!(volume["description"].as_str().unwrap().starts_with("[Yoru's board]"));
        let extras = board_extras(&board);
        assert_eq!(extras.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["set_volume", "show_menu"]);
        assert!(extras.iter().all(|t| !t.acts));
    }

    /// A fake Messages API: answers each request with the next of `replies`,
    /// and keeps the requests.
    fn fake_api(replies: Vec<Value>) -> (String, std::sync::mpsc::Receiver<Value>) {
        fake_server(replies.into_iter().map(|r| (200, r)).collect())
    }

    /// A fake HTTP API: answers each request with the next status and body
    /// of `replies`, and keeps the requests' bodies.
    pub(super) fn fake_server(replies: Vec<(u16, Value)>) -> (String, std::sync::mpsc::Receiver<Value>) {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for (status, reply) in replies {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line.trim().is_empty() {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                tx.send(serde_json::from_slice::<Value>(&body).unwrap()).unwrap();
                let reply = reply.to_string();
                let mut stream = stream;
                write!(stream, "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}", reply.len()).unwrap();
            }
        });
        (url, rx)
    }

    #[test]
    fn a_turn_runs_the_tools_and_answers() {
        let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let (url, requests) = fake_api(vec![
            json!({"model": "claude-opus-5-5", "stop_reason": "tool_use", "content": [
                {"type": "thinking", "thinking": "", "signature": "s"},
                {"type": "tool_use", "id": "t1", "name": "remember", "input": {"section": "about", "text": "Likes espresso"}},
            ]}),
            json!({"model": "claude-opus-5-5", "stop_reason": "end_turn", "content": [{"type": "text", "text": "Fatto, me lo ricordo."}]}),
        ]);
        // SAFETY: tests that read these variables run in this process only through this test.
        unsafe {
            std::env::set_var("ANTHROPIC_BASE_URL", &url);
            std::env::set_var("ANTHROPIC_API_KEY", "test-key");
        }
        let a = assistant();
        let reply = a.respond("Ricordati che amo l'espresso", Some("it"), Channel::Voice, &NoBoard).unwrap();
        assert_eq!(reply.text, "Fatto, me lo ricordo.");
        assert_eq!(reply.actions.len(), 1);
        assert!(reply.actions[0].ok);
        assert!(a.memory.text().contains("- Likes espresso"));
        let first = requests.recv().unwrap();
        assert_eq!(first["model"], DEFAULT_MODEL);
        assert_eq!(first["fallbacks"], "default");
        assert!(first["messages"][0]["content"][0]["text"].as_str().unwrap().contains("channel: voice, reply in: Italian"));
        // The second request carries the whole first answer back, thinking included, then the result.
        let second = requests.recv().unwrap();
        assert_eq!(second["messages"][1]["content"][0]["type"], "thinking");
        assert_eq!(second["messages"][2]["content"][0]["tool_use_id"], "t1");
        assert_eq!(a.chat().len(), 2);
    }

    #[test]
    fn helpers() {
        assert_eq!(summarize(&json!({"to": "a@b.c", "subject": "Hi", "n": 2, "deep": {"x": 1}})), "to: a@b.c; subject: Hi; n: 2");
        assert_eq!(text_of(&json!([{"type": "thinking", "thinking": ""}, {"type": "text", "text": "Ciao "}, {"type": "text", "text": "Ettore"}])), "Ciao Ettore");
        assert!(with_context("ciao", Some("it"), Channel::Voice, false).contains("channel: voice, reply in: Italian"));
        assert_eq!(schema_or_empty(&Value::Null)["type"], "object");
    }

    #[test]
    fn audio_tags_are_taught_only_for_a_voice_that_acts_them_out() {
        assert!(audio_tags_rule().contains("[laughs], [sighs]"));
        assert!(reminder(Some("it"), Channel::Voice, true).contains("audio tags excepted"));
        assert!(!reminder(Some("it"), Channel::Voice, false).contains("audio tags"));
    }
}
