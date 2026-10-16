//! Claude Code's sessions and permission requests ([`crate::claude::agents`]),
//! from the hooks to the frontend and the boards. The hub hands the
//! `PermissionRequest`s to [`Agents::ask`] while approvals are on; every change
//! reaches [`agents_loop`], which raises [`BridgeEvent::Agents`] and shows a
//! new request on each board whose profile has `approvals` (Yoru), with
//! `show_approval`, until it's answered or given up (`hide_approval`). The
//! board answers with an `approval` notification ([`on_notification`]).

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::claude::agents::{Agents, AgentsView, Approval, Decision};

use super::alerts::Targets;
use super::{BridgeConfig, BridgeEvent, EventSink, TOOL_TIMEOUT};

/// Hand the hub's permission requests to the agents, and the agents' changes
/// to [`agents_loop`].
pub(super) fn listen(config: &BridgeConfig) -> Receiver<AgentsView> {
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    config.agents.set_listener(Some(Arc::new(move |view| {
        let _ = tx.lock().unwrap().send(view);
    })));
    if let Some(hub) = &config.hub {
        let (agents, settings) = (config.agents.clone(), config.claude_alerts.clone());
        hub.set_claude_permission(Some(Arc::new(move |json: Value| {
            let wait = settings.lock().unwrap().approval_wait()?;
            agents.ask(&json, wait)
        })));
    }
    rx
}

pub(super) fn agents_loop(stop: &AtomicBool, targets: &Targets, views: Receiver<AgentsView>, on_event: &EventSink) {
    let mut shown: HashSet<String> = HashSet::new();
    while !stop.load(Ordering::Relaxed) {
        let view = match views.recv_timeout(Duration::from_millis(200)) {
            Ok(view) => view,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        show_on_boards(targets, &view.approvals, &mut shown);
        on_event(BridgeEvent::Agents { sessions: view.sessions, approvals: view.approvals });
    }
}

/// Show the requests that are new on the boards that can answer them, and
/// take away the ones that are over.
fn show_on_boards(targets: &Targets, approvals: &[Approval], shown: &mut HashSet<String>) {
    let boards: Vec<_> = targets.all().into_iter().filter(|t| t.kind.profile().approvals).filter_map(|t| t.link.upgrade()).collect();
    let new: Vec<&Approval> = approvals.iter().filter(|a| !shown.contains(&a.id)).collect();
    for a in new {
        let args = show_args(a);
        for link in &boards {
            // Firmware without the tool says so; nothing to do then.
            let _ = link.call_tool("show_approval", args.clone(), TOOL_TIMEOUT);
        }
        shown.insert(a.id.clone());
    }
    let over: Vec<String> = shown.iter().filter(|id| !approvals.iter().any(|a| &a.id == *id)).cloned().collect();
    for id in over {
        for link in &boards {
            let _ = link.call_tool("hide_approval", json!({"id": id}), TOOL_TIMEOUT);
        }
        shown.remove(&id);
    }
}

fn show_args(a: &Approval) -> Value {
    let seconds = a.expires.saturating_sub(a.asked) / 1000;
    let mut args = json!({"id": a.id, "tool": a.tool, "text": a.text, "seconds": seconds});
    if let Some(project) = &a.project {
        args["project"] = json!(project);
    }
    args
}

/// The board's `approval` notification: `{"id", "allow"}`.
pub(super) fn on_notification(agents: &Agents, params: &Value) {
    let (Some(id), Some(allow)) = (params["id"].as_str(), params["allow"].as_bool()) else { return };
    agents.answer(id, if allow { Decision::Allow } else { Decision::Deny });
}
