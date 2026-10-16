//! What the host says of its own accord, on a thread of its own: Claude Code
//! alerts ([`crate::claude::alerts`]), timers that are up, and the pet's
//! comments on your day ([`crate::quips`]), each once the board is out of
//! any voice conversation.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::claude::ClaudeMetrics;
use crate::claude::alerts::{self, Alert, AlertSettings, Alerts, HookEvent};
use crate::link::Link;
use crate::profile::BoardKind;
use crate::quips::Quip;
use crate::timers::Fired;
use crate::voice::Speaker;

use super::floor::Floor;
use super::{BridgeConfig, BridgeEvent, EventSink, TOOL_TIMEOUT, sleep_unless_stopped};

/// What the alerts thread gets: a hook event (with when it came) or the
/// Claude usage of a snapshot.
pub(super) enum AlertInput {
    Hook(HookEvent, Instant),
    Metrics(ClaudeMetrics),
    /// A timer that's up, to be said.
    Timer(Fired),
    /// The pet has something to say about your day.
    Quip(Quip),
}

/// A board the alerts can go to, while its session has it ready.
#[derive(Clone)]
pub(super) struct AlertTarget {
    /// The board's id, from its `hello`.
    pub id: Option<String>,
    pub link: Weak<Link>,
    pub kind: BoardKind,
    pub speaker: Arc<Speaker>,
    /// What the board's voice overlay shows, from its `voice_state`.
    pub voice_state: Arc<Mutex<String>>,
}

/// The boards that are ready, each under its session's number, and the voice
/// floor between them. Alerts, timers and quips go to one board only
/// ([`Targets::speaker`]).
#[derive(Default)]
pub(super) struct Targets {
    boards: Mutex<Vec<(u64, AlertTarget)>>,
    pub floor: Arc<Floor>,
}

impl Targets {
    /// Session `session`'s board is ready (again).
    pub fn set(&self, session: u64, target: AlertTarget) {
        let mut boards = self.boards.lock().unwrap();
        match boards.iter_mut().find(|(n, _)| *n == session) {
            Some(slot) => slot.1 = target,
            None => boards.push((session, target)),
        }
    }

    /// Session `session` ended.
    pub fn remove(&self, session: u64) {
        self.boards.lock().unwrap().retain(|(n, _)| *n != session);
    }

    /// Some board's voice overlay isn't idle: someone is talking with it.
    pub fn in_conversation(&self) -> bool {
        self.boards.lock().unwrap().iter().any(|(_, t)| *t.voice_state.lock().unwrap() != "idle")
    }

    /// The board that speaks: the one `origin` names (where a timer was
    /// set), else the `primary` board, else the one of the latest
    /// conversation, else the one connected first.
    pub fn speaker(&self, origin: Option<&str>, primary: Option<&str>) -> Option<AlertTarget> {
        choose(&self.boards.lock().unwrap(), origin, primary, self.floor.last()).cloned()
    }
}

fn choose<'a>(boards: &'a [(u64, AlertTarget)], origin: Option<&str>, primary: Option<&str>, last: Option<u64>) -> Option<&'a AlertTarget> {
    let by_id = |id: Option<&str>| id.and_then(|id| boards.iter().find(|(_, t)| t.id.as_deref() == Some(id)));
    by_id(origin)
        .or_else(|| by_id(primary))
        .or_else(|| last.and_then(|n| boards.iter().find(|(s, _)| *s == n)))
        .or_else(|| boards.first())
        .map(|(_, t)| t)
}

/// The board that says something now, with what it takes to say it.
fn speaker(config: &BridgeConfig, targets: &Targets, origin: Option<&str>) -> Option<(Arc<Link>, AlertTarget)> {
    let primary = config.primary.lock().unwrap().clone();
    let t = targets.speaker(origin, primary.as_deref())?;
    Some((t.link.upgrade()?, t))
}

/// An alert waits this long at most for a voice conversation to end.
const CONVERSATION_WAIT: Duration = Duration::from_secs(20);

pub(super) fn alert_loop(config: &BridgeConfig, stop: &AtomicBool, targets: &Targets, rx: Receiver<AlertInput>, on_event: &EventSink) {
    let mut alerts = Alerts::default();
    while !stop.load(Ordering::Relaxed) {
        let input = match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(input) => input,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let settings = config.claude_alerts.lock().unwrap().clone();
        let alert = match &input {
            AlertInput::Hook(event, at) => alerts.on_hook(event, &settings, *at),
            AlertInput::Metrics(metrics) => alerts.on_metrics(metrics, &settings),
            AlertInput::Timer(fired) => {
                let text = fired.spoken();
                let error = say_timer(config, stop, targets, fired).err();
                on_event(BridgeEvent::TimerFired { fired: fired.clone(), text, error });
                continue;
            }
            AlertInput::Quip(quip) => {
                let (text, error) = say_quip(config, stop, targets, quip, &settings.language);
                on_event(BridgeEvent::Quip { quip: quip.clone(), text, error });
                continue;
            }
        };
        let Some(alert) = alert else { continue };
        let error = give_alert(config, stop, targets, &settings, &alert).err();
        let text = alert.words(&settings.language).spoken;
        on_event(BridgeEvent::ClaudeAlert { alert, text, error });
    }
}

