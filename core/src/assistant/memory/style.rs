//! How Yoru talks, tuned slowly and within limits (docs/memory.md, Style):
//! the user picks each setting (Settings, `style`); with each person Yoru
//! tries small changes in a corridor round it, each tied to a measure it
//! should improve, and keeps or undoes it after some turns. A few changes a
//! day at most, hours apart. Rules for a moment (late at night, softer)
//! apply only then and aren't saved.

use chrono::Timelike;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Memory, now_ms};

/// How far a setting may move from the user's value, either side.
pub const CORRIDOR: f32 = 20.0;
/// One change.
const STEP: f32 = 5.0;
/// Changes a day, at most, with each person.
const PER_DAY: i64 = 6;
/// Between two changes with one person.
const APART_MS: i64 = 2 * 3600 * 1000;
/// Turns after a change before it is judged.
const JUDGE_AFTER: i64 = 8;
/// Turns in a week before anything is tried.
const ENOUGH: i64 = 10;

/// The settings, each 0 to 100.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Style {
    /// 0: a few words; 100: a little more.
    pub length: f32,
    pub humour: f32,
    /// 0: casual; 100: formal.
    pub formality: f32,
    pub warmth: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self { length: 40.0, humour: 40.0, formality: 30.0, warmth: 60.0 }
    }
}

pub const SETTINGS: &[&str] = &["length", "humour", "formality", "warmth"];

/// The measures a change may aim at, and whether more is better.
pub const MEASURES: &[(&str, bool)] = &[("interruptions", false), ("mood", true), ("follow_ups", true)];

impl Style {
    fn get(&self, name: &str) -> f32 {
        match name {
            "length" => self.length,
            "humour" => self.humour,
            "formality" => self.formality,
            _ => self.warmth,
        }
    }

    fn set(&mut self, name: &str, v: f32) {
        let v = v.clamp(0.0, 100.0);
        match name {
            "length" => self.length = v,
            "humour" => self.humour = v,
            "formality" => self.formality = v,
            _ => self.warmth = v,
        }
    }

    /// Within the corridor round `chosen`.
    fn bounded(mut self, chosen: &Style) -> Style {
        for s in SETTINGS {
            let c = chosen.get(s);
            self.set(s, self.get(s).clamp(c - CORRIDOR, c + CORRIDOR));
        }
        self
    }

    /// What the prompt says of it, with `who`; `hour`: late at night, softer.
    pub fn line(&self, who: &str, hour: u32) -> String {
        let pick = |v: f32, w: [&'static str; 3]| if v < 35.0 { w[0] } else if v <= 65.0 { w[1] } else { w[2] };
        let mut out = format!(
            "Your way of talking with {who}: {}, {}, {}, {}.",
            pick(self.length, ["as few words as will do", "short answers", "short answers, with a detail more when it helps"]),
            pick(self.humour, ["serious, no jokes", "a joke now and then", "playful, with a joke when it fits"]),
            pick(self.formality, ["casual", "relaxed but polite", "formal"]),
            pick(self.warmth, ["matter-of-fact", "friendly", "warm and caring"]),
        );
        if !(7..22).contains(&hour) {
            out.push_str(" It is late: speak softly and calmly, with less energy.");
        }
        out
    }
}

/// A change being tried.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Trial {
    pub id: i64,
    pub setting: String,
    pub before: f32,
    pub measure: String,
    pub baseline: f32,
    pub at: i64,
}

/// A person's style: where it is now, and the change being tried.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PersonStyle {
    pub now: Option<Style>,
    pub trial: Option<Trial>,
}

impl Memory {
    /// How Yoru talks with `person` now: theirs, within the corridor round `chosen`.
    pub fn style(&self, person: i64, chosen: &Style) -> Style {
        let ps: PersonStyle = self.json(person, "style");
        ps.now.unwrap_or(*chosen).bounded(chosen)
    }

    /// A turn, for the measures.
    pub fn log_measures(&self, person: Option<i64>, words: usize, reply_words: usize, mood: f32, interrupted: bool, follow_up: bool) {
        self.with_db(|db| {
            let _ = db.execute(
                "INSERT INTO turns(at, person, words, reply_words, mood, interrupted, follow_up) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![now_ms(), person, words as i64, reply_words as i64, mood, interrupted, follow_up],
            );
        });
    }

