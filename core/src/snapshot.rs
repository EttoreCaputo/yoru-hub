//! The snapshot, the payload of a protocol v2 `metrics` frame, parsed by
//! Yoru's `main/metrics_parser.c`.
//!
//! Yoru shows no gauges: it reads only the computer's time and how long since
//! the last input (`pet`: night brightness, and when it gets sleepy) and the
//! timer that ends first (`timer`). Firmware 0.10.0 takes a snapshot without CPU or GPU
//! readings, as long as it has `pet`.

use serde::{Deserialize, Serialize};

use crate::presence::Presence;
use crate::timers::BoardTimer;

/// The snapshot format's version, the `v` field.
pub const SNAPSHOT_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub v: u32,
    pub ts: u64,
    /// The timer that ends first, set from the [`crate::Timers`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timer: Option<BoardTimer>,
    /// The time and how long since your last input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pet: Option<Presence>,
}

impl Snapshot {
    /// An empty snapshot taken now.
    pub fn now() -> Self {
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        Self { v: SNAPSHOT_VERSION, ts, timer: None, pet: None }
    }

    /// The firmware drops snapshots without the computer's time.
    pub fn is_sendable(&self) -> bool {
        self.pet.is_some()
    }

    /// The `metrics` frame payload.
    pub fn to_payload(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("snapshot is always serializable")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_is_there_is_sent() {
        let mut snap = Snapshot::now();
        snap.ts = 5;
        assert!(!snap.is_sendable());
        assert_eq!(String::from_utf8(snap.to_payload()).unwrap(), "{\"v\":2,\"ts\":5}");
        snap.pet = Some(Presence { min: 600, idle: Some(3) });
        assert!(snap.is_sendable());
        assert_eq!(String::from_utf8(snap.to_payload()).unwrap(), "{\"v\":2,\"ts\":5,\"pet\":{\"min\":600,\"idle\":3}}");
    }
}