/// Wait for the board to be out of a conversation, then show and say `alert`.
fn give_alert(config: &BridgeConfig, stop: &AtomicBool, targets: &Targets, settings: &AlertSettings, alert: &Alert) -> Result<(), String> {
    let (link, AlertTarget { speaker, voice_state, .. }) = self::speaker(config, targets, None).ok_or("the board isn't connected")?;
    let until = Instant::now() + CONVERSATION_WAIT;
    while *voice_state.lock().unwrap() != "idle" && Instant::now() < until && !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(250));
    }
    let tts = if settings.speak { config.voice.lock().unwrap().tts.clone() } else { None };
    alerts::deliver(&link, &speaker, tts.as_deref(), alert, &settings.language)
}

/// Say what `fired` was for, after a chime or two, once the board is out of
/// any conversation; not if it was silenced meanwhile.
fn say_timer(config: &BridgeConfig, stop: &AtomicBool, targets: &Targets, fired: &Fired) -> Result<(), String> {
    let Some(tts) = config.voice.lock().unwrap().tts.clone() else { return Err("spoken replies are off".into()) };
    let (link, AlertTarget { speaker, voice_state, .. }) = self::speaker(config, targets, fired.timer.board.as_deref()).ok_or("the board isn't connected")?;
    // The first chime plays meanwhile (the board rings from the next snapshot).
    sleep_unless_stopped(TIMER_SPEECH_DELAY, stop);
    let until = Instant::now() + CONVERSATION_WAIT;
    while *voice_state.lock().unwrap() != "idle" && Instant::now() < until && !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(250));
    }
    if fired.pomodoro.is_none() && !config.timers.rings(fired.timer.id) {
        return Ok(());
    }
    speaker.speak(&link, &tts, &fired.spoken(), &fired.timer.language).map(|_| ())
}

/// Put `quip` in the pet's words (the language model's, or the fallback's)
/// and say it after a scene, once the board is out of any conversation:
/// what was said, and why it wasn't.
fn say_quip(config: &BridgeConfig, stop: &AtomicBool, targets: &Targets, quip: &Quip, language: &str) -> (String, Option<String>) {
    let (tts, agent) = {
        let voice = config.voice.lock().unwrap();
        (voice.tts.clone(), voice.agent.clone())
    };
    let board = speaker(config, targets, None);
    let kind = board.as_ref().map(|(_, t)| t.kind).unwrap_or_default();
    let text = agent.and_then(|a| a.quip(&quip.fact, language, kind).ok()).unwrap_or_else(|| quip.fallback(language));
    let Some(tts) = tts else { return (text, Some("spoken replies are off".into())) };
    let Some((link, AlertTarget { speaker, voice_state, .. })) = board else { return (text, Some("the board isn't connected".into())) };
    let until = Instant::now() + CONVERSATION_WAIT;
    while *voice_state.lock().unwrap() != "idle" && Instant::now() < until && !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(250));
    }
    if *voice_state.lock().unwrap() != "idle" {
        // Talking with it meanwhile: the moment's gone.
        return (text, Some("the board was in a conversation".into()));
    }
    if link.call_tool(kind.profile().scene_tool, json!({"name": quip.topic.scene()}), TOOL_TIMEOUT).is_ok_and(|r| !r.is_error) {
        // Speaking would cut the scene short.
        thread::sleep(QUIP_SCENE);
    }
    let error = speaker.speak(&link, &tts, &text, language).err();
    (text, error)
}

/// The scene before the words, long enough to be seen.
const QUIP_SCENE: Duration = Duration::from_millis(1500);

/// From a timer's end to saying why: the first chime's length and a breath.
const TIMER_SPEECH_DELAY: Duration = Duration::from_millis(2600);

#[cfg(test)]
mod tests {
    use super::*;

    fn board(id: &str) -> AlertTarget {
        AlertTarget { id: Some(id.into()), link: Weak::new(), kind: BoardKind::Yoru, speaker: Arc::default(), voice_state: Arc::default() }
    }

    #[test]
    fn one_board_speaks_in_order_of_preference() {
        let boards = vec![(1, board("first")), (2, board("primary")), (3, board("talked")), (4, board("timer"))];
        let id = |t: Option<&AlertTarget>| t.and_then(|t| t.id.clone());
        assert_eq!(id(choose(&boards, Some("timer"), Some("primary"), Some(3))).as_deref(), Some("timer"));
        assert_eq!(id(choose(&boards, None, Some("primary"), Some(3))).as_deref(), Some("primary"));
        // A board that left is skipped.
        assert_eq!(id(choose(&boards, Some("gone"), Some("gone"), Some(3))).as_deref(), Some("talked"));
        assert_eq!(id(choose(&boards, None, None, Some(9))).as_deref(), Some("first"));
        assert!(choose(&[], Some("timer"), None, None).is_none());
    }
}