    /// The people Yoru talked with since `since` (Unix ms).
    pub fn talked_with(&self, since: i64) -> Vec<i64> {
        self.with_db(|db| {
            let Ok(mut stmt) = db.prepare("SELECT DISTINCT person FROM turns WHERE person IS NOT NULL AND at >= ?1") else { return Vec::new() };
            stmt.query_map([since], |r| r.get(0)).map(|rows| rows.flatten().collect()).unwrap_or_default()
        })
    }

    /// The last turn was cut short by the person (a barge-in).
    pub fn mark_interrupted(&self) {
        self.with_db(|db| {
            let _ = db.execute("UPDATE turns SET interrupted = 1 WHERE id = (SELECT max(id) FROM turns)", []);
        });
    }

    /// `measure` over `person`'s turns since `since`, and how many turns.
    fn measure(&self, person: i64, measure: &str, since: i64) -> (f32, i64) {
        let column = match measure {
            "interruptions" => "interrupted",
            "follow_ups" => "follow_up",
            _ => "mood",
        };
        self.with_db(|db| {
            db.query_row(
                &format!("SELECT coalesce(avg({column}), 0), count(*) FROM turns WHERE coalesce(person, ?1) = ?1 AND at >= ?2"),
                rusqlite::params![person, since],
                |r| Ok((r.get::<_, f64>(0)? as f32, r.get(1)?)),
            )
            .unwrap_or((0.0, 0))
        })
    }

    fn turns_since(&self, person: i64, since: i64) -> i64 {
        self.measure(person, "mood", since).1
    }
}

