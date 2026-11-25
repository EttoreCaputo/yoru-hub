//! Who does things (docs/icloud.md): with several Macs sharing one Yoru,
//! timers, tasks, watchers, the recap and the memory's consolidation must
//! fire once, not on every Mac. One Mac is the *doer*: the one with a board
//! connected, else the one used last. Its claim is a lease, a record in zone
//! `Settings` renewed every minute and taken over when it runs out, so a Mac
//! that is switched off hands over by itself.
//!
//! Without sync (or with it off) every Mac is its own doer: [`is_doer`] is
//! true until the app turns the lease on.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::sync::{Field, Record, Store, newer, now_ms};

/// How long a claim lasts unless renewed.
const LEASE_MS: i64 = 180_000;
/// Renewed once this much of it has passed.
const RENEW_MS: i64 = 60_000;
/// How long a Mac waits to hear from iCloud before claiming on its own: it
/// may be offline, and then it is the doer of itself.
const WAIT_MS: i64 = 120_000;
/// A Mac used this much later than the doer takes over (with no board on either).
const USED_LEAD_MS: i64 = 60_000;

#[derive(Debug, Clone, Default, PartialEq)]
struct Lease {
    holder: String,
    until: i64,
    board: bool,
    used: i64,
    changed: i64,
    by: String,
    queued: bool,
}

pub struct Doer {
    device: String,
    lease: Mutex<Lease>,
    /// Sync is running: the lease counts.
    active: AtomicBool,
    /// When sync started, and whether iCloud answered since.
    since: AtomicI64,
    heard: AtomicBool,
    board: AtomicBool,
    used: AtomicI64,
}

static DOER: OnceLock<Arc<Doer>> = OnceLock::new();

/// The lease of this Mac, made the first time it is asked for.
pub fn doer(device: impl FnOnce() -> String) -> Arc<Doer> {
    DOER.get_or_init(|| Arc::new(Doer::new(device()))).clone()
}

/// Whether this Mac fires timers, tasks, watchers and consolidation.
pub fn is_doer() -> bool {
    DOER.get().is_none_or(|d| d.is_doer(now_ms()))
}

impl Doer {
    pub fn new(device: String) -> Self {
        Self {
            device,
            lease: Mutex::default(),
            active: AtomicBool::new(false),
            since: AtomicI64::new(0),
            heard: AtomicBool::new(false),
            board: AtomicBool::new(false),
            used: AtomicI64::new(now_ms()),
        }
    }

    /// Sync started (or stopped: every Mac for itself again).
    pub fn set_active(&self, on: bool) {
        if on && !self.active.load(Ordering::SeqCst) {
            self.since.store(now_ms(), Ordering::SeqCst);
        }
        self.active.store(on, Ordering::SeqCst);
    }

    /// iCloud answered: the lease known here is the one there is.
    pub fn heard(&self) {
        self.heard.store(true, Ordering::SeqCst);
    }

    pub fn set_board(&self, on: bool) {
        self.board.store(on, Ordering::SeqCst);
    }

    /// The user just used this Mac.
    pub fn used_now(&self) {
        self.used.store(now_ms(), Ordering::SeqCst);
    }

    pub fn is_doer(&self, now: i64) -> bool {
        if !self.active.load(Ordering::SeqCst) {
            return true;
        }
        let l = self.lease.lock().unwrap();
        l.holder == self.device && l.until > now
    }

    /// The Mac holding the lease, and until when.
    pub fn holder(&self) -> (String, i64) {
        let l = self.lease.lock().unwrap();
        (l.holder.clone(), l.until)
    }

