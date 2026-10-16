//! The voice floor: with several boards in a room, one wake word can wake
//! more than one of them, and only one should answer. The first board to
//! start listening claims the floor; another that starts meanwhile is told to
//! stop. The floor is released when the conversation ends, or lapses when its
//! holder has started nothing for [`FLOOR_LAPSE`] (a session that hung).

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long the floor stays claimed after its holder last started listening.
const FLOOR_LAPSE: Duration = Duration::from_secs(30);

#[derive(Default)]
struct Held {
    /// The session holding the floor, and when it last started listening.
    by: Option<(u64, Instant)>,
    /// The session of the latest conversation, which alerts prefer.
    last: Option<u64>,
}

#[derive(Default)]
pub(super) struct Floor {
    held: Mutex<Held>,
}

impl Floor {
    /// Session `session`'s board started listening: true when it may go on.
    /// Its own next utterance (a follow-up, a barge-in) claims it again.
    pub fn claim(&self, session: u64) -> bool {
        self.claim_at(session, Instant::now())
    }

    fn claim_at(&self, session: u64, now: Instant) -> bool {
        let mut held = self.held.lock().unwrap();
        if let Some((by, at)) = held.by
            && by != session
            && now.duration_since(at) < FLOOR_LAPSE
        {
            return false;
        }
        held.by = Some((session, now));
        held.last = Some(session);
        true
    }

    /// Session `session`'s conversation is over, or the session ended.
    pub fn release(&self, session: u64) {
        let mut held = self.held.lock().unwrap();
        if held.by.is_some_and(|(by, _)| by == session) {
            held.by = None;
        }
    }

    /// The session of the latest conversation.
    pub fn last(&self) -> Option<u64> {
        self.held.lock().unwrap().last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_board_keeps_the_floor_until_released() {
        let floor = Floor::default();
        let t0 = Instant::now();
        assert!(floor.claim_at(1, t0));
        assert!(!floor.claim_at(2, t0 + Duration::from_secs(1)));
        // Its own follow-up is the same conversation.
        assert!(floor.claim_at(1, t0 + Duration::from_secs(5)));
        // Another's release changes nothing.
        floor.release(2);
        assert!(!floor.claim_at(2, t0 + Duration::from_secs(6)));
        floor.release(1);
        assert!(floor.claim_at(2, t0 + Duration::from_secs(7)));
        assert_eq!(floor.last(), Some(2));
    }

    #[test]
    fn the_floor_lapses_after_its_holder_goes_quiet() {
        let floor = Floor::default();
        let t0 = Instant::now();
        assert!(floor.claim_at(1, t0));
        assert!(floor.claim_at(1, t0 + Duration::from_secs(20)));
        // 30 s from the holder's last start, not its first.
        assert!(!floor.claim_at(2, t0 + Duration::from_secs(45)));
        assert!(floor.claim_at(2, t0 + Duration::from_secs(51)));
        assert_eq!(floor.last(), Some(2));
    }
}
