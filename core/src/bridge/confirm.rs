//! The assistant's cards ([`crate::assistant::Confirmations`]) on the boards:
//! every change reaches [`confirm_loop`], which raises
//! [`BridgeEvent::Confirmations`] and shows a new card on each board whose
//! profile has `approvals` (Yoru), with `show_approval`, until it's answered
//! or given up (`hide_approval`). The board answers with an `approval`
//! notification ([`on_notification`]).

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::assistant::{Confirmation, Confirmations};

use super::alerts::Targets;
use super::{BridgeConfig, BridgeEvent, EventSink, TOOL_TIMEOUT};

/// The cards' changes, for [`confirm_loop`].
pub(super) fn listen(config: &BridgeConfig) -> Receiver<Vec<Confirmation>> {
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    config.confirmations.set_listener(Some(Arc::new(move |cards| {
        let _ = tx.lock().unwrap().send(cards);
    })));
    rx
}

pub(super) fn confirm_loop(stop: &AtomicBool, targets: &Targets, changes: Receiver<Vec<Confirmation>>, on_event: &EventSink) {
    let mut shown: HashSet<String> = HashSet::new();
    while !stop.load(Ordering::Relaxed) {
        let cards = match changes.recv_timeout(Duration::from_millis(200)) {
            Ok(cards) => cards,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        show_on_boards(targets, &cards, &mut shown);
        on_event(BridgeEvent::Confirmations { cards });
    }
}

/// Show the cards that are new on the boards that can answer them, and take
/// away the ones that are over.
fn show_on_boards(targets: &Targets, cards: &[Confirmation], shown: &mut HashSet<String>) {
    let boards: Vec<_> = targets.all().into_iter().filter(|t| t.kind.profile().approvals).filter_map(|t| t.link.upgrade()).collect();
    let new: Vec<&Confirmation> = cards.iter().filter(|c| !shown.contains(&c.id)).collect();
    for card in new {
        let args = show_args(card);
        for link in &boards {
            // Firmware without the tool says so; nothing to do then.
            let _ = link.call_tool("show_approval", args.clone(), TOOL_TIMEOUT);
        }
        shown.insert(card.id.clone());
    }
    let over: Vec<String> = shown.iter().filter(|id| !cards.iter().any(|c| &c.id == *id)).cloned().collect();
    for id in over {
        for link in &boards {
            let _ = link.call_tool("hide_approval", json!({"id": id}), TOOL_TIMEOUT);
        }
        shown.remove(&id);
    }
}

fn show_args(card: &Confirmation) -> Value {
    let seconds = card.expires.saturating_sub(card.asked) / 1000;
    json!({"id": card.id, "tool": card.tool, "text": card.text, "seconds": seconds, "project": "Yoru"})
}

/// The board's `approval` notification: `{"id", "allow"}`.
pub(super) fn on_notification(confirmations: &Confirmations, params: &Value) {
    let (Some(id), Some(allow)) = (params["id"].as_str(), params["allow"].as_bool()) else { return };
    confirmations.answer(id, allow);
}
