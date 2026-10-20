//! Online services that run the voice's models instead of this computer:
//! speech-to-text ([`crate::Stt`]), the language model ([`crate::Llm`]) and
//! voices ([`crate::Tts`]).
//!
//! A [`Provider`] is a service: where it is, how to talk to it ([`Api`]),
//! where its API key comes from and the models of it Yoru knows
//! ([`CloudModel`]). Wherever a local model id goes (`--stt`, `--llm`,
//! `--tts-voice`, the app's settings) a cloud one does too, as
//! `provider:model`: `groq:whisper-large-v3-turbo`, `groq:hannah`. Local ids
//! have no `:`. ElevenLabs' voices are those of the person's account, by
//! their id (`elevenlabs:voice/<voice_id>`), see [`elevenlabs`].
//!
//! Adding a service that speaks OpenAI's API is one more [`Provider`] in
//! [`PROVIDERS`]; one that doesn't is another [`Api`], and a module like
//! [`openai`] that [`Client`] sends its requests to.
//!
//! API keys come from the provider's environment variable (`GROQ_API_KEY`)
//! or `keys.json` in the data folder ([`set_api_key`]), readable only
//! by its owner.
//!
//! A model that hits its provider's limits (429: too many requests a
//! minute, or the day's are used up) isn't asked again until it said to
//! retry, a minute when it didn't: its requests fail at once with
//! [`CloudError::RateLimited`], which [`crate::Stt`], [`crate::Llm`] and
//! [`crate::Tts`] answer with a local model when there's one.

pub mod anthropic;
pub mod elevenlabs;
mod openai;

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::models::Kind;

/// How a [`Provider`] is talked to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Api {
    /// OpenAI's: `/chat/completions`, `/audio/transcriptions`, `/audio/speech`.
    OpenAi,
    /// ElevenLabs': voices only ([`elevenlabs`]).
    #[serde(rename = "elevenlabs")]
    ElevenLabs,
    /// Anthropic's Messages API: the assistant's brain ([`anthropic`]).
    Anthropic,
}

/// An online service.
#[derive(Debug, PartialEq, Eq)]
pub struct Provider {
    /// The `provider` in `provider:model`, and its key in `keys.json`.
    pub id: &'static str,
    pub name: &'static str,
    pub api: Api,
    /// Requests go to `{base_url}/chat/completions` and so on.
    pub base_url: &'static str,
    /// Where the API key is read from first.
    pub key_env: &'static str,
    /// Where to get a key.
    pub keys_url: &'static str,
    /// One line about it for the person picking it.
    pub note: &'static str,
    pub models: &'static [CloudModel],
    /// Whether it needs a key: a server on this computer (Ollama) doesn't.
    pub needs_key: bool,
    /// Whether any model id is one of its models: the ones the person has
    /// (Ollama's), not a catalog ([`ANY_MODEL`]).
    pub any_model: bool,
}

/// A model of a [`Provider`].
#[derive(Debug, PartialEq, Eq)]
pub struct CloudModel {
    /// The `model` in `provider:model`.
    pub id: &'static str,
    pub kind: Kind,
    /// The provider's name for it.
    pub model: &'static str,
    /// A voice's name, for [`Kind::Voice`].
    pub voice: Option<&'static str>,
    /// A voice's language: `it` or `en`; none for one that speaks both.
    pub language: Option<&'static str>,
    pub note: &'static str,
    /// JSON merged into every chat request for it (thinking turned down, a
    /// higher token limit for a model that thinks anyway).
    pub params: &'static str,
    /// The longest text a voice says in one request, in characters (0: no limit).
    pub max_chars: usize,
}

const fn chat(id: &'static str, note: &'static str, params: &'static str) -> CloudModel {
    CloudModel { id, kind: Kind::Llm, model: id, voice: None, language: None, note, params, max_chars: 0 }
}

const fn whisper(id: &'static str, note: &'static str) -> CloudModel {
    CloudModel { id, kind: Kind::Whisper, model: id, voice: None, language: None, note, params: "", max_chars: 0 }
}

const ORPHEUS_EN: &str = "canopylabs/orpheus-v1-english";

