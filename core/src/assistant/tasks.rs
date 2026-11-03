//! Tasks: jobs the user gives Yoru to do on his own, once or on a schedule
//! ("every Monday at 9, send me the open issues", "on the 1st of the month,
//! sum up last month's invoices", "every 3 hours, look at the server's
//! status"). Unlike a watcher, a task always runs when its time comes and
//! may act, as far as the user allowed it ([`Acting`]).
//!
//! Kept in `tasks.json` in the data folder; set in the app (the Tasks page)
//! or by voice (the `tasks` tool, [`Tasks::call_tool`]). The scheduler
//! ([`super::watchers::run_scheduler`]) runs the due ones with [`run`], and
//! the ones the board asked for ("Do it now", `task_run`). The board's Tasks
//! app shows them ([`Tasks::board_list`], `set_tasks`, firmware 0.16.0).
//!
//! A time missed while the computer was asleep or off is still done within
//! [`GRACE`] of it; later than that it is counted as missed and the task
//! waits for its next time.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::inbox::now_ms;
use super::{Assistant, Host};
use crate::link::Tool;

/// How late a missed time is still done.
pub const GRACE: Duration = Duration::hours(6);
/// More would be noise on the page and in the prompt.
pub const MAX: usize = 40;
/// Times looked at to find the last one due: a year of hourly runs.
const SCAN_MAX: usize = 9_000;
const NAME_MAX: usize = 60;
const RESULT_MAX: usize = 600;
/// The board's Tasks app shows this many (`TASKS_MAX` in its `tasks.h`).
pub const BOARD_MAX: usize = 8;
/// The board's fields: its name, and what to do and what was done.
const BOARD_NAME: usize = 32;
const BOARD_TEXT: usize = 100;

/// When a task runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "repeat", rename_all = "lowercase")]
pub enum Schedule {
    /// On `date` at `at` (`HH:MM`), then the task turns itself off.
    Once { date: NaiveDate, at: String },
    /// Every day at `at`, or only on `days` (1 Monday to 7 Sunday).
    Daily {
        at: String,
        #[serde(default)]
        days: Vec<u8>,
    },
    /// On `day` of every month at `at`; past the month's end, on its last day.
    Monthly { day: u32, at: String },
    /// Every `every` hours from when it was set.
    Hourly { every: u32 },
}

impl Schedule {
    fn check(&self) -> Result<(), String> {
        match self {
            Schedule::Once { at, .. } | Schedule::Daily { at, .. } | Schedule::Monthly { at, .. } if parse(at).is_none() => Err("the time must be HH:MM".into()),
            Schedule::Daily { days, .. } if days.iter().any(|d| !(1..=7).contains(d)) => Err("days go from 1 (Monday) to 7 (Sunday)".into()),
            Schedule::Monthly { day, .. } if !(1..=31).contains(day) => Err("the day of the month goes from 1 to 31".into()),
            Schedule::Hourly { every } if !(1..=168).contains(every) => Err("every 1 to 168 hours".into()),
            _ => Ok(()),
        }
    }

    /// The first time after `t`; `from` is where hourly runs count from.
    pub fn next_after(&self, from: DateTime<Local>, t: DateTime<Local>) -> Option<DateTime<Local>> {
        match self {
            Schedule::Once { date, at } => local(date.and_time(parse(at)?)).filter(|o| *o > t),
            Schedule::Daily { at, days } => {
                let at = parse(at)?;
                (0..=8).map(|n| t.date_naive() + Duration::days(n)).filter(|d| days.is_empty() || days.contains(&(d.weekday().number_from_monday() as u8))).filter_map(|d| local(d.and_time(at))).find(|o| *o > t)
            }
            Schedule::Monthly { day, at } => {
                let at = parse(at)?;
                let (mut y, mut m) = (t.year(), t.month());
                for _ in 0..14 {
                    let date = NaiveDate::from_ymd_opt(y, m, (*day).min(month_days(y, m)))?;
                    if let Some(o) = local(date.and_time(at)).filter(|o| *o > t) {
                        return Some(o);
                    }
                    (y, m) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
                }
                None
            }
            Schedule::Hourly { every } => {
                let step = Duration::hours(i64::from((*every).max(1)));
                let n = if t < from { 1 } else { (t - from).num_seconds() / step.num_seconds() + 1 };
                Some(from + step * n as i32)
            }
        }
    }

