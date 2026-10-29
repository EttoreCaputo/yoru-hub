//! Yoru's errands: while the assistant calls one of the user's services, every
//! board shows Yoru going through a portal to fetch it (`show_fetch`,
//! firmware 0.11.0), and coming back when the call is over. The board decides
//! whether it can show one now and joins calls that overlap.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;

use crate::assistant::Errand;

use super::alerts::Targets;
use super::{BridgeConfig, TOOL_TIMEOUT};

/// The assistant's errands, for [`errand_loop`]; `None` without an assistant.
pub(super) fn listen(config: &BridgeConfig) -> Option<Receiver<Errand>> {
    let assistant = config.assistant.as_ref()?;
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    assistant.set_errands(Some(Arc::new(move |errand| {
        let _ = tx.lock().unwrap().send(errand);
    })));
    Some(rx)
}

/// Show each errand on the boards: on a thread of its own, so a slow board
/// never holds up the model's turn.
pub(super) fn errand_loop(stop: &AtomicBool, targets: &Targets, errands: Receiver<Errand>) {
    while !stop.load(Ordering::Relaxed) {
        let errand = match errands.recv_timeout(Duration::from_millis(200)) {
            Ok(errand) => errand,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let args = json!({"state": errand.state, "id": errand.id, "source": clip(&errand.source, 24)});
        for link in targets.all().into_iter().filter(|t| t.kind.profile().approvals).filter_map(|t| t.link.upgrade()) {
            // Firmware before 0.11.0 doesn't have the tool; nothing to do then.
            let _ = link.call_tool("show_fetch", args.clone(), TOOL_TIMEOUT);
        }
    }
}

/// At most `max` bytes, on a character boundary.
fn clip(text: &str, max: usize) -> String {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}
