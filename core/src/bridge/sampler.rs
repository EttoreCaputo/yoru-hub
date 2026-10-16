//! The computer's side of the bridge, once for every board: each interval it
//! reads the sensors and Claude Code's usage, ticks the timers, notes what's
//! playing and how present you are, keeps the pet's diary, and publishes the
//! snapshot for the sessions to send. Timers fire here, once, however many
//! boards there are; what's up goes to the alerts thread to be said.
//!
//! It samples only while a session is open, as the bridge always did: with no
//! board there is nobody to show the data to, and a timer that ends meanwhile
//! rings once one connects.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use chrono::Local;

use crate::claude::{ClaudeState, ClaudeUsage};
use crate::diary::{Diary, Seen};
use crate::quips::{Quip, QuipLevel, Quips};
use crate::sensors::Collector;
use crate::snapshot::Snapshot;
use crate::timers::Fired;

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
    /// The latest one as the voice reads it ("how hot is the CPU").
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
    collector: Mutex<Collector>,
    claude: Mutex<ClaudeUsage>,
    pet: Mutex<Pet>,
}

impl Sampler {
    pub fn new(config: &BridgeConfig) -> Self {
        Self {
            collector: Mutex::new(Collector::new()),
            claude: Mutex::new(ClaudeUsage::new()),
            pet: Mutex::new(Pet { diary: Diary::open(config.diary.clone()), quips: Quips::default(), observed: None }),
        }
    }

    /// Sample every `config.interval` while a session is open, until `stop`.
    pub fn run(&self, config: &BridgeConfig, stop: &AtomicBool, latest: &Latest, alert_tx: &Sender<AlertInput>, targets: &Targets) {
        let mut next = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            if !latest.any_session() {
                sleep_unless_stopped(IDLE_LOOK, stop);
                next = Instant::now();
                continue;
            }
            latest.publish(self.sample(config, alert_tx, targets));
            next += config.interval;
            let now = Instant::now();
            if next < now {
                next = now;
            }
            sleep_unless_stopped(next - now, stop);
        }
    }

    /// One snapshot of the computer, with what it set off: alerts, timers, a quip.
    fn sample(&self, config: &BridgeConfig, alert_tx: &Sender<AlertInput>, targets: &Targets) -> Snapshot {
        let mut snapshot = self.collector.lock().unwrap().sample();
        snapshot.claude = self.claude.lock().unwrap().sample();
        if let Some(metrics) = &snapshot.claude {
            let _ = alert_tx.send(AlertInput::Metrics(metrics.clone()));
        }
        let fired = config.timers.tick();
        for fired in &fired {
            let _ = alert_tx.send(AlertInput::Timer(fired.clone()));
        }
        snapshot.timer = config.timers.board_view();
        // The pet dances to it; a DualEye's music face asks for it too.
        let primary = config.primary.lock().unwrap().clone();
        let board = targets.speaker(None, primary.as_deref());
        if board.is_some() {
            config.music.want();
        }
        snapshot.music = config.music.board_view();
        snapshot.pet = Some(crate::presence::sample());
        if let Some(hub) = &config.hub {
            hub.set_snapshot(&snapshot);
        }
        // Only with a voice to say it, and every board out of any conversation.
        let can_speak = board.is_some() && config.voice.lock().unwrap().tts.is_some();
        let busy = !can_speak || targets.in_conversation();
        let level = *config.quips.lock().unwrap();
        if let Some(quip) = self.pet.lock().unwrap().observe(&snapshot, &fired, level, busy) {
            let _ = alert_tx.send(AlertInput::Quip(quip));
        }
        snapshot
    }
}

/// What the pet remembers of your days, and when it last spoke up about them.
struct Pet {
    diary: Diary,
    quips: Quips,
    observed: Option<Instant>,
}

impl Pet {
    /// Note `snapshot` in the diary, and whether there's something to say.
    fn observe(&mut self, snapshot: &Snapshot, fired: &[Fired], level: QuipLevel, busy: bool) -> Option<Quip> {
        let dt = self.observed.map_or(1, |t| t.elapsed().as_secs_f32().round().clamp(1.0, 5.0) as u32);
        self.observed = Some(Instant::now());
        let hottest = [snapshot.cpu.temp_c, snapshot.gpu.temp_c].into_iter().flatten().reduce(f32::max);
        let seen = Seen {
            idle_s: snapshot.pet.and_then(|p| p.idle),
            claude: snapshot.claude.as_ref().is_some_and(|c| c.state == ClaudeState::Work),
            music: snapshot.music.as_ref().is_some_and(|m| m.state == "play"),
            hottest_c: hottest,
        };
        let now = Local::now().naive_local();
        self.diary.observe(now, &seen, dt);
        for fired in fired {
            self.diary.fired(fired);
        }
        self.quips.consider(&self.diary, now, level, busy || seen.music)
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
                latest.publish(serde_json::from_value(serde_json::json!({"v": 2, "ts": 5})).unwrap());
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