    /// The last time in (`since`, `now`], if any.
    fn last_due(&self, from: DateTime<Local>, since: DateTime<Local>, now: DateTime<Local>) -> Option<DateTime<Local>> {
        let mut last = None;
        let mut t = since;
        for _ in 0..SCAN_MAX {
            match self.next_after(from, t) {
                Some(o) if o <= now => (last, t) = (Some(o), o),
                _ => break,
            }
        }
        last
    }

    /// How late a missed time may still be done.
    fn grace(&self) -> Duration {
        match self {
            Schedule::Hourly { every } => GRACE.min(Duration::hours(i64::from(*every))),
            _ => GRACE,
        }
    }

    /// In words, for the model.
    fn describe(&self) -> String {
        const DAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
        match self {
            Schedule::Once { date, at } => format!("once, on {date} at {at}"),
            Schedule::Daily { at, days } if days.is_empty() => format!("every day at {at}"),
            Schedule::Daily { at, days } => {
                let names: Vec<&str> = days.iter().filter_map(|d| DAYS.get(usize::from(*d).wrapping_sub(1)).copied()).collect();
                format!("every {} at {at}", names.join(", "))
            }
            Schedule::Monthly { day, at } => format!("on day {day} of every month at {at}"),
            Schedule::Hourly { every: 1 } => "every hour".into(),
            Schedule::Hourly { every } => format!("every {every} hours"),
        }
    }
}

/// How far a task may go.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Acting {
    /// It reads and tells, and changes nothing.
    Never,
    /// What changes something asks first, on the board and in the app.
    #[default]
    Ask,
    /// It acts without asking: only the user sets this, in the app.
    Free,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub name: String,
    /// What to do, in the user's words.
    pub what: String,
    #[serde(flatten)]
    pub schedule: Schedule,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub acting: Acting,
}

fn yes() -> bool {
    true
}

/// What a task has done, kept with it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskState {
    /// Unix ms hourly runs count from: when it was set or its schedule changed.
    #[serde(default)]
    pub from: u64,
    /// Unix ms: its times up to here are done (or missed).
    #[serde(default)]
    pub since: u64,
    #[serde(default)]
    pub last_run: u64,
    #[serde(default)]
    pub last_error: Option<String>,
    /// Yoru's line about what he did, last time.
    #[serde(default)]
    pub last_result: String,
    #[serde(default)]
    pub runs: u32,
    /// Times it couldn't be done, the computer being off.
    #[serde(default)]
    pub missed: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskEntry {
    pub task: Task,
    #[serde(default)]
    pub state: TaskState,
    /// Unix ms of its next time, for the app; not kept.
    #[serde(default, skip_deserializing)]
    pub next: Option<u64>,
    /// Being done right now; not kept.
    #[serde(default, skip_deserializing)]
    pub running: bool,
}

pub type TasksListener = Arc<dyn Fn(&[TaskEntry]) + Send + Sync>;

pub struct Tasks {
    file: Option<PathBuf>,
    items: Mutex<Vec<TaskEntry>>,
    listeners: Mutex<Vec<TasksListener>>,
    /// Goes up with every change, for the boards' copies.
    revision: AtomicU64,
    running: Mutex<Vec<String>>,
    /// Asked for from the board, for the scheduler to do.
    asked: Mutex<Vec<String>>,
}

impl std::fmt::Debug for Tasks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tasks").field("file", &self.file).finish_non_exhaustive()
    }
}

