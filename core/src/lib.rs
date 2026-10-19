//! Host side of Yoru, the personal assistant whose body is a round
//! ESP32-S3 board on the desk.
//!
//! [`Link`] speaks the board's protocol (framing in [`protocol`], JSON-RPC
//! and board tools), [`Bridge`] keeps the board connected on a background
//! thread and sends it the computer's [`Snapshot`], [`Hub`] shares that
//! connection with other processes and [`Board`] uses it (or the port, when
//! no bridge runs), and [`Esptool`] identifies and flashes the board.
//! [`firmware`] reads the version of a flash image and of the board's
//! firmware. The voice ([`voice`], [`Stt`], [`Tts`]) and the [`Agent`] answer
//! what Yoru hears; [`Timers`] keeps timers and reminders, [`ClaudeUsage`]
//! and [`claude`] follow Claude Code. The CLI and the Tauri app are thin
//! shells over this.

pub mod agent;
pub mod assistant;
pub mod bridge;
pub mod claude;
pub mod cloud;
#[cfg(feature = "connectors")]
pub mod connectors;
#[cfg(feature = "download")]
pub mod download;
pub mod firmware;
pub mod flasher;
pub mod hardware;
pub mod hub;
pub mod intents;
pub mod link;
pub mod llm;
pub mod models;
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod pointer;
pub mod presence;
pub mod profile;
pub mod protocol;
pub mod serial;
pub(crate) mod sidecar;
pub mod snapshot;
pub mod stt;
pub mod timers;
pub mod tts;
pub mod voice;
pub mod wifi;

pub use agent::{Agent, Toolbox, Turn};
pub use assistant::Assistant;
pub use bridge::{Bridge, BridgeConfig, BridgeEvent, Manager, ManagerEvent, Transport};
pub use claude::{ClaudeMetrics, ClaudeState, ClaudeUsage};
pub use firmware::{BoardFirmware, ImageInfo};
pub use flasher::{ChipInfo, Esptool, FlashEvent};
pub use hardware::{Hardware, Recommendation};
pub use hub::{Board, BoardInfo, Hub, HubStatus, Route};
pub use link::{CallError, Hello, Link, LinkEvent, Tool, ToolResult};
pub use llm::{Llm, LlmConfig};
pub use profile::{BoardKind, BoardProfile};
pub use serial::PortInfo;
pub use sidecar::keep_warm;
pub use snapshot::Snapshot;
pub use stt::{Stt, SttConfig, SttLanguage, Transcript};
pub use timers::{BoardTimer, ShowOn, TimerInfo, Timers};
pub use tts::{Tts, TtsConfig};
pub use voice::{Speaker, Spoken, Utterance, VoiceConfig};
