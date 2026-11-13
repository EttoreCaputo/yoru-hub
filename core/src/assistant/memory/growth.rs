//! How far Yoru has grown with the user (docs/assistant.md, Growing up): a
//! few real things counted from the memory, sent to the board, which turns
//! each milestone into something new (a colour, a gesture of its own).
//! Only counts: what they unlock is the board's (yoru docs/CONCEPT.md).

use serde::Serialize;

use super::Memory;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Growth {
    /// Days the user talked with Yoru (by voice or in the app).
    pub days: u32,
    /// What Yoru knows about the user, still true.
    pub facts: u32,
    /// The people Yoru knows by name.
    pub people: u32,
    /// Notifications the user called useful.
    pub useful: u32,
}

impl Memory {
    pub fn growth(&self) -> Growth {
        let user = self.user();
        let count = |sql: &str, p: &[&dyn rusqlite::ToSql]| self.with_db(|db| db.query_row(sql, p, |r| r.get::<_, u32>(0)).unwrap_or(0));
        Growth {
            days: count("SELECT COUNT(DISTINCT date(started / 1000, 'unixepoch', 'localtime')) FROM episodes WHERE source = 'conversation'", &[]),
            facts: count("SELECT COUNT(*) FROM facts WHERE about = ?1 AND valid_to IS NULL AND section != 'quiet'", &[&user]),
            people: count("SELECT COUNT(*) FROM entities WHERE is_user = 0 AND placeholder = 0 AND kind = 'person'", &[]),
            useful: self.useful_count(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_what_is_known() {
        let m = Memory::open(None);
        assert_eq!(m.growth(), Growth::default());
        m.log_turn("Ettore", "ciao", "ciao!");
        m.remember("Likes espresso", None, "about", false, false, None).unwrap();
        m.remember("Newsletters", None, "quiet", false, false, None).unwrap();
        m.relate_tool("Anna", "sibling", None, Some("f")).unwrap();
        let g = m.growth();
        assert_eq!((g.days, g.facts, g.people, g.useful), (1, 1, 1, 0));
    }
}
