//! Meeting detection (docs/meetings.md, Detection): Yoru notices a meeting
//! starting and asks whether to record it. Two signals, each covering the
//! other's blind spot:
//!
//! - the calendar says one is about to start (a minute or two before), so
//!   Yoru asks ahead and records as soon as the call starts;
//! - a call app takes the microphone (`yoru-audio mic-users`), so a call
//!   that isn't in the calendar is noticed too. A browser counts only while
//!   the calendar has a meeting on: it takes the microphone for much else.
//!
//! A recording the detection started stops once the call app has let the
//! microphone go for a while. [`Detector`] only decides; the app asks the
//! questions and records.

use std::collections::HashSet;
use std::io::{self, BufRead, BufReader};
use std::process::{ChildStdin, Command, Stdio};
use std::thread;

use serde::Deserialize;
use serde_json::Value;

/// Asked this long before a calendar meeting starts.
pub const HEADS_UP_MS: i64 = 2 * 60 * 1000;
/// A meeting counts as on from this long before its start (people join early).
const EARLY_MS: i64 = 10 * 60 * 1000;
/// A "yes" given ahead waits this long after the start for the call.
const ARMED_MS: i64 = 15 * 60 * 1000;
/// The call app off the microphone this long: the call is over.
pub const QUIET_STOP_MS: i64 = 45 * 1000;

/// Apps that take the microphone only for calls.
const CALL_APPS: &[&str] = &[
    "us.zoom.xos",
    "com.microsoft.teams2",
    "com.microsoft.teams",
    "com.apple.FaceTime",
    "com.tinyspeck.slackmacgap",
    "com.hnc.Discord",
    "Cisco-Systems.Spark",
    "com.cisco.webexmeetingsapp",
    "com.webex.meetingmanager",
    "net.whatsapp.WhatsApp",
    "desktop.WhatsApp",
    "ru.keepcoder.Telegram",
    "com.skype.skype",
    "com.google.meet",
];

/// Browsers: a call in a tab (Meet, Teams on the web) is theirs.
const BROWSERS: &[&str] = &[
    "com.google.Chrome",
    "com.apple.Safari",
    "org.mozilla.firefox",
    "company.thebrowser.Browser",
    "com.brave.Browser",
    "com.microsoft.edgemac",
    "com.operasoftware.Opera",
    "com.vivaldi.Vivaldi",
];

/// Yoru's own app: its notch and its recordings use the microphone too.
const OURS: &str = "it.bytebusters.yoru";

/// Links that make a calendar event a call.
const CALL_LINKS: &[&str] = &["meet.google.com", "zoom.us/", "teams.microsoft.com", "teams.live.com", "webex.com", "whereby.com", "facetime.apple.com"];

/// An app taking sound from a microphone.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MicUser {
    /// Its bundle id.
    pub id: String,
    pub name: String,
}

/// A meeting in the calendar.
#[derive(Debug, Clone, PartialEq)]
pub struct CalendarMeeting {
    pub id: String,
    pub title: String,
    /// Unix ms.
    pub start: i64,
    pub end: i64,
}

fn is_call_app(id: &str) -> bool {
    CALL_APPS.contains(&id)
}

fn is_browser(id: &str) -> bool {
    BROWSERS.contains(&id)
}

/// The meetings in Google Calendar's `list_events` answer: timed events
/// with a call link or other people, not declined.
pub fn meetings_in(answer: &str) -> Vec<CalendarMeeting> {
    let Ok(value) = serde_json::from_str::<Value>(answer) else { return Vec::new() };
    let time = |v: &Value| v.as_str().and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()).map(|t| t.timestamp_millis());
    value["events"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            if e["all_day"].as_bool().unwrap_or(false) || e["status"] == "cancelled" {
                return None;
            }
            let attendees = e["attendees"].as_array().cloned().unwrap_or_default();
            if attendees.iter().any(|a| a["self"].as_bool().unwrap_or(false) && a["response"] == "declined") {
                return None;
            }
            let text = format!("{} {} {}", e["video_call"].as_str().unwrap_or(""), e["location"].as_str().unwrap_or(""), e["description"].as_str().unwrap_or(""));
            let call = e["video_call"].is_string() || CALL_LINKS.iter().any(|l| text.contains(l)) || attendees.len() >= 2;
            if !call {
                return None;
            }
            Some(CalendarMeeting {
                id: e["event_id"].as_str()?.to_string(),
                title: e["summary"].as_str().unwrap_or("").trim().to_string(),
                start: time(&e["start"])?,
                end: time(&e["end"])?,
            })
        })
        .collect()
}

/// How far ahead the calendar is read.
const LOOK_AHEAD_MIN: i64 = 120;

/// The arguments for `list_events`: what is on from a little while ago to
/// two hours ahead.
pub fn calendar_query() -> Value {
    let now = chrono::Local::now();
    serde_json::json!({
        "time_min": (now - chrono::Duration::milliseconds(EARLY_MS)).to_rfc3339(),
        "time_max": (now + chrono::Duration::minutes(LOOK_AHEAD_MIN)).to_rfc3339(),
        "max_results": 20,
    })
}

