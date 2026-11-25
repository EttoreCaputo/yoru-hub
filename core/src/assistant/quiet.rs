//! Quiet hours: when notifications arrive without a sound or a knock (only
//! urgent ones still do). Part of the [`Settings`](super::Settings); "do not
//! disturb" for a while is [`Assistant::set_quiet_until`](super::Assistant::set_quiet_until).

use chrono::{Local, NaiveTime};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct QuietHours {
    pub on: bool,
    /// `HH:MM`, local time; quiet hours may run past midnight.
    pub from: String,
    pub to: String,
}

impl Default for QuietHours {
    fn default() -> Self {
        Self { on: true, from: "22:30".into(), to: "07:30".into() }
    }
}

impl QuietHours {
    /// Whether `now` falls in the quiet hours.
    pub fn is_quiet(&self, now: NaiveTime) -> bool {
        let (Some(from), Some(to)) = (parse(&self.from), parse(&self.to)) else { return false };
        self.on && if from <= to { now >= from && now < to } else { now >= from || now < to }
    }

    pub fn quiet_now(&self) -> bool {
        self.is_quiet(Local::now().time())
    }
}

fn parse(hhmm: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(hhmm.trim(), "%H:%M").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn quiet_hours_cross_midnight() {
        let q = QuietHours::default();
        assert!(q.is_quiet(t(23, 0)) && q.is_quiet(t(3, 0)));
        assert!(!q.is_quiet(t(7, 30)) && !q.is_quiet(t(12, 0)));
        let day = QuietHours { from: "13:00".into(), to: "15:00".into(), ..QuietHours::default() };
        assert!(day.is_quiet(t(14, 0)) && !day.is_quiet(t(15, 0)));
        assert!(!QuietHours { on: false, ..QuietHours::default() }.is_quiet(t(23, 0)));
    }
}
