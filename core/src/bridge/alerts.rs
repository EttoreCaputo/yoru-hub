//! What the host says of its own accord, on a thread of its own: timers that
//! are up, the assistant's notifications and what the coding agents did,
//! each once the board is out of any voice conversation.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::assistant::{InboxChange, Notification, Urgency};
use crate::link::Link;
use crate::profile::BoardKind;
use crate::timers::Fired;
use crate::voice::Speaker;

use super::floor::Floor;
use super::{BridgeConfig, BridgeEvent, EventSink, sleep_unless_stopped};

/// What the alerts thread gets.
pub(super) enum AlertInput {
    /// A timer that's up, to be said.
    Timer(Fired),
    /// The assistant's inbox changed.
    Inbox(InboxChange),
    /// Something to say out loud, in a language (`it`, `en`): a coding
    /// agent finished or needs the user.
    Say { text: String, language: String },
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
/// floor between them. Timers and urgent notifications are said by one board only
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
    /// conversation, else the one connected first. A real board always
    /// comes before the notch (docs/notch.md): it rings, chimes and shows
    /// the timer, where the notch could only say it.
    pub fn speaker(&self, origin: Option<&str>, primary: Option<&str>) -> Option<AlertTarget> {
        choose(&self.boards.lock().unwrap(), origin, primary, self.floor.last()).cloned()
    }
}

fn choose<'a>(boards: &'a [(u64, AlertTarget)], origin: Option<&str>, primary: Option<&str>, last: Option<u64>) -> Option<&'a AlertTarget> {
    let real = |t: &AlertTarget| t.id.as_deref() != Some(crate::notch::ID);
    let pick = |real_only: bool| {
        let ok = |t: &AlertTarget| !real_only || real(t);
        let by_id = |id: Option<&str>| id.and_then(|id| boards.iter().find(|(_, t)| t.id.as_deref() == Some(id) && ok(t)));
        by_id(origin).or_else(|| by_id(primary)).or_else(|| last.and_then(|n| boards.iter().find(|(s, t)| *s == n && ok(t)))).or_else(|| boards.iter().find(|(_, t)| ok(t)))
    };
    pick(true).or_else(|| pick(false)).map(|(_, t)| t)
}

/// The board that says something now, with what it takes to say it.
fn speaker(config: &BridgeConfig, targets: &Targets, origin: Option<&str>) -> Option<(Arc<Link>, AlertTarget)> {
    let primary = config.primary.lock().unwrap().clone();
    let t = targets.speaker(origin, primary.as_deref())?;
    Some((t.link.upgrade()?, t))
}

/// Speaking waits this long at most for a voice conversation to end.
const CONVERSATION_WAIT: Duration = Duration::from_secs(20);

pub(super) fn alert_loop(config: &BridgeConfig, stop: &AtomicBool, targets: &Targets, rx: Receiver<AlertInput>, on_event: &EventSink) {
    while !stop.load(Ordering::Relaxed) {
        let input = match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(input) => input,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        match input {
            AlertInput::Inbox(InboxChange::New(n)) => {
                let error = deliver_notification(config, stop, targets, &n).err();
                on_event(BridgeEvent::Notified { id: n.id.clone(), error });
            }
            AlertInput::Inbox(InboxChange::Removed(id)) => mirror(targets, "hide_notification", json!({"id": id})),
            AlertInput::Inbox(InboxChange::Cleared) => mirror(targets, "clear_notifications", json!({})),
            AlertInput::Timer(fired) => {
                let text = fired.spoken();
                let error = say_timer(config, stop, targets, &fired).err();
                on_event(BridgeEvent::TimerFired { fired, text, error });
            }
            AlertInput::Say { text, language } => match say(config, stop, targets, &text, &language) {
                Ok(()) => crate::hostlog::line("said", &text),
                Err(e) => crate::hostlog::line("said", &format!("{text} (not said: {e})")),
            },
        }
    }
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

/// Say `text` once the board is out of any conversation, and show it in the
/// conversation; in quiet hours only show it (it's in the inbox too).
fn say(config: &BridgeConfig, stop: &AtomicBool, targets: &Targets, text: &str, language: &str) -> Result<(), String> {
    if let Some(assistant) = &config.assistant {
        assistant.said_unasked(text);
        if assistant.quiet_now() {
            return Err("quiet hours".into());
        }
    }
    let Some(tts) = config.voice.lock().unwrap().tts.clone() else { return Err("spoken replies are off".into()) };
    let (link, AlertTarget { speaker, voice_state, .. }) = self::speaker(config, targets, None).ok_or("the board isn't connected")?;
    let until = Instant::now() + CONVERSATION_WAIT;
    while *voice_state.lock().unwrap() != "idle" && Instant::now() < until && !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(250));
    }
    speaker.speak(&link, &tts, text, language).map(|_| ())
}

/// Put `n` on every board's inbox; an urgent one is also said, by the board
/// that speaks. In quiet hours anything less than urgent arrives silently, and
/// a source the user said «Enough» to arrives a step quieter.
fn deliver_notification(config: &BridgeConfig, stop: &AtomicBool, targets: &Targets, n: &Notification) -> Result<(), String> {
    let quiet = config.assistant.as_ref().is_some_and(|a| a.quiet_now());
    let urgency = if quiet && n.urgency < Urgency::Urgent { Urgency::Low } else { n.urgency };
    let urgency = config.assistant.as_ref().map_or(urgency, |a| a.memory.braked(n.by.as_deref(), &n.source, urgency));
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

    #[test]
    fn a_real_board_comes_before_the_notch() {
        let notch = crate::notch::ID;
        let boards = vec![(1, board(notch)), (2, board("desk"))];
        let id = |t: Option<&AlertTarget>| t.and_then(|t| t.id.clone());
        // A timer set on the notch, and the notch's conversation the latest: the board still rings.
        assert_eq!(id(choose(&boards, Some(notch), None, Some(1))).as_deref(), Some("desk"));
        // The notch alone says it.
        assert_eq!(id(choose(&boards[..1], Some(notch), None, Some(1))).as_deref(), Some(notch));
    }
}
