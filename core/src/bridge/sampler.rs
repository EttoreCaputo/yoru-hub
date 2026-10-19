//! The computer's side of the bridge, once for every board: each interval it
//! reads Claude Code's usage, ticks the timers, notes the time and how present
//! you are, and publishes the snapshot for the sessions to send. Timers fire
//! here, once, however many boards there are; what's up goes to the alerts
//! thread to be said.
//!
//! It samples only while a session is open, as the bridge always did: with no
//! board there is nobody to show the data to, and a timer that ends meanwhile
//! rings once one connects.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::claude::ClaudeUsage;
use crate::snapshot::Snapshot;

use super::alerts::{AlertInput, Targets};
use super::{BridgeConfig, sleep_unless_stopped};

/// While no session is open, how often to look whether one has.
const IDLE_LOOK: Duration = Duration::from_millis(200);

/// The latest snapshot, numbered, for the sessions to wait on.
#[derive(Default)]
pub(super) struct Latest {
    slot: Mutex<(u64, Option<Arc<Snapshot>>)>,
    fresh: Condvar,
    /// Sessions open now: the sampler samples while there is one.
    sessions: AtomicUsize,
    /// The latest one as the voice reads it.
    pub voice: Arc<Mutex<Option<Snapshot>>>,
}

impl Latest {
    fn publish(&self, snapshot: Snapshot) {
        *self.voice.lock().unwrap() = Some(snapshot.clone());
        let mut slot = self.slot.lock().unwrap();
        slot.0 += 1;
        slot.1 = Some(Arc::new(snapshot));
        self.fresh.notify_all();
    }

    /// The first snapshot after number `seen`, waiting up to `timeout` for it.
    pub fn newer(&self, seen: u64, timeout: Duration) -> Option<(u64, Arc<Snapshot>)> {
        let slot = self.slot.lock().unwrap();
        let (slot, _) = self.fresh.wait_timeout_while(slot, timeout, |s| s.0 <= seen).unwrap();
        match &slot.1 {
            Some(snapshot) if slot.0 > seen => Some((slot.0, snapshot.clone())),
            _ => None,
        }
    }

    /// The number of the latest snapshot: a session that starts now waits for the next one.
    pub fn number(&self) -> u64 {
        self.slot.lock().unwrap().0
    }

    /// Count a session as open until the guard is dropped.
    pub fn open_session(&self) -> SessionOpen<'_> {
        self.sessions.fetch_add(1, Ordering::Relaxed);
        SessionOpen(self)
    }

    fn any_session(&self) -> bool {
        self.sessions.load(Ordering::Relaxed) > 0
    }
}

/// An open session, for [`Latest::open_session`].
pub(super) struct SessionOpen<'a>(&'a Latest);

impl Drop for SessionOpen<'_> {
    fn drop(&mut self) {
        self.0.sessions.fetch_sub(1, Ordering::Relaxed);
    }
}

/// What the sampler keeps from one snapshot to the next.
pub(super) struct Sampler {
    claude: Mutex<ClaudeUsage>,
}

impl Sampler {
    pub fn new(_config: &BridgeConfig) -> Self {
        Self { claude: Mutex::new(ClaudeUsage::new()) }
    }

    /// Sample every `config.interval` while a session is open, until `stop`.
    pub fn run(&self, config: &BridgeConfig, stop: &AtomicBool, latest: &Latest, alert_tx: &Sender<AlertInput>, _targets: &Targets) {
        let mut next = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            if !latest.any_session() {
                sleep_unless_stopped(IDLE_LOOK, stop);
                next = Instant::now();
                continue;
            }
            latest.publish(self.sample(config, alert_tx));
            next += config.interval;
            let now = Instant::now();
            if next < now {
                next = now;
            }
            sleep_unless_stopped(next - now, stop);
        }
    }

    /// One snapshot of the computer, with what it set off: alerts and timers.
    fn sample(&self, config: &BridgeConfig, alert_tx: &Sender<AlertInput>) -> Snapshot {
        let mut snapshot = Snapshot::now();
        snapshot.claude = self.claude.lock().unwrap().sample();
        if let Some(metrics) = &snapshot.claude {
            let _ = alert_tx.send(AlertInput::Metrics(metrics.clone()));
        }
        for fired in config.timers.tick() {
            let _ = alert_tx.send(AlertInput::Timer(fired));
        }
        snapshot.timer = config.timers.board_view();
        snapshot.pet = Some(crate::presence::sample());
        if let Some(hub) = &config.hub {
            hub.set_snapshot(&snapshot);
        }
        snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn sessions_wait_for_the_next_snapshot() {
        let latest = Arc::new(Latest::default());
        assert!(latest.newer(0, Duration::from_millis(10)).is_none());
        let seen = latest.number();
        let publisher = {
            let latest = latest.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(30));
                latest.publish(Snapshot::now());
            })
        };
        let (n, _) = latest.newer(seen, Duration::from_secs(2)).expect("a snapshot");
        assert_eq!(n, seen + 1);
        assert!(latest.voice.lock().unwrap().is_some());
        publisher.join().unwrap();
        // Already seen: nothing newer.
        assert!(latest.newer(n, Duration::from_millis(10)).is_none());
    }

    #[test]
    fn counts_open_sessions() {
        let latest = Latest::default();
        assert!(!latest.any_session());
        let open = latest.open_session();
        assert!(latest.any_session());
        drop(open);
        assert!(!latest.any_session());
    }
}