impl Tasks {
    /// The tasks kept in `file` (`None`: in memory only).
    pub fn open(file: Option<PathBuf>) -> Self {
        let items = file.as_ref().and_then(|f| fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { file, items: Mutex::new(items), listeners: Mutex::default(), revision: AtomicU64::new(0), running: Mutex::default(), asked: Mutex::default() }
    }

    /// Changes whenever a task does, or starts or ends a run.
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }

    /// The board asked for task `id` now: the scheduler does it next.
    pub fn ask_run(&self, id: &str) -> Result<(), String> {
        self.get(id).ok_or_else(|| format!("no task {id}"))?;
        let mut asked = self.asked.lock().unwrap();
        if !asked.iter().any(|a| a == id) {
            asked.push(id.to_string());
        }
        Ok(())
    }

    pub fn has_asked(&self) -> bool {
        !self.asked.lock().unwrap().is_empty()
    }

    /// The tasks the board asked for, taken.
    pub fn take_asked(&self) -> Vec<Task> {
        let ids = std::mem::take(&mut *self.asked.lock().unwrap());
        ids.iter().filter_map(|id| self.get(id)).collect()
    }

    fn set_running(&self, id: &str, on: bool) {
        {
            let mut running = self.running.lock().unwrap();
            running.retain(|r| r != id);
            if on {
                running.push(id.to_string());
            }
        }
        self.told();
    }

    pub fn listen(&self, listener: TasksListener) {
        self.listeners.lock().unwrap().push(listener);
    }

    pub fn list(&self) -> Vec<TaskEntry> {
        self.list_at(Local::now())
    }

    fn list_at(&self, now: DateTime<Local>) -> Vec<TaskEntry> {
        let items = self.items.lock().unwrap();
        items
            .iter()
            .map(|e| {
                let next = e.task.enabled.then(|| e.task.schedule.next_after(ms_time(e.state.from), ms_time(e.state.since).max(now))).flatten();
                let running = self.running.lock().unwrap().contains(&e.task.id);
                TaskEntry { next: next.map(|n| n.timestamp_millis() as u64), running, ..e.clone() }
            })
            .collect()
    }

    /// `set_tasks`' arguments: the list as the board's Tasks app takes it,
    /// times in Unix seconds, texts cut to its fields.
    pub fn board_list(&self) -> Value {
        let cut = |s: &str, n: usize| s.chars().take(n).collect::<String>();
        let list: Vec<Value> = self
            .list()
            .into_iter()
            .take(BOARD_MAX)
            .map(|e| {
                let mut v = serde_json::to_value(&e.task.schedule).unwrap_or_else(|_| json!({}));
                v["id"] = json!(e.task.id);
                v["name"] = json!(cut(&e.task.name, BOARD_NAME));
                v["what"] = json!(cut(&e.task.what, BOARD_TEXT));
                v["acting"] = json!(e.task.acting);
                v["enabled"] = json!(e.task.enabled);
                v["next"] = json!(e.next.map_or(0, |n| n / 1000));
                v["last"] = json!(e.state.last_run / 1000);
                v["result"] = json!(cut(&e.state.last_result, BOARD_TEXT));
                v["failed"] = json!(e.state.last_error.is_some());
                v["running"] = json!(e.running);
                v
            })
            .collect();
        json!({"tasks": list})
    }

    /// Add `task`, or change the one with its id. A new schedule, or a task
    /// turned back on, counts its times from now: nothing old comes due.
    pub fn put(&self, task: Task) -> Result<TaskEntry, String> {
        self.put_at(task, Local::now())
    }