/// `ms` as the local time of day (`10:30`).
pub fn clock(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string()).unwrap_or_default()
}

/// What the app should do now.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// A meeting in the calendar is about to start: ask whether to record
    /// it once the call starts ([`Detector::armed`] on yes).
    HeadsUp(CalendarMeeting),
    /// A call started: ask whether to record `app` (and its meeting, if the
    /// calendar has one on).
    Ask { app: MicUser, meeting: Option<CalendarMeeting> },
    /// Record `app`'s call: the user said yes ahead.
    Record { app: MicUser, title: String },
    /// The call the detection recorded is over.
    Stop,
}

#[derive(Debug, Clone)]
struct Armed {
    title: String,
    until: i64,
}

/// The detection's state: what it already asked, a "yes" given ahead, and
/// the recording it started.
#[derive(Debug, Default)]
pub struct Detector {
    meetings: Vec<CalendarMeeting>,
    /// Meetings (their ids) and calls (`app:<bundle id>`) already asked about.
    asked: HashSet<String>,
    armed: Option<Armed>,
    /// The app whose call the detection is recording.
    following: Option<String>,
    quiet_since: Option<i64>,
    /// A question is waiting for the user: one at a time.
    asking: bool,
}

impl Detector {
    pub fn set_meetings(&mut self, meetings: Vec<CalendarMeeting>) {
        self.meetings = meetings;
    }

    /// The question asked is answered (or gone unanswered).
    pub fn answered(&mut self) {
        self.asking = false;
    }

    /// The user said yes ahead to `meeting`.
    pub fn armed(&mut self, meeting: &CalendarMeeting) {
        self.armed = Some(Armed { title: meeting.title.clone(), until: meeting.start + ARMED_MS });
    }

    /// The detection started recording `app`'s call.
    pub fn following(&mut self, app: &str) {
        self.following = Some(app.to_string());
        self.quiet_since = None;
    }

    /// Look at the clock, the apps on the microphone and whether a
    /// recording runs: the next thing to do, if any.
    pub fn step(&mut self, now: i64, users: &[MicUser], recording: bool) -> Option<Action> {
        let users: Vec<&MicUser> = users.iter().filter(|u| !u.id.starts_with(OURS)).collect();
        // A call that ended may be asked about again when another starts.
        self.asked.retain(|k| k.strip_prefix("app:").is_none_or(|id| users.iter().any(|u| u.id == id)));
        if self.armed.as_ref().is_some_and(|a| now > a.until) {
            self.armed = None;
        }
        if recording {
            let app = self.following.clone()?;
            if users.iter().any(|u| u.id == app) {
                self.quiet_since = None;
                return None;
            }
            let since = *self.quiet_since.get_or_insert(now);
            if now - since < QUIET_STOP_MS {
                return None;
            }
            self.following = None;
            self.quiet_since = None;
            return Some(Action::Stop);
        }
        // Stopped by hand, or never ours.
        self.following = None;
        self.quiet_since = None;
        if self.asking {
            return None;
        }
        let on = self.meetings.iter().find(|m| m.start - EARLY_MS <= now && now <= m.end).cloned();
        if let Some(app) = users.iter().find(|u| is_call_app(&u.id) || (is_browser(&u.id) && on.is_some())) {
            if let Some(armed) = self.armed.take() {
                return Some(Action::Record { app: (*app).clone(), title: armed.title });
            }
            let call = format!("app:{}", app.id);
            let fresh = !self.asked.contains(&call) && on.as_ref().is_none_or(|m| !self.asked.contains(&m.id));
            if !fresh {
                return None;
            }
            self.asked.insert(call);
            if let Some(m) = &on {
                self.asked.insert(m.id.clone());
            }
            self.asking = true;
            return Some(Action::Ask { app: (*app).clone(), meeting: on });
        }
        let soon = self.meetings.iter().find(|m| m.start - HEADS_UP_MS <= now && now <= m.start + 60_000 && !self.asked.contains(&m.id)).cloned()?;
        self.asked.insert(soon.id.clone());
        self.asking = true;
        Some(Action::HeadsUp(soon))
    }
}

/// `yoru-audio mic-users` running: `on_change` hears the apps on the
/// microphone, at once and at every change, on a thread of its own. Dropped,
/// it ends the helper (its stdin closes).
pub struct MicWatch {
    _stdin: ChildStdin,
}

