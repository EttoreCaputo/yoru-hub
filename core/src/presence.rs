//! The person at the computer, for the board: the local
//! time, for its day and night (the screen dims at night), and how long
//! since the last mouse or keyboard input, for "away" and "back". Sent as
//! the snapshot's `host` key.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presence {
    /// Minutes since local midnight.
    pub min: u16,
    /// Seconds without input; absent where the OS won't tell (Linux), and
    /// the board then takes you as there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle: Option<u32>,
    /// The time zone as POSIX writes it (`CET-1CEST,M3.5.0,M10.5.0/3`), so
    /// the board keeps local time, changes of time included, on its own
    /// clock; absent where the system doesn't say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
}

/// The board keeps at most this much of the time zone.
const TZ_MAX: usize = 47;

/// Now.
pub fn sample() -> Presence {
    use chrono::Timelike;
    let now = chrono::Local::now();
    Presence { min: (now.hour() * 60 + now.minute()) as u16, idle: platform::idle_seconds(), tz: posix_tz() }
}

/// The system's time zone as a POSIX string: the footer a TZif file of
/// version 2 or later ends with (`\n<TZ>\n`), from `/etc/localtime`.
fn posix_tz() -> Option<String> {
    posix_tz_of(&std::fs::read("/etc/localtime").ok()?)
}

fn posix_tz_of(tzif: &[u8]) -> Option<String> {
    if !tzif.starts_with(b"TZif") || tzif.last() != Some(&b'\n') {
        return None;
    }
    let body = &tzif[..tzif.len() - 1];
    let start = body.iter().rposition(|&b| b == b'\n')? + 1;
    let tz = std::str::from_utf8(&body[start..]).ok()?;
    (!tz.is_empty() && tz.len() <= TZ_MAX && tz.bytes().all(|b| b.is_ascii_graphic())).then(|| tz.to_string())
}

#[cfg(target_os = "macos")]
mod platform {
    /// kCGEventSourceStateCombinedSessionState
    const COMBINED_SESSION: i32 = 0;
    /// kCGAnyInputEventType
    const ANY_INPUT: u32 = !0;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
    }

    pub(super) fn idle_seconds() -> Option<u32> {
        // SAFETY: a plain query with no pointers.
        let s = unsafe { CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION, ANY_INPUT) };
        (s.is_finite() && s >= 0.0).then_some(s as u32)
    }
}

#[cfg(target_os = "windows")]
mod platform {
    #[repr(C)]
    struct LastInputInfo {
        size: u32,
        time: u32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetLastInputInfo(info: *mut LastInputInfo) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetTickCount() -> u32;
    }

    pub(super) fn idle_seconds() -> Option<u32> {
        let mut info = LastInputInfo { size: size_of::<LastInputInfo>() as u32, time: 0 };
        // SAFETY: plain Win32 calls with a valid, sized out struct.
        unsafe {
            if GetLastInputInfo(&mut info) == 0 {
                return None;
            }
            Some(GetTickCount().wrapping_sub(info.time) / 1000)
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    pub(super) fn idle_seconds() -> Option<u32> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minutes_are_within_a_day() {
        assert!(sample().min < 24 * 60);
    }

    #[test]
    fn idle_is_left_out_when_unknown() {
        let json = serde_json::to_string(&Presence { min: 600, idle: None, tz: None }).unwrap();
        assert_eq!(json, r#"{"min":600}"#);
    }

    #[test]
    fn the_time_zone_is_the_tzif_footer() {
        let mut file = b"TZif2\0\0\0 binary data \n".to_vec();
        file.extend_from_slice(b"CET-1CEST,M3.5.0,M10.5.0/3\n");
        assert_eq!(posix_tz_of(&file).as_deref(), Some("CET-1CEST,M3.5.0,M10.5.0/3"));
        assert_eq!(posix_tz_of(b"TZif1 without a footer"), None);
        assert_eq!(posix_tz_of(b"not a zone\nUTC0\n"), None);
    }
}