    fn put_at(&self, mut task: Task, now: DateTime<Local>) -> Result<TaskEntry, String> {
        task.what = task.what.trim().to_string();
        if task.what.is_empty() {
            return Err("a task needs something to do".into());
        }
        task.name = task.name.trim().chars().take(NAME_MAX).collect();
        if task.name.is_empty() {
            task.name = task.what.chars().take(NAME_MAX).collect();
        }
        if let Schedule::Daily { days, .. } = &mut task.schedule {
            days.sort_unstable();
            days.dedup();
            if days.len() == 7 {
                days.clear();
            }
        }
        task.schedule.check()?;
        let now_ms = now.timestamp_millis() as u64;
        let entry = self.change(|items| {
            let i = items.iter().position(|e| !task.id.is_empty() && e.task.id == task.id);
            let fresh = match i {
                Some(i) => items[i].task.schedule != task.schedule || (!items[i].task.enabled && task.enabled),
                None => true,
            };
            if fresh && task.enabled && matches!(task.schedule, Schedule::Once { .. }) && task.schedule.next_after(now, now).is_none() {
                return Err("that time has passed already".into());
            }
            let i = match i {
                Some(i) => {
                    items[i].task = task;
                    i
                }
                None => {
                    if items.len() >= MAX {
                        return Err(format!("there are {MAX} tasks already"));
                    }
                    task.id = new_id(&task.name, items);
                    items.push(TaskEntry { task, state: TaskState::default(), next: None, running: false });
                    items.len() - 1
                }
            };
            if fresh {
                items[i].state.from = now_ms;
                items[i].state.since = now_ms;
            }
            Ok(items[i].task.id.clone())
        })?;
        self.list_at(now).into_iter().find(|e| e.task.id == entry).ok_or_else(|| "the task went missing".into())
    }

    /// Remove the task `which` names (its id or name); its name.
    pub fn remove(&self, which: &str) -> Result<String, String> {
        self.change(|items| {
            let i = find(items, which)?;
            Ok(items.remove(i).task.name)
        })
    }

    pub fn get(&self, id: &str) -> Option<Task> {
        self.items.lock().unwrap().iter().find(|e| e.task.id == id).map(|e| e.task.clone())
    }

    /// The tasks due at `now`, each marked as taken. Times missed for longer
    /// than their grace are counted, not run; a one-off task turns off.
    pub fn take_due(&self, now: DateTime<Local>) -> Vec<Task> {
        let mut due = Vec::new();
        let _ = self.change(|items| {
            let mut any = false;
            for e in items.iter_mut().filter(|e| e.task.enabled) {
                let Some(o) = e.task.schedule.last_due(ms_time(e.state.from), ms_time(e.state.since), now) else { continue };
                any = true;
                e.state.since = o.timestamp_millis() as u64;
                if now - o <= e.task.schedule.grace() {
                    due.push(e.task.clone());
                } else {
                    // The app says so in its own words: no error to keep.
                    e.state.missed += 1;
                }
                if matches!(e.task.schedule, Schedule::Once { .. }) {
                    e.task.enabled = false;
                }
            }
            if any { Ok(()) } else { Err(String::new()) }
        });
        due
    }

    /// Keep how a run went.
    pub fn record(&self, id: &str, outcome: &Result<String, String>) {
        let _ = self.change(|items| {
            let i = find(items, id)?;
            let s = &mut items[i].state;
            s.last_run = now_ms();
            s.runs += 1;
            match outcome {
                Ok(line) => {
                    s.last_result = line.trim().chars().take(RESULT_MAX).collect();
                    s.last_error = None;
                }
                Err(e) => s.last_error = Some(e.clone()),
            }
            Ok(())
        });
    }

    /// `f` on the list; saved and told when it succeeds.
    fn change<T>(&self, f: impl FnOnce(&mut Vec<TaskEntry>) -> Result<T, String>) -> Result<T, String> {
        let out = {
            let mut items = self.items.lock().unwrap();
            let out = f(&mut items)?;
            self.save(&items);
            out
        };
        self.told();
        Ok(out)
    }

    /// A new revision, and the listeners told.
    fn told(&self) {
        self.revision.fetch_add(1, Ordering::Relaxed);
        let list = self.list();
        let listeners = self.listeners.lock().unwrap().clone();
        for listener in listeners {
            listener(&list);
        }
    }