const fn orpheus(voice: &'static str, note: &'static str) -> CloudModel {
    CloudModel { id: voice, kind: Kind::Voice, model: ORPHEUS_EN, voice: Some(voice), language: Some("en"), note, params: "", max_chars: 200 }
}

/// [GroqCloud](https://console.groq.com): free with daily limits. The
/// limits that matter for a voice command (free plan, October 2026): 8K
/// tokens a minute for the GPT-OSS and Qwen models, about five commands a
/// minute with the board's tools; 100 sentences a day spoken by Orpheus,
/// 2,000 transcriptions a day by Whisper.
pub const GROQ: Provider = Provider {
    id: "groq",
    name: "Groq",
    api: Api::OpenAi,
    base_url: "https://api.groq.com/openai/v1",
    key_env: "GROQ_API_KEY",
    keys_url: "https://console.groq.com/keys",
    note: "Free with daily limits; what you say is sent to Groq",
    models: &[
        whisper("whisper-large-v3-turbo", "Whisper large v3 turbo on Groq: fast and accurate, 2,000 a day free"),
        whisper("whisper-large-v3", "Whisper large v3 on Groq: a little more accurate, slower"),
        chat(
            "openai/gpt-oss-20b",
            "GPT-OSS 20B on Groq: fast, good at tools; about 5 commands a minute free",
            r#"{"reasoning_effort": "low", "include_reasoning": false, "max_tokens": 1024}"#,
        ),
        chat(
            "openai/gpt-oss-120b",
            "GPT-OSS 120B on Groq: smarter, a little slower; same free limits",
            r#"{"reasoning_effort": "low", "include_reasoning": false, "max_tokens": 1024}"#,
        ),
        chat("qwen/qwen3.8-27b", "Qwen3.8 27B on Groq (preview): good Italian", r#"{"reasoning_effort": "none"}"#),
        chat("llama-3.3-70b-versatile", "Llama 3.3 70B on Groq: good Italian, no thinking", ""),
        orpheus("hannah", "Orpheus on Groq, English, woman's voice; 100 sentences a day free"),
        orpheus("diana", "Orpheus on Groq, English, woman's voice"),
        orpheus("autumn", "Orpheus on Groq, English, woman's voice"),
        orpheus("troy", "Orpheus on Groq, English, man's voice"),
        orpheus("austin", "Orpheus on Groq, English, man's voice"),
        orpheus("daniel", "Orpheus on Groq, English, man's voice"),
    ],
    needs_key: true,
    any_model: false,
};

/// [ElevenLabs](https://elevenlabs.io): voices only, those of the account,
/// so none in the catalog. The free plan (October 2026): 10,000 credits a
/// month, about 20 minutes of speech; voices are designed on its site.
pub const ELEVENLABS: Provider = Provider {
    id: "elevenlabs",
    name: "ElevenLabs",
    api: Api::ElevenLabs,
    base_url: "https://api.elevenlabs.io",
    key_env: "ELEVENLABS_API_KEY",
    keys_url: "https://elevenlabs.io/app/developers/api-keys",
    note: "The voices of your account; free plan: 10,000 credits a month. The answers are sent to ElevenLabs",
    models: &[],
    needs_key: true,
    any_model: false,
};

/// What `elevenlabs:voice/<voice_id>` names: a voice of the person's
/// account, which isn't in the catalog. Its id is the [`Client`]'s.
const ACCOUNT_VOICE: CloudModel = CloudModel {
    id: "voice",
    kind: Kind::Voice,
    model: "eleven_flash_v2_5",
    voice: Some("voice"),
    language: None,
    note: "A voice of your ElevenLabs account",
    params: "",
    max_chars: 0,
};

/// [Anthropic](https://platform.claude.com): Claude, the assistant's brain
/// ([`crate::assistant`]). Paid by use; what you ask and what the
/// connectors read go to Anthropic.
pub const ANTHROPIC: Provider = Provider {
    id: "anthropic",
    name: "Anthropic",
    api: Api::Anthropic,
    base_url: "https://api.anthropic.com",
    key_env: "ANTHROPIC_API_KEY",
    keys_url: "https://platform.claude.com/settings/keys",
    note: "Claude, Yoru's brain; paid by use. What you ask and what Yoru reads for you are sent to Anthropic",
    models: &[],
    needs_key: true,
    any_model: false,
};

/// [OpenAI](https://platform.openai.com): a brain for the assistant, paid
/// by use, and Whisper. Only the models that call tools through Chat
/// Completions are here (October 2026): GPT-6 Luna and GPT-5.4 Mini do it
/// only with `reasoning_effort` set to `none`; GPT-6.1 Sol and GPT-6 Astra
/// call tools only through the Responses API, which Yoru doesn't speak.
pub const OPENAI: Provider = Provider {
    id: "openai",
    name: "OpenAI",
    api: Api::OpenAi,
    base_url: "https://api.openai.com/v1",
    key_env: "OPENAI_API_KEY",
    keys_url: "https://platform.openai.com/api-keys",
    note: "GPT, a brain for Yoru; paid by use. What you ask and what Yoru reads for you are sent to OpenAI",
    models: &[
        chat("gpt-6-luna", "GPT-6 Luna: fast and cheap ($0.10 / $0.50 a million tokens)", NO_REASONING),
        chat("gpt-5.4-mini", "GPT-5.4 Mini: smarter, still quick ($0.75 / $4.50)", NO_REASONING),
        chat("gpt-5.6-terra", "GPT-5.6 Terra: smarter still, pricier ($2 / $12)", NO_REASONING),
        whisper("whisper-1", "Whisper on OpenAI: paid by the minute"),
    ],
    needs_key: true,
    any_model: false,
};

/// OpenAI's models call tools through Chat Completions only without reasoning.
const NO_REASONING: &str = r#"{"reasoning_effort": "none"}"#;

/// [Ollama](https://ollama.com): models on this computer, free and private,
/// through its OpenAI-compatible API. No key; the models are the ones the
/// person pulled ([`list_models`]), so any id is one ([`ANY_MODEL`]).
/// `OLLAMA_BASE_URL` points it at another machine.
pub const OLLAMA: Provider = Provider {
    id: "ollama",
    name: "Ollama",
    api: Api::OpenAi,
    base_url: "http://localhost:11434/v1",
    key_env: "OLLAMA_API_KEY",
    keys_url: "https://ollama.com/search?c=tools",
    note: "Models on this computer: free and private, as smart as the computer allows",
    models: &[],
    needs_key: false,
    any_model: true,
};

/// What `ollama:<model>` names: a model the person has, not in the catalog.
/// Its name is the [`Client`]'s.
const ANY_MODEL: CloudModel = CloudModel { id: "model", kind: Kind::Llm, model: "", voice: None, language: None, note: "A model you have", params: "", max_chars: 0 };

/// Every service Yoru can use.
pub const PROVIDERS: &[Provider] = &[ANTHROPIC, OPENAI, GROQ, OLLAMA, ELEVENLABS];

impl Provider {
    pub fn by_id(id: &str) -> Option<&'static Provider> {
        PROVIDERS.iter().find(|p| p.id == id)
    }

    /// Where its requests go: `GROQ_BASE_URL` (its key's variable, with
    /// `_BASE_URL`) when set, for a proxy or a test server, else
    /// [`Provider::base_url`].
    pub fn url(&self) -> String {
        let env = self.key_env.replace("_API_KEY", "_BASE_URL");
        std::env::var(env).ok().map(|u| u.trim().trim_end_matches('/').to_string()).filter(|u| !u.is_empty()).unwrap_or_else(|| self.base_url.into())
    }

    /// Its API key: from [`Provider::key_env`], else saved with [`set_api_key`].
    pub fn api_key(&self) -> Option<String> {
        self.key_source().map(|(key, _)| key)
    }

    /// Whether its key works, asking something free of it.
    pub fn check_key(&'static self) -> Result<(), CloudError> {
        let key = self.key_or_none()?;
        match self.api {
            Api::OpenAi if self.any_model => openai::list_models(&self.client(&ANY_MODEL, key)).map(|_| ()),
            Api::OpenAi => match self.models.first() {
                Some(model) => self.client(model, key).check(),
                None => Ok(()),
            },
            Api::ElevenLabs => elevenlabs::check_key(&key),
            Api::Anthropic => anthropic::check_key(&key),
        }
    }

    /// Its key, or an empty one when it needs none.
    fn key_or_none(&self) -> Result<String, CloudError> {
        match self.api_key() {
            Some(key) => Ok(key),
            None if !self.needs_key => Ok(String::new()),
            None => Err(CloudError::NoKey { provider: self.name, env: self.key_env }),
        }
    }

    fn client(&'static self, model: &'static CloudModel, key: String) -> Client {
        Client { model: CloudRef { provider: self, model }, key, voice_id: None, name: None }
    }

    /// Its API key and where it came from.
    pub fn key_source(&self) -> Option<(String, KeySource)> {
        if let Some(key) = std::env::var(self.key_env).ok().map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
            return Some((key, KeySource::Env));
        }
        saved_keys().remove(self.id).map(|k| (k, KeySource::Saved))
    }
}