/// The stats the brain is shown to propose a change.
fn stats(memory: &Memory, person: i64) -> Value {
    let week = now_ms() - 7 * 86_400_000;
    let (reply_words, words): (f64, f64) = memory.with_db(|db| {
        db.query_row(
            "SELECT coalesce(avg(reply_words), 0), coalesce(avg(words), 0) FROM turns WHERE coalesce(person, ?1) = ?1 AND at >= ?2",
            rusqlite::params![person, week],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or((0.0, 0.0))
    });
    let m = |name: &str| memory.measure(person, name, week).0;
    json!({
        "turns_this_week": memory.turns_since(person, week),
        "their_words_per_turn": words.round(),
        "your_words_per_reply": reply_words.round(),
        "interruptions": m("interruptions"),
        "mood": m("mood"),
        "follow_ups": m("follow_ups"),
    })
}

/// One pass of tuning for `person`: judge the change being tried once it has
/// enough turns, or try a new one if the day allows. What happened, if anything.
pub fn tune(memory: &Memory, person: i64, who: &str, chosen: &Style, ask: super::sleep::Ask) -> Result<Option<String>, String> {
    let mut ps: PersonStyle = memory.json(person, "style");
    let mut style = ps.now.unwrap_or(*chosen).bounded(chosen);
    if let Some(trial) = ps.trial.clone() {
        let (value, turns) = memory.measure(person, &trial.measure, trial.at);
        if turns < JUDGE_AFTER {
            return Ok(None);
        }
        let more_is_better = MEASURES.iter().find(|(m, _)| *m == trial.measure).is_none_or(|(_, b)| *b);
        let better = if more_is_better { value >= trial.baseline } else { value <= trial.baseline };
        if !better {
            style.set(&trial.setting, trial.before);
        }
        let outcome = if better { "kept" } else { "undone" };
        memory.with_db(|db| {
            let _ = db.execute("UPDATE style_changes SET outcome = ?1 WHERE id = ?2", rusqlite::params![outcome, trial.id]);
        });
        ps.trial = None;
        ps.now = Some(style);
        memory.set_json(person, "style", &ps);
        return Ok(Some(format!("{who}: {} {} ({} {:.2} against {:.2})", trial.setting, outcome, trial.measure, value, trial.baseline)));
    }
    let day_ago = now_ms() - 86_400_000;
    let (today, last): (i64, i64) = memory.with_db(|db| {
        db.query_row("SELECT count(*), coalesce(max(at), 0) FROM style_changes WHERE person = ?1 AND at >= ?2", rusqlite::params![person, day_ago], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap_or((0, 0))
    });
    if today >= PER_DAY || now_ms() - last < APART_MS || memory.turns_since(person, now_ms() - 7 * 86_400_000) < ENOUGH {
        return Ok(None);
    }
    let measures: Vec<&str> = MEASURES.iter().map(|(m, _)| *m).collect();
    let schema = json!({
        "type": "object",
        "properties": {
            "change": {"type": "boolean", "description": "False when nothing needs changing"},
            "setting": {"type": "string", "enum": SETTINGS},
            "direction": {"type": "string", "enum": ["up", "down"]},
            "measure": {"type": "string", "enum": measures, "description": "What the change should improve"},
            "why": {"type": "string"},
        },
        "required": ["change", "setting", "direction", "measure", "why"],
        "additionalProperties": false,
    });
    let prompt = format!(
        "You are Yoru, the user's assistant, looking at how you talk with {who}, to try one small change. \
         Your style now (0 to 100): {}. The user chose: {}; each setting may move at most {CORRIDOR} either side of it. \
         The last week with {who}: {}. interruptions: how often they cut you off (lower is better); mood: how they felt, -1 to 1 \
         (higher is better); follow_ups: how often they went on talking after your answer (higher is better). \
         Propose one change of one setting that should improve one measure, or none if all is well. It will be measured and undone if worse.",
        serde_json::to_string(&style).unwrap_or_default(),
        serde_json::to_string(chosen).unwrap_or_default(),
        stats(memory, person),
    );
    let answer: Value = serde_json::from_str(&ask(&prompt, &schema)?).map_err(|e| e.to_string())?;
    if answer["change"] != true {
        return Ok(None);
    }
    let setting = answer["setting"].as_str().filter(|s| SETTINGS.contains(s)).ok_or("no setting")?.to_string();
    let measure = answer["measure"].as_str().filter(|m| measures.contains(m)).ok_or("no measure")?.to_string();
    let before = style.get(&setting);
    let step = if answer["direction"] == "down" { -STEP } else { STEP };
    style.set(&setting, before + step);
    let style = style.bounded(chosen);
    let after = style.get(&setting);
    if (after - before).abs() < 0.5 {
        return Ok(None);
    }
    let baseline = memory.measure(person, &measure, now_ms() - 7 * 86_400_000).0;
    let id = memory.with_db(|db| {
        let _ = db.execute(
            "INSERT INTO style_changes(at, person, setting, before, after, measure, baseline) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![now_ms(), person, setting, before, after, measure, baseline],
        );
        db.last_insert_rowid()
    });
    ps.trial = Some(Trial { id, setting: setting.clone(), before, measure: measure.clone(), baseline, at: now_ms() });
    ps.now = Some(style);
    memory.set_json(person, "style", &ps);
    Ok(Some(format!("{who}: trying {setting} {before:.0} → {after:.0} for {measure}")))
}

/// The local hour, for [`Style::line`].
pub fn hour_now() -> u32 {
    chrono::Local::now().hour()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_stays_in_its_corridor_and_is_judged() {
        let memory = Memory::open(None);
        let user = memory.user();
        let chosen = Style::default();
        for _ in 0..12 {
            memory.log_measures(Some(user), 5, 30, 0.0, true, false);
        }
        std::thread::sleep(std::time::Duration::from_millis(3));
        let ask = |_: &str, _: &Value| -> Result<String, String> {
            Ok(json!({"change": true, "setting": "length", "direction": "down", "measure": "interruptions", "why": "cut off often"}).to_string())
        };
        let tried = tune(&memory, user, "Ettore", &chosen, &ask).unwrap().unwrap();
        assert!(tried.contains("length 40 → 35"), "{tried}");
        // Not twice within two hours.
        assert!(tune(&memory, user, "Ettore", &chosen, &ask).unwrap().is_none());
        for _ in 0..8 {
            memory.log_measures(Some(user), 5, 15, 0.2, false, true);
        }
        let judged = tune(&memory, user, "Ettore", &chosen, &ask).unwrap().unwrap();
        assert!(judged.contains("kept"), "{judged}");
        assert_eq!(memory.style(user, &chosen).length, 35.0);
        let far = Style { length: 90.0, ..chosen };
        assert_eq!(far.bounded(&chosen).length, 60.0);
        assert!(chosen.line("Ettore", 23).contains("late"));
    }
}
