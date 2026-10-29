//! Your habits: things to do every day ("water, 8 times", "a walk"), how many
//! times each is done today, and how many days in a row it was. Kept in
//! `habits.json` in the data folder; set by voice or in the app (the
//! `habits` tool, [`Habits::call_tool`]), shown and ticked off in the board's
//! Habits app (firmware 0.11.0, `set_habits` and `habit_check`).
//!
//! A day is the computer's: at midnight every count starts again from 0. A
//! day counts for the streak once its goal is reached, and stays counted if
//! the count drops again that day only until it is under the goal.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The board shows this many (`HABITS_MAX` in its `habits.h`); more aren't kept.
pub const MAX: usize = 8;
const MAX_GOAL: u32 = 99;
/// Days of history kept per habit: enough for any streak worth telling.
const KEEP_DAYS: usize = 400;
/// The board's name field.
const NAME_MAX: usize = 24;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Habit {
    /// Short and stable: the board answers with it.
    pub id: String,
    pub name: String,
    /// Times a day.
    pub goal: u32,
    /// `#RRGGBB`; the board picks one when it's missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Today's count, as of `day`.
    #[serde(default)]
    pub done: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day: Option<NaiveDate>,
    /// The days its goal was reached, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub full_days: Vec<NaiveDate>,
}

impl Habit {
    /// Today's count: 0 once the day it counted is over.
    pub fn done_on(&self, today: NaiveDate) -> u32 {
        if self.day == Some(today) { self.done } else { 0 }
    }

    /// Days in a row its goal was reached, up to yesterday, plus today once
    /// it is.
    pub fn streak_on(&self, today: NaiveDate) -> u32 {
        let mut day = if self.full_days.last() == Some(&today) { today } else { today.pred_opt().unwrap_or(today) };
        let mut n = 0;
        for d in self.full_days.iter().rev() {
            if *d != day {
                break;
            }
            n += 1;
            day = day.pred_opt().unwrap_or(day);
        }
        n
    }

    fn set_done(&mut self, today: NaiveDate, done: u32) {
        self.done = done.min(self.goal);
        self.day = Some(today);
        let full = self.done >= self.goal;
        let counted = self.full_days.last() == Some(&today);
        if full && !counted {
            self.full_days.push(today);
            let excess = self.full_days.len().saturating_sub(KEEP_DAYS);
            self.full_days.drain(..excess);
        } else if !full && counted {
            self.full_days.pop();
        }
    }

    fn matches(&self, which: &str) -> bool {
        let which = which.trim().to_lowercase();
        let name = self.name.to_lowercase();
        which == self.id || name == which || (!which.is_empty() && (name.contains(&which) || which.contains(&name)))
    }
}

/// A habit as the app and the tools show it, for today.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HabitView {
    pub id: String,
    pub name: String,
    pub goal: u32,
    pub done: u32,
    pub streak: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

pub type HabitsListener = Arc<dyn Fn(&[HabitView]) + Send + Sync>;

pub struct Habits {
    items: Mutex<Vec<Habit>>,
    file: Option<PathBuf>,
    /// Goes up with every change, for the boards' copies.
    revision: AtomicU64,
    listeners: Mutex<Vec<HabitsListener>>,
}

impl std::fmt::Debug for Habits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Habits").field("file", &self.file).finish_non_exhaustive()
    }
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

impl Habits {
    /// The habits kept in `file` (`None`: in memory only).
    pub fn open(file: Option<PathBuf>) -> Self {
        let items = file.as_ref().and_then(|f| fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { items: Mutex::new(items), file, revision: AtomicU64::new(0), listeners: Mutex::default() }
    }

    pub fn listen(&self, listener: HabitsListener) {
        self.listeners.lock().unwrap().push(listener);
    }

    /// Changes whenever a habit does; a new day changes [`today`] instead.
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }

    pub fn list(&self) -> Vec<HabitView> {
        self.list_on(today())
    }

    fn list_on(&self, today: NaiveDate) -> Vec<HabitView> {
        let items = self.items.lock().unwrap();
        items
            .iter()
            .map(|h| HabitView { id: h.id.clone(), name: h.name.clone(), goal: h.goal, done: h.done_on(today), streak: h.streak_on(today), color: h.color.clone() })
            .collect()
    }

    /// `set_habits`' arguments: today's list as the board's Habits app takes it.
    pub fn board_list(&self) -> Value {
        self.board_list_on(today())
    }

    fn board_list_on(&self, today: NaiveDate) -> Value {
        let list: Vec<Value> = self
            .list_on(today)
            .into_iter()
            .map(|h| {
                let mut v = json!({"id": h.id, "name": h.name, "done": h.done, "goal": h.goal, "streak": h.streak});
                if let Some(color) = h.color {
                    v["color"] = json!(color);
                }
                v
            })
            .collect();
        json!({"habits": list})
    }