/// Where a [`Provider`]'s key came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum KeySource {
    /// Its environment variable, which wins over a saved one.
    Env,
    /// `keys.json`.
    Saved,
}

/// A cloud model and its provider: what a `provider:model` id names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloudRef {
    pub provider: &'static Provider,
    pub model: &'static CloudModel,
}

impl CloudRef {
    /// `groq:hannah` → Groq's Hannah. `None` for a local id (no `:`) or one
    /// that isn't known. Any `elevenlabs:voice/<voice_id>` is known, and any
    /// `ollama:<model>`.
    pub fn by_id(id: &str) -> Option<Self> {
        let (provider, model) = id.split_once(':')?;
        let provider = Provider::by_id(provider)?;
        if provider.api == Api::ElevenLabs && account_voice_id(model).is_some() {
            return Some(Self { provider, model: &ACCOUNT_VOICE });
        }
        if provider.any_model && !model.is_empty() {
            return Some(Self { provider, model: &ANY_MODEL });
        }
        let model = provider.models.iter().find(|m| m.id == model)?;
        Some(Self { provider, model })
    }

    /// Every cloud model of `kind`.
    pub fn of_kind(kind: Kind) -> impl Iterator<Item = CloudRef> {
        PROVIDERS.iter().flat_map(|provider| provider.models.iter().map(move |model| CloudRef { provider, model })).filter(move |r| r.model.kind == kind)
    }

