//! What the user makes of what Yoru tells them (docs/assistant.md, Feedback):
//! on the board an opened notification leaves with «Useful» or «Enough».
//! Each verdict is kept; «Enough» also goes in `me.md`'s "Don't tell me
//! about" ([`crate::assistant::notes::Notes::keep_quiet`]), and a source that keeps getting it arrives more quietly.

use rusqlite::params;

use super::{Memory, now_ms};
use crate::assistant::inbox::{Notification, Urgency};

/// The verdicts of one source that count, and for how long.
const RECENT: usize = 8;
const RECENT_MS: i64 = 30 * 24 * 3600 * 1000;
/// This many «Enough» among them, and more than the «Useful», brake the source.
const BRAKE_AT: usize = 2;

#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub at: i64,
    pub title: String,
    pub useful: bool,
}

impl Memory {
    /// The user's verdict on `n` («Enough» also goes in `me.md`: the caller's).
    pub fn judge(&self, n: &Notification, useful: bool) {
        self.with_db(|db| {
            let _ = db.execute(
                "INSERT INTO verdicts(at, source, by, title, useful) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![now_ms(), n.source, n.by, n.title, useful],
            );
        });
    }

    /// The newest verdicts on what `by` (a watcher or routine) told, or,
    /// without one, on what came from `source`.
    pub fn verdicts(&self, by: Option<&str>, source: &str) -> Vec<Verdict> {
        let (column, key) = by.map_or(("source", source), |by| ("by", by));
        self.with_db(|db| {
            let sql = format!("SELECT at, title, useful FROM verdicts WHERE {column} = ?1 AND at >= ?2 ORDER BY id DESC LIMIT ?3");
            let Ok(mut stmt) = db.prepare(&sql) else { return Vec::new() };
            stmt.query_map(params![key, now_ms() - RECENT_MS, RECENT as i64], |r| Ok(Verdict { at: r.get(0)?, title: r.get(1)?, useful: r.get(2)? }))
                .map(|rows| rows.flatten().collect())
                .unwrap_or_default()
        })
    }

    /// How loudly a notification of this source arrives: a step quieter once
    /// the user said «Enough» to it a few times, and not «Useful» as often.
    /// What can't wait still comes through.
    pub fn braked(&self, by: Option<&str>, source: &str, urgency: Urgency) -> Urgency {
        let verdicts = self.verdicts(by, source);
        let enough = verdicts.iter().filter(|v| !v.useful).count();
        if enough < BRAKE_AT || enough <= verdicts.len() - enough {
            return urgency;
        }
        match urgency {
            Urgency::High => Urgency::Normal,
            Urgency::Normal => Urgency::Low,
            other => other,
        }
    }

    /// The verdicts as the triage reads them; empty without any.
    pub fn verdict_lines(&self, by: Option<&str>, source: &str) -> String {
        self.verdicts(by, source).iter().map(|v| format!("- {}: {}", if v.useful { "useful" } else { "enough" }, v.title)).collect::<Vec<_>>().join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(source: &str, by: Option<&str>, title: &str) -> Notification {
        Notification { id: "n1".into(), source: source.into(), title: title.into(), text: String::new(), urgency: Urgency::Normal, url: None, at: 0, read: false, by: by.map(str::to_string) }
    }

    #[test]
    fn enough_is_kept_quiet_and_brakes_the_source() {
        let m = Memory::open(None);
        m.judge(&note("Gmail", Some("w1"), "A newsletter"), false);
        assert_eq!(m.braked(Some("w1"), "Gmail", Urgency::Normal), Urgency::Normal);
        m.judge(&note("Gmail", Some("w1"), "Another newsletter"), false);
        assert_eq!(m.braked(Some("w1"), "Gmail", Urgency::Normal), Urgency::Low);
        assert_eq!(m.braked(Some("w1"), "Gmail", Urgency::High), Urgency::Normal);
        assert_eq!(m.braked(Some("w1"), "Gmail", Urgency::Urgent), Urgency::Urgent);
        // Another watcher isn't touched.
        assert_eq!(m.braked(Some("w2"), "Gmail", Urgency::Normal), Urgency::Normal);
    }

    #[test]
    fn useful_lifts_the_brake() {
        let m = Memory::open(None);
        for title in ["a", "b"] {
            m.judge(&note("Calendar", None, title), false);
        }
        for title in ["c", "d"] {
            m.judge(&note("Calendar", None, title), true);
        }
        assert_eq!(m.braked(None, "Calendar", Urgency::Normal), Urgency::Normal);
        assert!(m.verdict_lines(None, "Calendar").starts_with("- useful: d\n- useful: c\n- enough: b"));
    }
}