    /// A new habit, `goal` times a day.
    pub fn add(&self, name: &str, goal: u32, color: Option<&str>) -> Result<HabitView, String> {
        let name: String = name.trim().chars().take(NAME_MAX).collect();
        if name.is_empty() {
            return Err("a habit needs a name".into());
        }
        if !(1..=MAX_GOAL).contains(&goal) {
            return Err(format!("a habit is done 1 to {MAX_GOAL} times a day"));
        }
        let color = match color.map(str::trim).filter(|c| !c.is_empty()) {
            Some(c) if is_color(c) => Some(c.to_uppercase()),
            Some(_) => return Err("color must be #RRGGBB".into()),
            None => None,
        };
        self.change(|items| {
            if items.iter().any(|h| h.name.eq_ignore_ascii_case(&name)) {
                return Err(format!("there is a habit called {name} already"));
            }
            if items.len() >= MAX {
                return Err(format!("there are {MAX} habits already, as many as the board shows"));
            }
            let id = new_id(&name, items);
            let habit = Habit { id, name, goal, color, done: 0, day: None, full_days: Vec::new() };
            items.push(habit.clone());
            Ok(HabitView { id: habit.id, name: habit.name, goal, done: 0, streak: 0, color: habit.color })
        })
    }

    /// Remove the habit `which` names (its id or name); its name.
    pub fn remove(&self, which: &str) -> Result<String, String> {
        self.change(|items| {
            let i = find(items, which)?;
            Ok(items.remove(i).name)
        })
    }

    /// Change a habit's name, goal or colour.
    pub fn edit(&self, which: &str, name: Option<&str>, goal: Option<u32>, color: Option<&str>) -> Result<(), String> {
        let today = today();
        self.change(|items| {
            let i = find(items, which)?;
            if let Some(goal) = goal {
                if !(1..=MAX_GOAL).contains(&goal) {
                    return Err(format!("a habit is done 1 to {MAX_GOAL} times a day"));
                }
                items[i].goal = goal;
                let done = items[i].done_on(today);
                items[i].set_done(today, done);
            }
            if let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) {
                items[i].name = name.chars().take(NAME_MAX).collect();
            }
            if let Some(color) = color {
                items[i].color = match color.trim() {
                    "" => None,
                    c if is_color(c) => Some(c.to_uppercase()),
                    _ => return Err("color must be #RRGGBB".into()),
                };
            }
            Ok(())
        })
    }

    /// Today's count of `which` set to `done` (kept within 0 and its goal),
    /// as the board's `habit_check` says it.
    pub fn set_done(&self, which: &str, done: u32) -> Result<HabitView, String> {
        self.set_done_on(today(), which, done)
    }

    fn set_done_on(&self, today: NaiveDate, which: &str, done: u32) -> Result<HabitView, String> {
        self.change(|items| {
            let i = find(items, which)?;
            items[i].set_done(today, done);
            let h = &items[i];
            Ok(HabitView { id: h.id.clone(), name: h.name.clone(), goal: h.goal, done: h.done, streak: h.streak_on(today), color: h.color.clone() })
        })
    }

    /// One more time today (`by` 1), or one less (-1).
    pub fn check(&self, which: &str, by: i32) -> Result<HabitView, String> {
        self.check_on(today(), which, by)
    }

    fn check_on(&self, today: NaiveDate, which: &str, by: i32) -> Result<HabitView, String> {
        let done = {
            let items = self.items.lock().unwrap();
            let h = &items[find(&items, which)?];
            (h.done_on(today) as i64 + by as i64).max(0) as u32
        };
        self.set_done_on(today, which, done)
    }

    fn change<T>(&self, f: impl FnOnce(&mut Vec<Habit>) -> Result<T, String>) -> Result<T, String> {
        let out = f(&mut self.items.lock().unwrap())?;
        self.revision.fetch_add(1, Ordering::Relaxed);
        self.save();
        let list = self.list();
        let listeners = self.listeners.lock().unwrap().clone();
        for listener in listeners {
            listener(&list);
        }
        Ok(out)
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(&*self.items.lock().unwrap()) {
            let tmp = file.with_extension("json.tmp");
            if fs::write(&tmp, json).is_ok() {
                let _ = fs::rename(tmp, file);
            }
        }
    }

    /// The one tool the model gets for them: one, not five, so a brain with
    /// a small budget doesn't pay for it.
    pub fn tool() -> Value {
        json!({
            "name": "habits",
            "description": "The user's daily habits (things to do every day, some several times: water 8 times, a walk), shown on Yoru's board with \
today's count and the streak. list them; add one (name, goal: times a day, 1 unless said); remove one; done: one more time today; undo: one less.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["list", "add", "remove", "done", "undo"]},
                    "name": {"type": "string", "description": "The habit, a word or two (\"water\")"},
                    "goal": {"type": "number", "description": "Times a day, for add"},
                },
                "required": ["action"],
            },
        })
    }

    /// Run the `habits` tool.
    pub fn call_tool(&self, input: &Value) -> Result<String, String> {
        let name = input["name"].as_str().map(str::trim).filter(|n| !n.is_empty());
        let named = || name.ok_or_else(|| "name is required".to_string());
        let line = |h: &HabitView| format!("{}: {} of {} today, streak {}", h.name, h.done, h.goal, h.streak);
        match input["action"].as_str().unwrap_or("list") {
            "add" => {
                let goal = input["goal"].as_f64().filter(|g| *g >= 1.0).map_or(1, |g| g.round() as u32);
                self.add(named()?, goal, None).map(|h| format!("added {}", line(&h)))
            }
            "remove" => self.remove(named()?).map(|n| format!("removed {n}")),
            "done" => self.check(named()?, 1).map(|h| line(&h)),
            "undo" => self.check(named()?, -1).map(|h| line(&h)),
            _ => {
                let list = self.list();
                Ok(if list.is_empty() { "no habits yet".into() } else { list.iter().map(line).collect::<Vec<_>>().join("\n") })
            }
        }
    }
}