impl MicWatch {
    pub fn start(on_change: impl Fn(Vec<MicUser>) + Send + 'static, on_end: impl FnOnce() + Send + 'static) -> io::Result<MicWatch> {
        let mut child = Command::new(super::helper()?).arg("mic-users").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        thread::Builder::new().name("yoru-mic-users".into()).spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
                if value["event"] == "mic_users" {
                    on_change(serde_json::from_value(value["apps"].clone()).unwrap_or_default());
                }
            }
            let _ = child.wait();
            on_end();
        })?;
        Ok(MicWatch { _stdin: stdin })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    fn app(id: &str) -> MicUser {
        MicUser { id: id.into(), name: id.rsplit('.').next().unwrap().into() }
    }

    fn standup(start: i64) -> CalendarMeeting {
        CalendarMeeting { id: "ev1".into(), title: "Standup".into(), start, end: start + 30 * MIN }
    }

    #[test]
    fn calendar_events_that_are_calls() {
        let answer = serde_json::json!({"events": [
            {"event_id": "a", "summary": "Standup", "start": "2026-11-18T10:00:00+01:00", "end": "2026-11-18T10:15:00+01:00", "all_day": false, "video_call": "https://meet.google.com/abc"},
            {"event_id": "b", "summary": "Dentist", "start": "2026-11-18T11:00:00+01:00", "end": "2026-11-18T12:00:00+01:00", "all_day": false},
            {"event_id": "c", "summary": "Zoom", "start": "2026-11-18T12:00:00+01:00", "end": "2026-11-18T13:00:00+01:00", "all_day": false, "location": "https://us02web.zoom.us/j/123"},
            {"event_id": "d", "summary": "Holiday", "start": "2026-11-18", "end": "2026-11-19", "all_day": true, "video_call": "https://meet.google.com/x"},
            {"event_id": "e", "summary": "Declined", "start": "2026-11-18T14:00:00+01:00", "end": "2026-11-18T15:00:00+01:00", "all_day": false,
             "attendees": [{"email": "me@x", "self": true, "response": "declined"}, {"email": "o@x", "self": false}]},
        ]});
        let found: Vec<String> = meetings_in(&answer.to_string()).into_iter().map(|m| m.id).collect();
        assert_eq!(found, ["a", "c"]);
    }

    #[test]
    fn a_meeting_about_to_start_is_asked_once_then_recorded_when_the_call_starts() {
        let mut d = Detector::default();
        let t = 1_000 * MIN;
        d.set_meetings(vec![standup(t)]);
        assert_eq!(d.step(t - 5 * MIN, &[], false), None);
        assert_eq!(d.step(t - 2 * MIN, &[], false), Some(Action::HeadsUp(standup(t))));
        // One question at a time, and the same one never twice.
        assert_eq!(d.step(t - MIN, &[], false), None);
        d.answered();
        d.armed(&standup(t));
        assert_eq!(d.step(t - MIN, &[], false), None);
        let chrome = app("com.google.Chrome");
        assert_eq!(d.step(t + MIN, &[chrome.clone()], false), Some(Action::Record { app: chrome, title: "Standup".into() }));
    }

    #[test]
    fn a_no_ahead_means_no_question_when_the_call_starts() {
        let mut d = Detector::default();
        let t = 1_000 * MIN;
        d.set_meetings(vec![standup(t)]);
        assert!(matches!(d.step(t - MIN, &[], false), Some(Action::HeadsUp(_))));
        d.answered();
        assert_eq!(d.step(t, &[app("us.zoom.xos")], false), None);
    }

    #[test]
    fn a_call_app_on_the_microphone_is_asked_about_a_browser_only_in_a_meeting() {
        let mut d = Detector::default();
        let t = 1_000 * MIN;
        // A browser alone, or Yoru's own microphone: nothing.
        assert_eq!(d.step(t, &[app("com.google.Chrome"), app("it.bytebusters.yoru.hub")], false), None);
        let zoom = app("us.zoom.xos");
        assert_eq!(d.step(t, &[zoom.clone()], false), Some(Action::Ask { app: zoom.clone(), meeting: None }));
        d.answered();
        assert_eq!(d.step(t + MIN, &[zoom.clone()], false), None);
        // The call ended; the next one is asked about again.
        assert_eq!(d.step(t + 2 * MIN, &[], false), None);
        assert!(matches!(d.step(t + 3 * MIN, &[zoom], false), Some(Action::Ask { .. })));
    }

    #[test]
    fn the_recording_stops_once_the_call_app_lets_the_microphone_go() {
        let mut d = Detector::default();
        let t = 1_000 * MIN;
        let zoom = app("us.zoom.xos");
        d.following(&zoom.id);
        assert_eq!(d.step(t, &[zoom.clone()], true), None);
        assert_eq!(d.step(t + MIN, &[], true), None);
        assert_eq!(d.step(t + MIN + QUIET_STOP_MS / 2, &[], true), None);
        // Back for a moment (muted and unmuted): it waits again.
        assert_eq!(d.step(t + 2 * MIN, &[zoom], true), None);
        assert_eq!(d.step(t + 3 * MIN, &[], true), None);
        assert_eq!(d.step(t + 3 * MIN + QUIET_STOP_MS, &[], true), Some(Action::Stop));
    }

    #[test]
    fn a_recording_started_by_hand_is_left_alone() {
        let mut d = Detector::default();
        assert_eq!(d.step(1_000 * MIN, &[], true), None);
        assert_eq!(d.step(1_010 * MIN, &[], true), None);
    }
}
