//! `yoru.log` in the data folder: what each board and the notch did, one
//! line an event (connections, the board's own log, what was heard and
//! answered, voice errors), so a problem can be read after the fact. Snapshots
//! and other chatter stay out. It starts over at [`MAX_BYTES`], keeping the
//! previous file as `yoru.log.1`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::sync::Mutex;

use chrono::Local;

use crate::bridge::BridgeEvent;

/// Size at which the log starts over.
const MAX_BYTES: u64 = 2 * 1024 * 1024;

static WRITING: Mutex<()> = Mutex::new(());

/// Log what `event` says about `target` (`None`: the host itself).
pub fn event(target: Option<&str>, event: &BridgeEvent) {
    if let Some(line) = describe(event) {
        write(target.unwrap_or("host"), &line);
    }
}

/// Log a line of the host's own.
pub fn line(target: &str, line: &str) {
    write(target, line);
}

/// Log panics too, with where they happened: a thread that dies (the
/// voice's, a session's) otherwise leaves no trace, the app having no
/// terminal. The previous hook still runs.
pub fn log_panics() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed");
        let backtrace = std::backtrace::Backtrace::force_capture();
        write("panic", &format!("thread {name}: {info}\n{backtrace}"));
        previous(info);
    }));
}

fn describe(event: &BridgeEvent) -> Option<String> {
    Some(match event {
        BridgeEvent::Waiting { reason } => format!("waiting: {reason}"),
        BridgeEvent::Connected { port } => format!("connected on {port}"),
        BridgeEvent::Disconnected { port, reason, .. } => format!("disconnected from {port}: {reason}"),
        BridgeEvent::BoardLog { line } => line.clone(),
        BridgeEvent::Firmware { firmware } => format!("firmware {}", serde_json::to_string(firmware).unwrap_or_default()),
        BridgeEvent::Wake { word, .. } => format!("wake word: {word}"),
        BridgeEvent::Listening { id, trigger } => format!("listening #{id} ({trigger})"),
        BridgeEvent::Utterance { utterance, duration_ms, peak_db, .. } => format!(
            "heard #{} ({}): {duration_ms} ms, ended {}, speech {}, peak {}",
            utterance.id,
            utterance.trigger,
            utterance.reason,
            utterance.speech,
            peak_db.map_or("-".into(), |p| format!("{p:.0} dB"))
        ),
        BridgeEvent::Transcript { id, transcript } => match transcript {
            Some(t) => format!("transcript #{id} ({}, {} ms): {}", t.language, t.elapsed_ms, t.text),
            None => format!("transcript #{id}: nothing"),
        },
        BridgeEvent::Reply { id, text, by, actions, elapsed_ms, .. } => {
            let actions = if actions.is_empty() { String::new() } else { format!(" [{}]", actions.join("; ")) };
            format!("reply #{id} by {by} ({elapsed_ms} ms): {text}{actions}")
        }
        BridgeEvent::Spoken { id, spoken } => format!("spoken #{id}: {} ms played, ended {}", spoken.played_ms, spoken.reason),
        BridgeEvent::VoiceError { message } => format!("voice error: {message}"),
        BridgeEvent::TimerFired { text, error, .. } => format!("timer: {text}{}", error.as_ref().map_or(String::new(), |e| format!(" (error: {e})"))),
        BridgeEvent::Notified { id, error: Some(e) } => format!("notification {id} failed: {e}"),
        _ => return None,
    })
}

fn write(target: &str, line: &str) {
    // Tests run fake boards: their lines aren't the user's.
    if cfg!(test) {
        return;
    }
    let Some(dir) = crate::paths::data_dir() else { return };
    let path = dir.join("yoru.log");
    let _guard = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    if fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = fs::rename(&path, dir.join("yoru.log.1"));
    }
    let stamped = format!("{} [{target}] {line}\n", Local::now().format("%Y-%m-%d %H:%M:%S%.3f"));
    let _ = OpenOptions::new().create(true).append(true).open(&path).and_then(|mut f| f.write_all(stamped.as_bytes()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chatter_stays_out() {
        assert!(describe(&BridgeEvent::VoiceState { state: "idle".into() }).is_none());
        assert_eq!(describe(&BridgeEvent::VoiceError { message: "no key".into() }).as_deref(), Some("voice error: no key"));
    }
}
