//! Drive every board found, on USB and WiFi, and print what each one does:
//! a check of the manager with real boards, until the CLI has it.
//!
//!     cargo run -p yoru-core --example boards [SECONDS]

use std::time::{Duration, Instant};

use yoru_core::{BridgeConfig, BridgeEvent, Manager};

fn main() {
    let seconds = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(15);
    let config = BridgeConfig::default();
    let started = Instant::now();
    let manager = Manager::spawn(config, None, move |e| {
        let who = match (&e.target, &e.board_kind, &e.board) {
            (None, ..) => "host".to_string(),
            (Some(t), Some(k), Some(id)) => format!("{k} {id} ({t})"),
            (Some(t), ..) => t.clone(),
        };
        let what = match &e.event {
            BridgeEvent::Snapshot { sent, .. } => format!("snapshot, sent: {sent}"),
            BridgeEvent::BoardLog { line } if !line.starts_with("host:") => return,
            other => format!("{other:?}").chars().take(140).collect(),
        };
        println!("{:>5.1}s  {who}: {what}", started.elapsed().as_secs_f32());
    });
    std::thread::sleep(Duration::from_secs(seconds));
    manager.stop();
}
