//! The snapshot, the payload of a protocol v2 `metrics` frame, parsed by
//! Yoru's `main/metrics_parser.c`.
//!
//! The board reads only the computer's time and how long since the last
//! input (`host`, for the night brightness and the idle screen).

use serde::{Deserialize, Serialize};

use crate::presence::Presence;

/// The snapshot format's version, the `v` field.
pub const SNAPSHOT_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub v: u32,
    pub ts: u64,
    /// The time and how long since your last input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<Presence>,
}

impl Snapshot {
    /// An empty snapshot taken now.
    pub fn now() -> Self {
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        Self { v: SNAPSHOT_VERSION, ts, host: None }
    }

    /// The firmware drops snapshots without the computer's time.
    pub fn is_sendable(&self) -> bool {
        self.host.is_some()
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
        snap.host = Some(Presence { min: 600, idle: Some(3), tz: None });
        assert!(snap.is_sendable());
        assert_eq!(String::from_utf8(snap.to_payload()).unwrap(), "{\"v\":2,\"ts\":5,\"host\":{\"min\":600,\"idle\":3}}");
    }
}