    /// Its `provider:model` id.
    pub fn id(&self) -> String {
        format!("{}:{}", self.provider.id, self.model.id)
    }
}

/// The `<voice_id>` of `voice/<voice_id>`.
fn account_voice_id(model: &str) -> Option<&str> {
    model.strip_prefix("voice/").filter(|v| !v.is_empty())
}

/// Whether `id` names a cloud model (or tries to: it has a `:`).
pub fn is_cloud_id(id: &str) -> bool {
    id.contains(':')
}

/// The chat models `provider` has now (Ollama's: the ones the person pulled).
pub fn list_models(provider: &'static Provider) -> Result<Vec<String>, CloudError> {
    let key = provider.key_or_none()?;
    openai::list_models(&provider.client(&ANY_MODEL, key))
}

fn keys_file() -> Option<PathBuf> {
    crate::claude::data_dir().map(|d| d.join("keys.json"))
}

fn saved_keys() -> BTreeMap<String, String> {
    keys_file().and_then(|f| fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Save `provider`'s API key in `keys.json` (`None` forgets it), readable
/// only by this user.
pub fn set_api_key(provider: &Provider, key: Option<&str>) -> io::Result<()> {
    let file = keys_file().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data folder"))?;
    let mut keys = saved_keys();
    match key.map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => keys.insert(provider.id.to_string(), k.to_string()),
        None => keys.remove(provider.id),
    };
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_vec_pretty(&keys).map_err(io::Error::other)?;
    let tmp = file.with_extension("json.tmp");
    write_private(&tmp, &json)?;
    fs::rename(tmp, file)
}

#[cfg(unix)]
pub(crate) fn write_private(path: &std::path::Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    f.write_all(bytes)
}

#[cfg(not(unix))]
pub(crate) fn write_private(path: &std::path::Path, bytes: &[u8]) -> io::Result<()> {
    fs::write(path, bytes)
}

/// What went wrong talking to a provider. Its message starts with the
/// provider's name.
#[derive(Debug, Clone, PartialEq)]
pub enum CloudError {
    /// No key for the provider.
    NoKey { provider: &'static str, env: &'static str },
    /// The key was refused (401, 403).
    BadKey { provider: &'static str },
    /// Too many requests (429): how long to wait, when it said, and whether
    /// what ran out is the day's allowance rather than the minute's.
    RateLimited { provider: &'static str, retry_after: Option<Duration>, per_day: bool },
    /// Any other error status, with the provider's message.
    Status { provider: &'static str, status: u16, message: String },
    /// It couldn't be reached.
    Network { provider: &'static str, why: String },
    /// It answered something unexpected.
    Invalid { provider: &'static str, why: String },
}

impl fmt::Display for CloudError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CloudError::NoKey { provider, env } => write!(f, "{provider}: no API key (set {env}, or add one in the app)"),
            CloudError::BadKey { provider } => write!(f, "{provider}: the API key was refused"),
            CloudError::RateLimited { provider, retry_after, per_day } => {
                let what = if *per_day { "the day's limit" } else { "the rate limit" };
                match retry_after {
                    Some(d) => write!(f, "{provider}: {what} was reached, try again in {}", wait_text(*d)),
                    None => write!(f, "{provider}: {what} was reached"),
                }
            }
            CloudError::Status { provider, status, message } => write!(f, "{provider}: {status} {message}"),
            CloudError::Network { provider, why } => write!(f, "{provider}: {why}"),
            CloudError::Invalid { provider, why } => write!(f, "{provider}: {why}"),
        }
    }
}

impl std::error::Error for CloudError {}

impl CloudError {
    /// The provider's limits were hit: a local model should answer instead.
    pub fn is_limit(&self) -> bool {
        matches!(self, CloudError::RateLimited { .. })
    }
}

/// A wait for people: `40 s`, `12 min`, `3 h 5 min`.
pub fn wait_text(d: Duration) -> String {
    let s = d.as_secs().max(1);
    match s {
        0..=59 => format!("{s} s"),
        60..=3599 => format!("{} min", s.div_ceil(60)),
        _ => format!("{} h {} min", s / 3600, (s % 3600) / 60),
    }
}

/// How long a model that hit its limits without saying when to retry is left alone.
const COOLDOWN: Duration = Duration::from_secs(60);

/// The models that hit their limits, by `provider:model` id: until when, and
/// whether it was the day's limit.
static LIMITED: Mutex<BTreeMap<String, (Instant, bool)>> = Mutex::new(BTreeMap::new());

/// A transcription: the text, its language as the provider names it
/// (`italian`, `english`) and `segments` with `avg_logprob`, like
/// whisper-server's `verbose_json`.
pub type TranscriptionReply = Value;

/// One cloud model and the key to use it with. Its [`fmt::Debug`] leaves
/// the key out.
#[derive(Clone, PartialEq, Eq)]
pub struct Client {
    pub model: CloudRef,
    key: String,
    /// The account's voice an `elevenlabs:voice/<voice_id>` names.
    pub voice_id: Option<String>,
    /// The model an `ollama:<model>` names, which isn't in the catalog.
    pub name: Option<String>,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client").field("model", &self.model.id()).finish_non_exhaustive()
    }
}

impl Client {
    /// The cloud model `id` (`groq:hannah`) of `kind`, with its provider's key.
    pub fn new(id: &str, kind: Kind) -> Result<Self, String> {
        let model = CloudRef::by_id(id).filter(|r| r.model.kind == kind).ok_or_else(|| format!("unknown cloud model {id}"))?;
        let key = model.provider.key_or_none().map_err(|e| e.to_string())?;
        let voice_id = id.split_once(':').and_then(|(_, m)| account_voice_id(m)).filter(|_| model.provider.api == Api::ElevenLabs).map(str::to_string);
        let name = id.split_once(':').map(|(_, m)| m.to_string()).filter(|_| model.provider.any_model);
        Ok(Self { model, key, voice_id, name })
    }

    /// The provider's name for the model: the catalog's, or the one the id gave.
    pub fn model_name(&self) -> &str {
        self.name.as_deref().unwrap_or(self.model.model.model)
    }

    /// Check the key and that the service answers (lists its models: free).
    pub fn check(&self) -> Result<(), CloudError> {
        match self.model.provider.api {
            Api::OpenAi => openai::check(self),
            Api::ElevenLabs => elevenlabs::check(self),
            Api::Anthropic => anthropic::check_key(&self.key),
        }
    }


    /// A chat completion: OpenAI's request (`messages`, `tools`, sampling);
    /// the first choice's message. Settings only llama.cpp knows are left
    /// out, and the model's [`CloudModel::params`] added.
    pub fn chat(&self, request: &Value) -> Result<Value, CloudError> {
        self.limited(|| match self.model.provider.api {
            Api::OpenAi => openai::chat(self, request),
            Api::ElevenLabs | Api::Anthropic => Err(self.unsupported("chat")),
        })
    }

    /// Transcribe a WAV, in `language` (`it`, `en`) or whichever it hears (`None`).
    pub fn transcribe(&self, wav: &[u8], language: Option<&str>, prompt: &str) -> Result<TranscriptionReply, CloudError> {
        self.limited(|| match self.model.provider.api {
            Api::OpenAi => openai::transcribe(self, wav, language, prompt),
            Api::ElevenLabs | Api::Anthropic => Err(self.unsupported("transcription")),
        })
    }

    /// Speak `text`, in `language` (`it`, `en`) where the voice speaks more
    /// than one, with the voice: a WAV.
    pub fn speak(&self, text: &str, language: Option<&str>) -> Result<Vec<u8>, CloudError> {
        self.limited(|| match self.model.provider.api {
            Api::OpenAi => openai::speak(self, text),
            Api::ElevenLabs => elevenlabs::speak(self, text, language),
            Api::Anthropic => Err(self.unsupported("speech")),
        })
    }

    fn unsupported(&self, what: &str) -> CloudError {
        CloudError::Invalid { provider: self.model.provider.name, why: format!("no {what} here") }
    }

    /// Whether the model is left alone after hitting its limits.
    pub fn is_limited(&self) -> bool {
        self.limit().is_some()
    }

    /// The limit the model is waiting out, as the error its requests get.
    pub fn limit(&self) -> Option<CloudError> {
        let mut limited = LIMITED.lock().unwrap();
        let id = self.model.id();
        let left = limited.get(&id).and_then(|(until, per_day)| Some((until.checked_duration_since(Instant::now())?, *per_day)));
        if left.is_none() {
            limited.remove(&id);
        }
        left.map(|(left, per_day)| CloudError::RateLimited { provider: self.model.provider.name, retry_after: Some(left), per_day })
    }

    /// `request`, unless the model hit its limits and it's not yet time to
    /// retry; a limit it hits now is remembered.
    fn limited<T>(&self, request: impl FnOnce() -> Result<T, CloudError>) -> Result<T, CloudError> {
        if let Some(limit) = self.limit() {
            return Err(limit);
        }
        let reply = request();
        if let Err(CloudError::RateLimited { retry_after, per_day, .. }) = &reply {
            LIMITED.lock().unwrap().insert(self.model.id(), (Instant::now() + retry_after.unwrap_or(COOLDOWN), *per_day));
        }
        reply
    }
}

/// `text` in pieces of at most `max` characters (0: one piece), split
/// between words: give it a sentence at a time.
pub fn chunks(text: &str, max: usize) -> Vec<String> {
    let text = text.trim();
    if max == 0 || text.chars().count() <= max {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let len = current.chars().count();
        if len > 0 && len + 1 + word.chars().count() > max {
            out.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        // A word longer than the limit on its own: cut it.
        let mut word: Vec<char> = word.chars().collect();
        while word.len() > max {
            out.push(word.drain(..max).collect());
        }
        current.extend(word);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        let r = CloudRef::by_id("groq:hannah").unwrap();
        assert_eq!((r.provider.id, r.model.kind, r.model.voice), ("groq", Kind::Voice, Some("hannah")));
        assert_eq!(r.id(), "groq:hannah");
        assert_eq!(CloudRef::by_id("groq:openai/gpt-oss-20b").map(|r| r.model.kind), Some(Kind::Llm));
        assert!(CloudRef::by_id("small").is_none());
        assert!(CloudRef::by_id("nobody:small").is_none());
        // Any voice of an ElevenLabs account, by its id.
        let r = CloudRef::by_id("elevenlabs:voice/JBFqnCBsd6RMkjVDRZzb").unwrap();
        assert_eq!((r.provider.id, r.model.kind), ("elevenlabs", Kind::Voice));
        assert!(CloudRef::by_id("elevenlabs:voice/").is_none());
        assert!(CloudRef::by_id("groq:voice/x").is_none());
    }

    #[test]
    fn catalog_is_consistent() {
        for p in PROVIDERS {
            // A key only ever goes over HTTPS; a server on this computer needs none.
            assert!(!p.id.contains(':') && (p.base_url.starts_with("https://") || (!p.needs_key && p.base_url.starts_with("http://localhost"))), "{}", p.id);
            for m in p.models {
                assert_eq!(p.models.iter().filter(|o| o.id == m.id).count(), 1, "{} twice", m.id);
                assert_eq!(m.kind == Kind::Voice, m.voice.is_some(), "{}", m.id);
                // A voice of one language says so, but for ElevenLabs': theirs speak both.
                assert_eq!(m.kind == Kind::Voice && p.api == Api::OpenAi, m.language.is_some(), "{}", m.id);
                assert!(m.params.is_empty() || serde_json::from_str::<serde_json::Map<String, Value>>(m.params).is_ok(), "{}", m.id);
            }
        }
        // Local ids never look like cloud ones.
        assert!(crate::models::MODELS.iter().all(|m| !is_cloud_id(m.id)));
    }

    #[test]
    fn a_limit_is_remembered() {
        let client = Client { model: CloudRef::by_id("groq:whisper-large-v3").unwrap(), key: "k".into(), voice_id: None, name: None };
        let limit = || CloudError::RateLimited { provider: "Groq", retry_after: Some(Duration::from_secs(30)), per_day: true };
        assert!(!client.is_limited());
        assert_eq!(client.limited(|| Err::<(), _>(limit())), Err(limit()));
        assert!(client.is_limited());
        assert!(matches!(client.limit(), Some(CloudError::RateLimited { per_day: true, .. })));
        // Not asked again while it's limited.
        let reply = client.limited(|| -> Result<(), CloudError> { panic!("asked") });
        assert!(reply.is_err_and(|e| e.is_limit()));
        // Only its own: other tests keep limits of their own models.
        LIMITED.lock().unwrap().remove(&client.model.id());
        assert_eq!(client.limited(|| Ok(1)), Ok(1));
    }

    #[test]
    fn waits_read_well() {
        assert_eq!(wait_text(Duration::from_secs(40)), "40 s");
        assert_eq!(wait_text(Duration::from_secs(61)), "2 min");
        assert_eq!(wait_text(Duration::from_secs(3 * 3600 + 300)), "3 h 5 min");
        let e = CloudError::RateLimited { provider: "Groq", retry_after: Some(Duration::from_secs(90)), per_day: true };
        assert_eq!(e.to_string(), "Groq: the day's limit was reached, try again in 2 min");
    }

    #[test]
    fn chunks_fit() {
        assert_eq!(chunks("Short.", 200), ["Short."]);
        let long = "word ".repeat(100);
        let parts = chunks(&long, 42);
        assert!(parts.iter().all(|p| p.chars().count() <= 42));
        assert_eq!(parts.join(" "), long.trim());
        assert_eq!(chunks(&"x".repeat(10), 4), ["xxxx", "xxxx", "xx"]);
    }
}
