//! The cards the assistant waits on before it does something that changes
//! things (sends a mail, creates an event): [`Confirmations::ask`] holds the
//! turn until the user answers in the app or on the board
//! ([`Confirmations::answer`]), or gives up after a while.
//!
//! The bridge shows each card on the board (`show_approval`, taken away with
//! `hide_approval`) and takes the board's `approval` notification as the
//! answer; the app shows the same cards in the conversation.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Length of a card's text on the board.
const TEXT: usize = 72;

/// A card waiting for an answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Confirmation {
    /// Ours, to answer it by.
    pub id: String,
    /// What would run: the service and its tool (`linear: create_issue`).
    pub tool: String,
    /// What it would do, short, for the board's card.
    pub text: String,
    /// The tool's input in full, for the app.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Unix ms.
    pub asked: u64,
    /// When it stops waiting, Unix ms.
    pub expires: u64,
}

/// Told every time the cards change, with all those waiting (oldest first).
pub type ConfirmListener = Arc<dyn Fn(Vec<Confirmation>) + Send + Sync>;

struct Pending {
    card: Confirmation,
    answer: Option<bool>,
}

#[derive(Default)]
struct Inner {
    pending: Vec<Pending>,
    next: u64,
}

/// The cards waiting. Shared between the assistant's hosts (which ask), the
/// bridge (which shows them on the board) and the app (which shows and answers).
#[derive(Default)]
pub struct Confirmations {
    inner: Mutex<Inner>,
    answered: Condvar,
    listener: Mutex<Option<ConfirmListener>>,
}

impl std::fmt::Debug for Confirmations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Confirmations").finish_non_exhaustive()
    }
}

impl Confirmations {
    /// The cards still waiting, oldest first.
    pub fn view(&self) -> Vec<Confirmation> {
        self.inner.lock().unwrap().pending.iter().filter(|p| p.answer.is_none()).map(|p| p.card.clone()).collect()
    }

    pub fn set_listener(&self, listener: Option<ConfirmListener>) {
        *self.listener.lock().unwrap() = listener;
    }

    /// Ask the user to allow `tool` (`text` on the board, `detail` in the
    /// app); `None` when nobody answered within `wait`.
    pub fn ask(&self, tool: &str, text: &str, detail: Option<&str>, wait: Duration) -> Option<bool> {
        let now = SystemTime::now();
        let id = {
            let mut inner = self.inner.lock().unwrap();
            inner.next += 1;
            let id = format!("y{}", inner.next);
            let card = Confirmation {
                id: id.clone(),
                tool: clip(tool, 40),
                text: clip(text, TEXT),
                detail: detail.map(|d| clip(d, 2000)),
                asked: unix_ms(now),
                expires: unix_ms(now + wait),
            };
            inner.pending.push(Pending { card, answer: None });
            id
        };
        self.changed();
        let until = Instant::now() + wait;
        let mut inner = self.inner.lock().unwrap();
        let answer = loop {
            let Some(at) = inner.pending.iter().position(|p| p.card.id == id) else { break None };
            if let Some(answer) = inner.pending[at].answer {
                inner.pending.remove(at);
                break Some(answer);
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                inner.pending.remove(at);
                break None;
            }
            inner = self.answered.wait_timeout(inner, left).unwrap().0;
        };
        drop(inner);
        self.changed();
        answer
    }

    /// Answer card `id`; false when it's no longer waiting.
    pub fn answer(&self, id: &str, allow: bool) -> bool {
        let found = {
            let mut inner = self.inner.lock().unwrap();
            match inner.pending.iter_mut().find(|p| p.card.id == id && p.answer.is_none()) {
                Some(p) => {
                    p.answer = Some(allow);
                    true
                }
                None => false,
            }
        };
        self.answered.notify_all();
        found
    }

    fn changed(&self) {
        let listener = self.listener.lock().unwrap().clone();
        if let Some(listener) = listener {
            listener(self.view());
        }
    }
}

/// At most `max` characters, with an ellipsis when cut.
fn clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", text[..at].trim_end()),
        None => text.to_string(),
    }
}

fn unix_ms(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    #[test]
    fn a_card_waits_for_its_answer() {
        let cards = Arc::new(Confirmations::default());
        let asking = {
            let cards = cards.clone();
            thread::spawn(move || cards.ask("linear: create_issue", "Create issue \"Swipe\"", Some("{}"), Duration::from_secs(5)))
        };
        let card = loop {
            if let Some(c) = cards.view().first().cloned() {
                break c;
            }
            thread::sleep(Duration::from_millis(5));
        };
        assert_eq!((card.tool.as_str(), card.detail.as_deref()), ("linear: create_issue", Some("{}")));
        assert!(cards.answer(&card.id, false));
        assert_eq!(asking.join().unwrap(), Some(false));
        assert!(cards.view().is_empty());
        assert!(!cards.answer(&card.id, true));
    }

    #[test]
    fn a_card_gives_up() {
        let cards = Confirmations::default();
        assert_eq!(cards.ask("x", "y", None, Duration::from_millis(30)), None);
        assert!(cards.view().is_empty());
        assert_eq!(clip("abcdef", 3), "abc…");
    }
}
