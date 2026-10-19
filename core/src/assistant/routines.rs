//! The assistant's routines: the morning recap at a set time, and quiet
//! hours, when notifications arrive without a sound or a knock (only urgent
//! ones still do). Part of the [`Settings`](super::Settings).

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveTime, Weekday};
use serde::{Deserialize, Serialize};

/// What the recap is asked to do, unless the settings give their own words.
pub const RECAP_TASK: &str = "Prepare the user's morning recap. Look at what their connected services hold for today: \
the calendar (today's events and the first one coming), the unread mail that matters (from people, not newsletters), \
anything pending or due in their tasks and issues, and what the memory says they care about. \
Then notify once, urgency normal, source Yoru, title \"Good morning\" (in the user's language): a text of at most five short sentences \
with what matters today, the most important first. If a service isn't connected, leave it out without saying so.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Routines {
    pub recap: bool,
    /// `HH:MM`, local time.
    pub recap_at: String,
    pub recap_weekdays_only: bool,
    /// The recap's own instructions; empty for [`RECAP_TASK`].
    pub recap_task: String,
    pub quiet: bool,
    /// `HH:MM`; quiet hours may run past midnight.
    pub quiet_from: String,
    pub quiet_to: String,
}

impl Default for Routines {
    fn default() -> Self {
        Self {
            recap: true,
            recap_at: "08:30".into(),
            recap_weekdays_only: false,
            recap_task: String::new(),
            quiet: true,
            quiet_from: "22:30".into(),
            quiet_to: "07:30".into(),
        }
    }
}

impl Routines {
    /// Whether `now` falls in the quiet hours.
    pub fn is_quiet(&self, now: NaiveTime) -> bool {
        let (Some(from), Some(to)) = (parse(&self.quiet_from), parse(&self.quiet_to)) else { return false };
        self.quiet && if from <= to { now >= from && now < to } else { now >= from || now < to }
    }

    pub fn quiet_now(&self) -> bool {
        self.is_quiet(Local::now().time())
    }

    /// Whether the recap is due at `now`, last given on `last`. It is given
    /// once a day, from its time until noon: a computer that was asleep at
    /// 8:30 still gets it at 9.
    pub fn recap_due(&self, now: DateTime<Local>, last: Option<NaiveDate>) -> bool {
        let Some(at) = parse(&self.recap_at) else { return false };
        let today = now.date_naive();
        let weekend = matches!(now.weekday(), Weekday::Sat | Weekday::Sun);
        let until = at.max(NaiveTime::from_hms_opt(12, 0, 0).unwrap());
        self.recap && !(self.recap_weekdays_only && weekend) && last != Some(today) && now.time() >= at && now.time() < until
    }

    pub fn recap_task(&self) -> &str {
        if self.recap_task.trim().is_empty() { RECAP_TASK } else { &self.recap_task }
    }
}

fn parse(hhmm: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(hhmm.trim(), "%H:%M").ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn quiet_hours_cross_midnight() {
        let r = Routines::default();
        assert!(r.is_quiet(t(23, 0)) && r.is_quiet(t(3, 0)));
        assert!(!r.is_quiet(t(7, 30)) && !r.is_quiet(t(12, 0)));
        let day = Routines { quiet_from: "13:00".into(), quiet_to: "15:00".into(), ..Routines::default() };
        assert!(day.is_quiet(t(14, 0)) && !day.is_quiet(t(15, 0)));
        assert!(!Routines { quiet: false, ..Routines::default() }.is_quiet(t(23, 0)));
    }

    #[test]
    fn the_recap_comes_once_a_morning() {
        let r = Routines::default();
        // Monday 19 October 2026.
        let at = |h, m| Local.with_ymd_and_hms(2026, 10, 19, h, m, 0).unwrap();
        assert!(!r.recap_due(at(8, 0), None));
        assert!(r.recap_due(at(8, 30), None));
        assert!(r.recap_due(at(10, 0), None));
        assert!(!r.recap_due(at(12, 30), None));
        assert!(!r.recap_due(at(9, 0), Some(at(9, 0).date_naive())));
        let weekdays = Routines { recap_weekdays_only: true, ..Routines::default() };
        let sunday = Local.with_ymd_and_hms(2026, 10, 18, 9, 0, 0).unwrap();
        assert!(!weekdays.recap_due(sunday, None) && r.recap_due(sunday, None));
        assert_eq!(r.recap_task(), RECAP_TASK);
    }
}