fn find(items: &[Habit], which: &str) -> Result<usize, String> {
    items
        .iter()
        .position(|h| h.id == which.trim())
        .or_else(|| items.iter().position(|h| h.matches(which)))
        .ok_or_else(|| if items.is_empty() { "there are no habits yet".to_string() } else { format!("no habit like {which}") })
}

/// From the name, ASCII, unique among `items`, at most 23 bytes (the board's).
fn new_id(name: &str, items: &[Habit]) -> String {
    let base: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .take(18)
        .collect();
    let base = if base.is_empty() { "habit".to_string() } else { base };
    let mut id = base.clone();
    let mut n = 2;
    while items.iter().any(|h| h.id == id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

fn is_color(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|ch| ch.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    #[test]
    fn counts_and_streaks() {
        let habits = Habits::open(None);
        let water = habits.add("Water", 3, Some("#40c8e0")).unwrap();
        assert_eq!((water.id.as_str(), water.color.as_deref()), ("water", Some("#40C8E0")));
        for d in [26, 27] {
            habits.set_done_on(day(d), "water", 3).unwrap();
        }
        // Not done yet today: yesterday's streak still stands.
        assert_eq!(habits.list_on(day(28))[0].streak, 2);
        assert_eq!(habits.list_on(day(28))[0].done, 0);
        assert_eq!(habits.check_on(day(28), "Water", 1).unwrap().streak, 2);
        habits.check_on(day(28), "water", 1).unwrap();
        assert_eq!(habits.check_on(day(28), "water", 1).unwrap().streak, 3);
        // Past the goal it stays at the goal; under it, today stops counting.
        assert_eq!(habits.check_on(day(28), "water", 1).unwrap().done, 3);
        assert_eq!(habits.check_on(day(28), "water", -1).unwrap().streak, 2);
        // A day missed breaks it.
        assert_eq!(habits.list_on(day(30))[0].streak, 0);
    }

    #[test]
    fn the_board_list_and_the_tool() {
        let habits = Habits::open(None);
        let before = habits.revision();
        assert!(habits.call_tool(&json!({"action": "add", "name": "Walk"})).unwrap().starts_with("added Walk"));
        assert!(habits.revision() > before);
        assert!(habits.add("walk", 1, None).is_err());
        assert!(habits.add("Read", 0, None).is_err());
        habits.add("Two words!", 2, None).unwrap();
        let list = habits.board_list_on(day(28));
        assert_eq!(list["habits"][1]["id"], "two-words");
        assert_eq!(list["habits"][0]["goal"], 1);
        assert!(list["habits"][0].get("color").is_none());
        assert!(habits.call_tool(&json!({"action": "done", "name": "walk"})).unwrap().contains("1 of 1"));
        assert_eq!(habits.call_tool(&json!({"action": "remove", "name": "two"})).unwrap(), "removed Two words!");
        assert!(habits.call_tool(&json!({"action": "done", "name": "swim"})).is_err());
    }

    #[test]
    fn kept_in_a_file() {
        let dir = std::env::temp_dir().join(format!("yoru-habits-{}", std::process::id()));
        let file = dir.join("habits.json");
        Habits::open(Some(file.clone())).add("Water", 8, None).unwrap();
        let again = Habits::open(Some(file));
        assert_eq!(again.list()[0].goal, 8);
        let _ = fs::remove_dir_all(dir);
    }
}