    fn save(&self, items: &[TaskEntry]) {
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(items) {
            let tmp = file.with_extension("json.tmp");
            if fs::write(&tmp, json).is_ok() {
                let _ = fs::rename(tmp, file);
            }
        }
    }

    /// The model's one tool for them. It may not let a task act without
    /// asking: a mail read by another task could otherwise set one up.
    pub fn tool() -> Value {
        json!({
            "name": "tasks",
            "description": "Jobs the user gives you to do on your own, once or on a schedule (\"every Monday at 9 sum up my open issues\", \
\"tomorrow at 18 check whether Marco answered\"). Not for ringing at a time (that's a timer) nor for the user's own to-dos. \
list them; add one: what (the job, in the user's words), name (a few words), repeat (once: date and at; daily: at, days to limit it; \
monthly: day and at; hourly: every); remove one by name.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["list", "add", "remove"]},
                    "name": {"type": "string", "description": "A few words (\"Weekly issues\")"},
                    "what": {"type": "string", "description": "The job, as you'll be told it when it's time"},
                    "repeat": {"type": "string", "enum": ["once", "daily", "monthly", "hourly"]},
                    "at": {"type": "string", "description": "HH:MM, local time"},
                    "date": {"type": "string", "description": "YYYY-MM-DD, for once"},
                    "days": {"type": "array", "items": {"type": "integer"}, "description": "For daily: 1 Monday to 7 Sunday; none: every day"},
                    "day": {"type": "integer", "description": "Day of the month, for monthly"},
                    "every": {"type": "integer", "description": "Hours, for hourly"},
                },
                "required": ["action"],
            },
        })
    }

    /// Run the `tasks` tool.
    pub fn call_tool(&self, input: &Value) -> Result<String, String> {
        let text = |k: &str| input[k].as_str().map(str::trim).filter(|s| !s.is_empty());
        match input["action"].as_str().unwrap_or("list") {
            "add" => {
                let at = || text("at").map(str::to_string).ok_or_else(|| "at (HH:MM) is required".to_string());
                let schedule = match text("repeat").unwrap_or("once") {
                    "once" => {
                        let date = text("date").ok_or("date is required for once")?;
                        Schedule::Once { date: NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| "date must be YYYY-MM-DD")?, at: at()? }
                    }
                    "daily" => {
                        let days = input["days"].as_array().map(|d| d.iter().filter_map(Value::as_u64).map(|d| d as u8).collect()).unwrap_or_default();
                        Schedule::Daily { at: at()?, days }
                    }
                    "monthly" => Schedule::Monthly { day: input["day"].as_u64().ok_or("day is required for monthly")? as u32, at: at()? },
                    "hourly" => Schedule::Hourly { every: input["every"].as_u64().unwrap_or(1) as u32 },
                    other => return Err(format!("unknown repeat {other}")),
                };
                let task = Task {
                    id: String::new(),
                    name: text("name").unwrap_or_default().into(),
                    what: text("what").ok_or("what is required")?.into(),
                    schedule,
                    enabled: true,
                    acting: Acting::Ask,
                };
                self.put(task).map(|e| format!("added {}", line(&e)))
            }
            "remove" => self.remove(text("name").ok_or("name is required")?).map(|n| format!("removed {n}")),
            _ => {
                let list = self.list();
                Ok(if list.is_empty() { "no tasks yet".into() } else { list.iter().map(line).collect::<Vec<_>>().join("\n") })
            }
        }
    }
}

/// A task in one line, for the model.
fn line(e: &TaskEntry) -> String {
    let next = e.next.map(|n| format!(", next {}", ms_time(n).format("%a %-d %b %H:%M"))).unwrap_or_else(|| ", off".into());
    format!("{}: {} ({}{next})", e.task.name, e.task.what, e.task.schedule.describe())
}