    /// Claim, renew or leave the lease; every few seconds while sync runs.
    pub fn tick(&self, now: i64) {
        if !self.active.load(Ordering::SeqCst) {
            return;
        }
        let ready = self.heard.load(Ordering::SeqCst) || now - self.since.load(Ordering::SeqCst) > WAIT_MS;
        let board = self.board.load(Ordering::SeqCst);
        let used = self.used.load(Ordering::SeqCst);
        let mut l = self.lease.lock().unwrap();
        let mine = l.holder == self.device;
        let take = if mine {
            l.until - now < LEASE_MS - RENEW_MS || l.board != board
        } else {
            ready && (l.until <= now || (board && !l.board) || (!board && !l.board && used > l.used + USED_LEAD_MS))
        };
        if take {
            *l = Lease { holder: self.device.clone(), until: now + LEASE_MS, board, used, changed: now.max(l.changed + 1), by: self.device.clone(), queued: true };
        }
    }

    fn record(&self, l: &Lease) -> Record {
        let fields = BTreeMap::from([
            ("holder".to_string(), Field::Text(l.holder.clone())),
            ("until".to_string(), Field::Int(l.until)),
            ("board".to_string(), Field::Int(l.board as i64)),
            ("used".to_string(), Field::Int(l.used)),
        ]);
        Record { zone: "Settings".into(), kind: "lease".into(), uid: "doer".into(), changed: l.changed, by: l.by.clone(), deleted: false, fields }
    }
}

impl Store for Doer {
    fn device(&self) -> String {
        self.device.clone()
    }

    fn changes(&self, _max: usize) -> Vec<Record> {
        let l = self.lease.lock().unwrap();
        if l.queued { vec![self.record(&l)] } else { Vec::new() }
    }

    fn sent(&self, records: &[Record]) {
        let mut l = self.lease.lock().unwrap();
        if records.iter().any(|r| r.kind == "lease" && r.changed == l.changed) {
            l.queued = false;
        }
    }

    fn apply(&self, records: &[Record]) -> usize {
        let mut l = self.lease.lock().unwrap();
        let mut changed = 0;
        for r in records.iter().filter(|r| r.zone == "Settings" && r.kind == "lease" && r.uid == "doer") {
            if !newer((r.changed, &r.by), (l.changed, &l.by)) {
                continue;
            }
            let int = |k: &str| match r.fields.get(k) {
                Some(Field::Int(i)) => *i,
                _ => 0,
            };
            *l = Lease {
                holder: r.fields.get("holder").and_then(Field::as_text).unwrap_or_default().to_string(),
                until: int("until"),
                board: int("board") != 0,
                used: int("used"),
                changed: r.changed,
                by: r.by.clone(),
                queued: false,
            };
            changed += 1;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pass(from: &Doer, to: &Doer) {
        let r = from.changes(10);
        to.apply(&r);
        from.sent(&r);
    }

    #[test]
    fn one_mac_does_things_and_the_board_draws_it() {
        let (a, b) = (Doer::new("a".into()), Doer::new("b".into()));
        assert!(a.is_doer(0), "without sync every Mac is its own doer");
        a.set_active(true);
        b.set_active(true);
        let t = now_ms();
        assert!(!a.is_doer(t), "nobody until it claims");
        a.heard();
        b.heard();
        a.tick(t);
        pass(&a, &b);
        b.tick(t + 1000);
        assert!(a.is_doer(t + 1000) && !b.is_doer(t + 1000));
        assert!(b.changes(10).is_empty(), "b leaves it to a");

        // A board on b: b takes over, and a lets go.
        b.set_board(true);
        b.tick(t + 2000);
        pass(&b, &a);
        a.tick(t + 3000);
        assert!(b.is_doer(t + 3000) && !a.is_doer(t + 3000));

        // b goes quiet: when its lease runs out, a takes it.
        a.tick(t + 2000 + LEASE_MS + 1);
        assert!(a.is_doer(t + 2000 + LEASE_MS + 1));
    }

    #[test]
    fn a_mac_that_heard_nothing_waits_before_claiming() {
        let a = Doer::new("a".into());
        a.set_active(true);
        let t = now_ms();
        a.tick(t);
        assert!(!a.is_doer(t));
        a.tick(t + WAIT_MS + 1);
        assert!(a.is_doer(t + WAIT_MS + 1), "offline, it does things itself");
    }
}
