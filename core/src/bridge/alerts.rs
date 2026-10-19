//! What the host says of its own accord, on a thread of its own: Claude Code
//! alerts ([`crate::claude::alerts`]) and timers that are up, each once the
//! board is out of any voice conversation.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::assistant::{InboxChange, Notification, Urgency};
use crate::claude::ClaudeMetrics;
use crate::claude::alerts::{self, Alert, AlertSettings, Alerts, HookEvent};
use crate::link::Link;
use crate::profile::BoardKind;
use crate::timers::Fired;
use crate::voice::Speaker;

use super::floor::Floor;
use super::{BridgeConfig, BridgeEvent, EventSink, sleep_unless_stopped};

/// What the alerts thread gets: a hook event (with when it came) or the
/// Claude usage of a snapshot.
pub(super) enum AlertInput {
    Hook(HookEvent, Instant),
    Metrics(ClaudeMetrics),
    /// A timer that's up, to be said.
    Timer(Fired),
    /// The assistant's inbox changed.
    Inbox(InboxChange),
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
/// floor between them. Alerts and timers go to one board only
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

    /// Every board that's ready.
    pub fn all(&self) -> Vec<AlertTarget> {
        self.boards.lock().unwrap().iter().map(|(_, t)| t.clone()).collect()
    }

    /// Session `session` ended.
    pub fn remove(&self, session: u64) {
        self.boards.lock().unwrap().retain(|(n, _)| *n != session);
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
        if let AlertInput::Inbox(change) = &input {
            match change {
                InboxChange::New(n) => {
                    let error = deliver_notification(config, stop, targets, n).err();
                    on_event(BridgeEvent::Notified { id: n.id.clone(), error });
                }
                InboxChange::Removed(id) => mirror(targets, "hide_notification", json!({"id": id})),
                InboxChange::Cleared => mirror(targets, "clear_notifications", json!({})),
            }
            continue;
        }
        let alert = match &input {
            AlertInput::Hook(event, at) => alerts.on_hook(event, &settings, *at),
            AlertInput::Metrics(metrics) => alerts.on_metrics(metrics, &settings),
            AlertInput::Inbox(_) => continue,
            AlertInput::Timer(fired) => {
                let text = fired.spoken();
                let error = say_timer(config, stop, targets, fired).err();
                on_event(BridgeEvent::TimerFired { fired: fired.clone(), text, error });
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

/// Put `n` on every board's inbox; an urgent one is also said, by the board
/// that speaks. In quiet hours anything less than urgent arrives silently.
fn deliver_notification(config: &BridgeConfig, stop: &AtomicBool, targets: &Targets, n: &Notification) -> Result<(), String> {
    let quiet = config.assistant.as_ref().is_some_and(|a| a.quiet_now());
    let urgency = if quiet && n.urgency < Urgency::Urgent { Urgency::Low } else { n.urgency };
    let boards: Vec<Arc<Link>> = targets.all().into_iter().filter_map(|t| t.link.upgrade()).collect();
    if boards.is_empty() {
        return Err("the board isn't connected".into());
    }
    let args = notification_args(n, urgency);
    for link in &boards {
        // Firmware before 0.10.0 doesn't have the inbox.
        let _ = link.call_tool("show_notification", args.clone(), super::TOOL_TIMEOUT);
    }
    if urgency == Urgency::Urgent {
        let Some(tts) = config.voice.lock().unwrap().tts.clone() else { return Ok(()) };
        let (link, AlertTarget { speaker, voice_state, .. }) = self::speaker(config, targets, None).ok_or("the board isn't connected")?;
        let until = Instant::now() + CONVERSATION_WAIT;
        while *voice_state.lock().unwrap() != "idle" && Instant::now() < until && !stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(250));
        }
        let text = if n.text.is_empty() { n.title.clone() } else { format!("{}. {}", n.title, n.text) };
        let language = crate::intents::guess_language(&text);
        speaker.speak(&link, &tts, &text, language)?;
    }
    Ok(())
}

/// Call `tool` on every board that's ready.
fn mirror(targets: &Targets, tool: &str, args: serde_json::Value) {
    for link in targets.all().into_iter().filter_map(|t| t.link.upgrade()) {
        let _ = link.call_tool(tool, args.clone(), super::TOOL_TIMEOUT);
    }
}

/// `show_notification`'s arguments for `n`, at `urgency`.
pub(super) fn notification_args(n: &Notification, urgency: Urgency) -> serde_json::Value {
    let mut args = json!({"id": n.id, "source": n.source, "title": n.title, "text": n.text, "urgency": urgency.name()});
    if n.url.is_some() {
        args["actions"] = json!(["Open"]);
    }
    args
}

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