/// A host for a task, as far as it may act: `live` is the board and the
/// app's cards.
pub fn host<'a>(acting: Acting, live: &'a dyn Host) -> Box<dyn Host + 'a> {
    match acting {
        Acting::Never => Box::new(Allowing(live, None)),
        Acting::Ask => Box::new(Allowing(live, Some(None))),
        Acting::Free => Box::new(Allowing(live, Some(Some(true)))),
    }
}

/// `live`, answering confirmations itself: `None` refuses them all,
/// `Some(None)` asks `live`, `Some(Some(yes))` answers `yes`.
struct Allowing<'a>(&'a dyn Host, Option<Option<bool>>);

impl Host for Allowing<'_> {
    fn board_tools(&self) -> Vec<Tool> {
        self.0.board_tools()
    }

    fn call_board(&self, name: &str, arguments: &Value) -> Result<String, String> {
        self.0.call_board(name, arguments)
    }

    fn confirm(&self, title: &str, text: &str, detail: &str) -> Option<bool> {
        match self.1 {
            None => Some(false),
            Some(None) => self.0.confirm(title, text, detail),
            Some(answer) => answer,
        }
    }
}

/// Do `task` now with `live` as its host, and keep how it went.
pub fn run(assistant: &Assistant, task: &Task, live: &dyn Host) -> Result<String, String> {
    let acting = match task.acting {
        Acting::Never => "This task may not change anything: read and tell only; tools that act are refused.",
        Acting::Ask => "Tools that change something ask the user first, on the board and in the app; if they say no or don't answer, it isn't done: say so when you tell them.",
        Acting::Free => "The user allowed this task to act without asking: do what it says, including what sends, creates or changes something.",
    };
    let prompt = format!(
        "The user gave you this task to do on your own, {when}: \"{name}\".\n<assigned>\n{what}\n</assigned>\n\n{acting}\n{before}\n\
Do it now. When it's done, tell the user with notify (source \"{name}\") what you did or found, briefly, unless the task says \
to tell them only in some case and it didn't come up.",
        when = task.schedule.describe(),
        name = task.name,
        what = task.what,
        before = told_before(assistant, &task.id),
    );
    let first = assistant.inbox.list().first().map(|n| n.id.clone());
    assistant.tasks.set_running(&task.id, true);
    let outcome = assistant.run_task(&prompt, host(task.acting, live).as_ref()).map(|r| r.text);
    assistant.tasks.set_running(&task.id, false);
    let new: Vec<String> = assistant.inbox.list().into_iter().take_while(|n| Some(&n.id) != first.as_ref()).map(|n| n.id).collect();
    assistant.inbox.tag(&new, &task.id);
    assistant.tasks.record(&task.id, &outcome);
    outcome
}

/// The newest notifications a task gave, so it knows what it said last time.
fn told_before(assistant: &Assistant, id: &str) -> String {
    let told: Vec<String> = assistant.inbox.list().into_iter().filter(|n| n.by.as_deref() == Some(id)).take(5).map(|n| format!("- {}: {}", n.title, n.text)).collect();
    if told.is_empty() { String::new() } else { format!("What you told the user the last times:\n{}\n", told.join("\n")) }
}

fn find(items: &[TaskEntry], which: &str) -> Result<usize, String> {
    let which = which.trim();
    let lower = which.to_lowercase();
    items
        .iter()
        .position(|e| e.task.id == which)
        .or_else(|| items.iter().position(|e| e.task.name.to_lowercase() == lower))
        .or_else(|| items.iter().position(|e| !lower.is_empty() && e.task.name.to_lowercase().contains(&lower)))
        .ok_or_else(|| if items.is_empty() { "there are no tasks yet".to_string() } else { format!("no task like {which}") })
}

/// From the name, ASCII and unique among `items`.
fn new_id(name: &str, items: &[TaskEntry]) -> String {
    let base: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let base: String = base.split('-').filter(|w| !w.is_empty()).collect::<Vec<_>>().join("-").chars().take(32).collect();
    let base = if base.is_empty() { "task".to_string() } else { base };
    let mut id = base.clone();
    let mut n = 2;
    while items.iter().any(|e| e.task.id == id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

fn parse(hhmm: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(hhmm.trim(), "%H:%M").ok()
}

/// `naive` as local time; in the hour a clock change skips, an hour later.
fn local(naive: NaiveDateTime) -> Option<DateTime<Local>> {
    Local.from_local_datetime(&naive).earliest().or_else(|| Local.from_local_datetime(&(naive + Duration::hours(1))).earliest())
}

fn ms_time(ms: u64) -> DateTime<Local> {
    Local.timestamp_millis_opt(ms as i64).single().unwrap_or_else(Local::now)
}

fn month_days(y: i32, m: u32) -> u32 {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    NaiveDate::from_ymd_opt(ny, nm, 1).and_then(|d| d.pred_opt()).map_or(28, |d| d.day())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(d: u32, h: u32, m: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 11, d, h, m, 0).unwrap()
    }

    fn task(schedule: Schedule) -> Task {
        Task { id: String::new(), name: "Issues".into(), what: "Sum up my open issues".into(), schedule, enabled: true, acting: Acting::Ask }
    }

    #[test]
    fn next_times() {
        let from = at(2, 10, 0);
        // Tuesday 3 November 2026.
        let weekly = Schedule::Daily { at: "09:00".into(), days: vec![1] };
        assert_eq!(weekly.next_after(from, at(3, 12, 0)), Some(at(9, 9, 0)));
        let daily = Schedule::Daily { at: "09:00".into(), days: vec![] };
        assert_eq!(daily.next_after(from, at(3, 8, 0)), Some(at(3, 9, 0)));
        assert_eq!(daily.next_after(from, at(3, 9, 0)), Some(at(4, 9, 0)));
        let monthly = Schedule::Monthly { day: 31, at: "18:00".into() };
        assert_eq!(monthly.next_after(from, at(3, 12, 0)), Some(at(30, 18, 0)), "November has 30 days");
        let hourly = Schedule::Hourly { every: 3 };
        assert_eq!(hourly.next_after(from, at(2, 14, 30)), Some(at(2, 16, 0)));
        let once = Schedule::Once { date: NaiveDate::from_ymd_opt(2026, 11, 5).unwrap(), at: "07:15".into() };
        assert_eq!(once.next_after(from, at(3, 12, 0)), Some(at(5, 7, 15)));
        assert_eq!(once.next_after(from, at(5, 7, 15)), None);
        assert!(Schedule::Daily { at: "9".into(), days: vec![] }.check().is_err());
        assert!(Schedule::Daily { at: "09:00".into(), days: vec![8] }.check().is_err());
    }

    #[test]
    fn due_once_and_missed_beyond_the_grace() {
        let tasks = Tasks::open(None);
        let daily = tasks.put_at(task(Schedule::Daily { at: "09:00".into(), days: vec![] }), at(2, 20, 0)).unwrap();
        assert_eq!(daily.next, Some(at(3, 9, 0).timestamp_millis() as u64));
        assert!(tasks.take_due(at(3, 8, 59)).is_empty());
        assert_eq!(tasks.take_due(at(3, 9, 0)).len(), 1);
        assert!(tasks.take_due(at(3, 9, 1)).is_empty(), "taken once");
        // Asleep from Tuesday to Thursday at 14: Thursday's is still done, Wednesday's just passes.
        let due = tasks.take_due(at(5, 14, 0));
        assert_eq!(due.len(), 1);
        assert_eq!(tasks.list()[0].state.missed, 0);
        assert!(tasks.take_due(at(6, 15, 30)).is_empty());
        assert_eq!(tasks.list()[0].state.missed, 1);
    }

    #[test]
    fn a_one_off_turns_off_and_a_new_schedule_starts_from_now() {
        let tasks = Tasks::open(None);
        let date = NaiveDate::from_ymd_opt(2026, 11, 4).unwrap();
        assert!(tasks.put_at(task(Schedule::Once { date, at: "07:00".into() }), at(4, 8, 0)).is_err(), "in the past");
        let e = tasks.put_at(task(Schedule::Once { date, at: "07:00".into() }), at(3, 8, 0)).unwrap();
        assert_eq!(tasks.take_due(at(4, 7, 5)).len(), 1);
        assert!(!tasks.list()[0].task.enabled);
        // Moved to the evening: from now on, nothing earlier is due.
        tasks.put_at(Task { schedule: Schedule::Daily { at: "07:00".into(), days: vec![] }, enabled: true, ..e.task }, at(4, 20, 0)).unwrap();
        assert!(tasks.take_due(at(4, 21, 0)).is_empty());
        assert_eq!(tasks.take_due(at(5, 7, 0)).len(), 1);
    }

    #[test]
    fn the_tool() {
        let tasks = Tasks::open(None);
        let added = tasks.call_tool(&json!({"action": "add", "name": "Weekly issues", "what": "Sum up my open GitHub issues", "repeat": "daily", "at": "09:00", "days": [1]})).unwrap();
        assert!(added.starts_with("added Weekly issues") && added.contains("every Monday at 09:00"), "{added}");
        assert_eq!(tasks.list()[0].task.acting, Acting::Ask, "the model can't make a task act freely");
        assert!(tasks.call_tool(&json!({"action": "add", "what": "x", "repeat": "monthly", "at": "10:00"})).is_err());
        assert!(tasks.call_tool(&json!({"action": "list"})).unwrap().contains("Sum up"));
        assert_eq!(tasks.call_tool(&json!({"action": "remove", "name": "weekly"})).unwrap(), "removed Weekly issues");
        let v = serde_json::to_value(task(Schedule::Hourly { every: 2 })).unwrap();
        assert_eq!((v["repeat"].as_str(), v["every"].as_u64()), (Some("hourly"), Some(2)));
    }

    #[test]
    fn the_board_list_and_its_asks() {
        let tasks = Tasks::open(None);
        let before = tasks.revision();
        let long = "x".repeat(300);
        tasks.put(Task { what: long, ..task(Schedule::Daily { at: "09:00".into(), days: vec![1, 3] }) }).unwrap();
        assert!(tasks.revision() > before);
        let list = tasks.board_list();
        let t = &list["tasks"][0];
        assert_eq!((t["id"].as_str(), t["repeat"].as_str(), t["days"][1].as_u64()), (Some("issues"), Some("daily"), Some(3)));
        assert_eq!(t["what"].as_str().unwrap().len(), BOARD_TEXT);
        assert!(t["next"].as_u64().unwrap() > 1_700_000_000 && t["next"].as_u64().unwrap() < 10_000_000_000, "seconds");
        assert_eq!((t["acting"].as_str(), t["running"].as_bool(), t["failed"].as_bool()), (Some("ask"), Some(false), Some(false)));
        assert!(tasks.ask_run("nope").is_err());
        tasks.ask_run("issues").unwrap();
        tasks.ask_run("issues").unwrap();
        assert!(tasks.has_asked());
        assert_eq!(tasks.take_asked().len(), 1);
        assert!(!tasks.has_asked());
        tasks.set_running("issues", true);
        assert!(tasks.list()[0].running && tasks.board_list()["tasks"][0]["running"] == true);
    }

    #[test]
    fn kept_in_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tasks.json");
        Tasks::open(Some(file.clone())).put(task(Schedule::Hourly { every: 4 })).unwrap();
        let again = Tasks::open(Some(file));
        assert_eq!(again.list()[0].task.id, "issues");
        assert!(again.list()[0].next.is_some());
    }
}
