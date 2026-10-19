//! Rule-based commands in Italian and English, so timers, reminders, the
//! volume and the time work by voice without a language model, and "stop"
//! silences a ringing alarm at once.
//!
//! [`understand`] looks for a few keywords in what was heard: "che
//! ore"/"what time", "luminosità"/"brightness", "volume", "timer",
//! "ricordami"/"remind me", "pomodoro". It returns the tools to call and
//! what to answer; anything else is answered with "Non ho capito" /
//! "Sorry, I didn't get that".

use chrono::{Local, NaiveTime, Timelike};
use serde_json::{Value, json};

use crate::timers::{self, TimerInfo};

/// What the host knows besides the words.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// The speaker's volume, for "louder" (`get_state` has it).
    pub volume: Option<u8>,
    /// The timers running, for "how long is left".
    pub timers: Vec<TimerInfo>,
    /// A timer rings, or rang until the wake word a moment ago: "stop" is for it.
    pub alarm: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// Board tools to call in order: name and arguments.
    pub calls: Vec<(String, Value)>,
    /// What to say once they have run.
    pub reply: String,
    /// What to say instead if one of them fails.
    pub failure: String,
    /// False when nothing matched: `reply` says so.
    pub understood: bool,
}

/// The lowered, accent-free words of `text`, digits kept together.
fn words(text: &str) -> Vec<String> {
    let plain: String = text
        .chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ä' => 'a',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ò' | 'ó' | 'ô' | 'ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            c if c.is_alphanumeric() => c,
            _ => ' ',
        })
        .collect();
    plain.split_whitespace().map(str::to_string).collect()
}

fn has(words: &[String], any: &[&str]) -> bool {
    words.iter().any(|w| any.iter().any(|a| a.strip_suffix('*').map_or(w == a, |p| w.starts_with(p))))
}

/// Two words in a row ("che ore", "turn off").
fn has_pair(words: &[String], pairs: &[(&str, &str)]) -> bool {
    words.windows(2).any(|w| pairs.iter().any(|(a, b)| w[0] == *a && w[1].starts_with(b)))
}

const NUMBER_WORDS: &[(&str, u32)] = &[
    ("zero", 0), ("dieci", 10), ("venti", 20), ("trenta", 30), ("quaranta", 40), ("cinquanta", 50), ("sessanta", 60),
    ("settanta", 70), ("ottanta", 80), ("novanta", 90), ("cento", 100), ("ten", 10), ("twenty", 20), ("thirty", 30),
    ("forty", 40), ("fifty", 50), ("sixty", 60), ("seventy", 70), ("eighty", 80), ("ninety", 90), ("hundred", 100),
];

/// The first number, in digits or as a round word.
fn number(words: &[String]) -> Option<u32> {
    words.iter().find_map(|w| w.parse::<u32>().ok().or_else(|| NUMBER_WORDS.iter().find(|(n, _)| w == n).map(|&(_, v)| v)))
}

fn plan(calls: Vec<(String, Value)>, it: bool, reply: String) -> Plan {
    let failure = if it { "Non ci sono riuscito." } else { "That didn't work." }.to_string();
    Plan { calls, reply, failure, understood: true }
}

fn tool(name: &str, arguments: Value) -> (String, Value) {
    (name.to_string(), arguments)
}

/// What to do about `text`, said in `language` (`it`, or anything else for English).
pub fn understand(text: &str, language: &str, ctx: &Context) -> Plan {
    let it = language == "it";
    let w = words(text);

    if let Some(plan) = timer_plan(text, &w, it, ctx) {
        return plan;
    }

    if has_pair(&w, &[("che", "or"), ("what", "time")]) || has(&w, &["orario"]) {
        let now = Local::now();
        let (h, m) = (now.hour(), now.minute());
        let reply = match (it, m) {
            (true, 0) => format!("Sono le {h}."),
            (true, _) => format!("Sono le {h} e {m}."),
            (false, _) => format!("It's {h}:{m:02}."),
        };
        return plan(vec![], it, reply);
    }

    if has(&w, &["luminosit*", "brightness", "bright", "luce"]) {
        let pct = if has(&w, &["massima", "massimo", "max", "maximum", "full", "piena"]) {
            Some(100)
        } else if has(&w, &["minima", "minimo", "min", "minimum"]) {
            Some(5)
        } else if has(&w, &["meta", "half"]) {
            Some(50)
        } else {
            number(&w)
        };
        if let Some(pct) = pct.filter(|p| *p <= 100) {
            let reply = if it { format!("Luminosità al {pct} per cento.") } else { format!("Brightness {pct} percent.") };
            return plan(vec![tool("set_brightness", json!({"percent": pct}))], it, reply);
        }
    }

    let louder = has(&w, &["alza", "aumenta", "louder", "up"]) || has_pair(&w, &[("piu", "fort")]);
    let quieter = has(&w, &["abbassa", "diminuisci", "quieter", "down", "lower"]) || has_pair(&w, &[("piu", "pian")]);
    if has(&w, &["volume", "voce", "voice", "parla", "speak"]) || louder || quieter {
        let current = ctx.volume.unwrap_or(60) as i32;
        let pct = if has(&w, &["massimo", "massima", "max", "maximum", "full"]) {
            Some(100)
        } else if let Some(n) = number(&w).filter(|n| *n <= 100) {
            Some(n as i32)
        } else if louder {
            Some((current + 20).min(100))
        } else if quieter {
            Some((current - 20).max(10))
        } else {
            None
        };
        if let Some(pct) = pct {
            let reply = if it { format!("Volume al {pct}.") } else { format!("Volume {pct}.") };
            return plan(vec![tool("set_volume", json!({"percent": pct}))], it, reply);
        }
    }

    if has_pair(&w, &[("cosa", "sai"), ("what", "can")]) || has(&w, &["aiuto", "help"]) {
        let reply = if it {
            "Senza un modello posso dirti l'ora, cambiare luminosità e volume, \
             impostare timer, promemoria e un pomodoro."
        } else {
            "Without a model I can tell you the time, change the brightness and the volume, \
             and set timers, reminders and a pomodoro."
        };
        return plan(vec![], it, reply.into());
    }

    Plan {
        calls: vec![],
        reply: if it { "Non ho capito." } else { "Sorry, I didn't get that." }.into(),
        failure: String::new(),
        understood: false,
    }
}

const UNITS_IT: &[&str] = &["uno", "due", "tre", "quattro", "cinque", "sei", "sette", "otto", "nove"];
const TEENS_IT: &[&str] = &["dieci", "undici", "dodici", "tredici", "quattordici", "quindici", "sedici", "diciassette", "diciotto", "diciannove"];
const TENS_IT: &[(&str, &str, u32)] = &[
    ("venti", "vent", 20), ("trenta", "trent", 30), ("quaranta", "quarant", 40), ("cinquanta", "cinquant", 50),
    ("sessanta", "sessant", 60), ("settanta", "settant", 70), ("ottanta", "ottant", 80), ("novanta", "novant", 90),
];
const UNITS_EN: &[&str] = &["one", "two", "three", "four", "five", "six", "seven", "eight", "nine"];
const TEENS_EN: &[&str] = &["ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen"];
const TENS_EN: &[(&str, u32)] = &[("twenty", 20), ("thirty", 30), ("forty", 40), ("fifty", 50), ("sixty", 60), ("seventy", 70), ("eighty", 80), ("ninety", 90)];

/// One number word, Italian or English, up to 99: "venticinque", "ventuno", "fifteen".
fn number_word(w: &str) -> Option<u32> {
    let pos = |list: &[&str]| list.iter().position(|u| *u == w).map(|i| i as u32);
    if let Some(n) = w.parse::<u32>().ok().or_else(|| pos(UNITS_IT).map(|i| i + 1)).or_else(|| pos(UNITS_EN).map(|i| i + 1)) {
        return Some(n);
    }
    if let Some(i) = pos(TEENS_IT).or_else(|| pos(TEENS_EN)) {
        return Some(10 + i);
    }
    if let Some(&(_, n)) = TENS_EN.iter().find(|(t, _)| *t == w) {
        return Some(n);
    }
    if matches!(w, "un" | "una" | "a" | "an") {
        return Some(1);
    }
    TENS_IT.iter().find_map(|&(full, short, n)| {
        let rest = w.strip_prefix(full).or_else(|| w.strip_prefix(short))?;
        if rest.is_empty() {
            return w.starts_with(full).then_some(n);
        }
        UNITS_IT.iter().position(|u| *u == rest).map(|i| n + i as u32 + 1)
    })
}

/// The number at `w[i]`, and how many words it takes ("twenty five": two).
fn number_at(w: &[String], i: usize) -> Option<(u32, usize)> {
    let n = number_word(w.get(i)?)?;
    if TENS_EN.iter().any(|(_, t)| *t == n)
        && let Some(u) = w.get(i + 1).and_then(|u| UNITS_EN.iter().position(|x| x == u))
    {
        return Some((n + u as u32 + 1, 2));
    }
    Some((n, 1))
}

fn unit_secs(w: &str) -> Option<u64> {
    if w.starts_with("second") || w.starts_with("sec") || w == "s" {
        Some(1)
    } else if w.starts_with("minut") || w == "min" || w == "mins" {
        Some(60)
    } else if matches!(w, "ora" | "ore" | "hour" | "hours" | "h" | "orette" | "oretta") {
        Some(3600)
    } else {
        None
    }
}

/// A length said in words: "10 minuti", "un'ora e mezza", "mezz'ora",
/// "un quarto d'ora", "1 ora e 30 minuti", "two and a half minutes"; and the
/// words it took, from..to.
fn duration(w: &[String]) -> Option<(u64, usize, usize)> {
    let mut total = 0u64;
    let (mut first, mut last) = (None, 0usize);
    let mut i = 0;
    while i < w.len() {
        let (secs, used) = if matches!(w[i].as_str(), "mezz" | "mezzora") {
            (1800, if w[i] == "mezz" && w.get(i + 1).is_some_and(|x| x == "ora") { 2 } else { 1 })
        } else if w[i] == "quarto" && w.get(i + 1).is_some_and(|x| x == "d") && w.get(i + 2).is_some_and(|x| x == "ora") {
            (900, 3)
        } else if w[i] == "half" && w.get(i + 1).is_some_and(|x| x == "an") && w.get(i + 2).is_some_and(|x| x.starts_with("hour")) {
            (1800, 3)
        } else if let Some((n, len)) = number_at(w, i) {
            let mut j = i + len;
            // "two and a half minutes"
            let half_before = w.get(j).is_some_and(|x| x == "and") && w.get(j + 1).is_some_and(|x| x == "a") && w.get(j + 2).is_some_and(|x| x == "half");
            if half_before {
                j += 3;
            }
            match w.get(j).and_then(|u| unit_secs(u)) {
                Some(unit) => {
                    let mut secs = n as u64 * unit + if half_before { unit / 2 } else { 0 };
                    // "un minuto e mezzo", "un'ora e mezza", "an hour and a half"
                    let it_half = w.get(j + 1).is_some_and(|x| x == "e") && w.get(j + 2).is_some_and(|x| x.starts_with("mezz"));
                    let en_half = w.get(j + 1).is_some_and(|x| x == "and") && w.get(j + 2).is_some_and(|x| x == "a") && w.get(j + 3).is_some_and(|x| x == "half");
                    let mut used = j + 1 - i;
                    if it_half {
                        secs += unit / 2;
                        used += 2;
                    } else if en_half {
                        secs += unit / 2;
                        used += 3;
                    }
                    (secs, used)
                }
                None => {
                    i += 1;
                    continue;
                }
            }
        } else {
            i += 1;
            continue;
        };
        total += secs;
        first.get_or_insert(i);
        last = i + used;
        i += used;
    }
    first.map(|f| (total, f, last))
}

/// "alle 17", "alle 17:30", "alle 5 e mezza", "at 5 pm", "at 17.30": the next such time of day.
fn clock_time(w: &[String]) -> Option<NaiveTime> {
    let at = w.iter().position(|x| matches!(x.as_str(), "alle" | "all" | "at" | "ore"))?;
    let (h, used) = number_at(w, at + 1)?;
    let j = at + 1 + used;
    let mut m = 0;
    if let Some((n, _)) = number_at(w, j).filter(|(n, _)| *n < 60) {
        m = n;
    } else if w.get(j).is_some_and(|x| x == "e") {
        if w.get(j + 1).is_some_and(|x| x.starts_with("mezz")) {
            m = 30;
        } else if w.get(j + 1).is_some_and(|x| x == "un") && w.get(j + 2).is_some_and(|x| x == "quarto") {
            m = 15;
        } else if let Some((n, _)) = number_at(w, j + 1).filter(|(n, _)| *n < 60) {
            m = n;
        }
    }
    let afternoon = w.iter().any(|x| matches!(x.as_str(), "pm" | "pomeriggio" | "sera" | "stasera"));
    let h = if afternoon && h < 12 { h + 12 } else { h };
    NaiveTime::from_hms_opt(h, m, 0)
}

/// What a reminder is for: what's left of the sentence once the request and
/// the time are taken out ("ricordami tra 10 minuti di chiamare Marco": "chiamare Marco").
fn reminder_text(text: &str) -> Option<String> {
    let original: Vec<&str> = text.split_whitespace().collect();
    let lowered: Vec<String> = original.iter().map(|o| words(o).join(" ")).collect();
    let drop_words = [
        "ricordami", "ricorda", "ricordarmi", "ricordare", "promemoria", "remind", "reminder", "me", "mi", "un", "a", "imposta", "metti", "set",
        "per", "favore", "please", "alexa",
    ];
    let mut keep = vec![true; original.len()];
    // The time: "tra/fra/in N unit", "alle H[:M]", "at H[:M] pm", "tomorrow"...
    for i in 0..original.len() {
        let l = lowered[i].as_str();
        if matches!(l, "tra" | "fra" | "in" | "entro" | "alle" | "all" | "at") {
            let rest: Vec<String> = lowered[i + 1..].iter().flat_map(|x| x.split(' ').map(str::to_string)).collect();
            let timed = duration(&rest).is_some_and(|(_, from, _)| from == 0) || (matches!(l, "alle" | "all" | "at") && rest.first().and_then(|x| number_word(x)).is_some());
            if timed {
                keep[i] = false;
                let mut j = i + 1;
                while j < original.len() {
                    let x = lowered[j].as_str();
                    let part_of_time = number_word(x.split(' ').next().unwrap_or("")).is_some()
                        || x.split(' ').all(|p| unit_secs(p).is_some() || matches!(p, "e" | "and" | "mezzo" | "mezza" | "mezz" | "quarto" | "d" | "half" | "pm" | "am" | "del" | "di" | "pomeriggio" | "sera" | "mattina" | "ora"));
                    if !part_of_time || x.is_empty() {
                        break;
                    }
                    // "di" belongs to the time only before "pomeriggio"/"sera".
                    if matches!(x, "di" | "del") && !lowered.get(j + 1).is_some_and(|n| matches!(n.as_str(), "pomeriggio" | "sera" | "mattina")) {
                        break;
                    }
                    keep[j] = false;
                    j += 1;
                }
            }
        }
    }
    let mut out: Vec<&str> = Vec::new();
    for (i, o) in original.iter().enumerate() {
        if !keep[i] {
            continue;
        }
        let l = lowered[i].as_str();
        // The request and the connective before the text.
        if out.is_empty() && (drop_words.contains(&l) || matches!(l, "di" | "che" | "to" | "that" | "about" | "of")) {
            continue;
        }
        out.push(o);
    }
    let text = out.join(" ");
    let text = text.trim().trim_end_matches(['.', '!', '?', ',']).trim().trim_start_matches([',', ':']).trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// A timer's name in the request: "timer per la pasta" / "pasta timer".
fn timer_label(w: &[String], it: bool) -> Option<String> {
    const SKIP: &[&str] = &[
        "il", "la", "lo", "le", "i", "gli", "l", "un", "una", "a", "an", "the", "my", "set", "start", "imposta", "metti", "avvia", "fai", "new",
        "nuovo", "alexa", "hey", "ok", "hi", "esp", "please", "another", "altro",
    ];
    let not_label = |x: &str| SKIP.contains(&x) || number_word(x).is_some() || unit_secs(x).is_some() || matches!(x, "e" | "and" | "mezzo" | "mezza" | "di" | "da" | "of" | "for" | "per" | "timer");
    if let Some(p) = w.iter().position(|x| x == "per" || x == "for") {
        let rest: Vec<&String> = w[p + 1..].iter().skip_while(|x| SKIP.contains(&x.as_str())).take_while(|x| !not_label(x)).take(2).collect();
        if !rest.is_empty() {
            return Some(rest.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" "));
        }
    }
    let t = w.iter().position(|x| x == "timer")?;
    // After it in Italian: "il timer della pasta".
    if it {
        let after = w[t + 1..].iter().skip_while(|x| matches!(x.as_str(), "del" | "della" | "dello" | "dei" | "delle" | "di")).next()?;
        return (w[t + 1..].first() != Some(after) && !not_label(after)).then(|| after.clone());
    }
    // Before it in English: "pasta timer".
    let before = w.get(t.checked_sub(1)?)?;
    (!not_label(before)).then(|| before.clone())
}

/// "Aggiungi 5 minuti al timer", "altri 5 minuti", "togli 2 minuti",
/// "sposta il promemoria alle 18", "rimanda di 10 minuti", "cambia il timer
/// a 10 minuti", "5 more minutes", "move the reminder to 6 pm": the
/// `update_timer` arguments, without `which`, and the reply.
fn timer_change(w: &[String], it: bool, reminder: bool) -> Option<(Value, String)> {
    let moved = has(w, &["sposta*", "rimand*", "posticip*", "anticip*", "move", "moved", "postpone*", "push", "delay*", "reschedule*", "snooze*"]);
    let changed = has(w, &["cambia*", "modifica*", "reimposta*", "change", "make", "reset"]);
    // "Aggiungi un timer" is a new one.
    let new_one = has_pair(w, &[("un", "timer"), ("una", "sveglia"), ("a", "timer"), ("another", "timer"), ("new", "timer"), ("nuovo", "timer"), ("altro", "timer")]);
    if new_one {
        return None;
    }
    // "Move it to 6 pm" too.
    let to_at: Vec<String> = w.iter().map(|x| if x == "to" { "at".into() } else { x.clone() }).collect();
    if (moved || changed) && let Some(at) = clock_time(w).or_else(|| clock_time(&to_at)) {
        let at = timers::clock_text(at);
        let reply = match (it, reminder) {
            (true, true) => format!("Promemoria spostato alle {at}."),
            (true, false) => format!("Fatto: il timer finisce alle {at}."),
            (false, true) => format!("Reminder moved to {at}."),
            (false, false) => format!("Done: the timer ends at {at}."),
        };
        return Some((json!({"at": at}), reply));
    }
    // "Five more minutes": the length without the "more".
    let plain: Vec<String> = w.iter().filter(|x| !matches!(x.as_str(), "more" | "extra")).cloned().collect();
    let (secs, from, _) = duration(&plain)?;
    let span = timers::span_words(secs, it);
    let before = from.checked_sub(1).map(|i| plain[i].as_str());
    let less = has(w, &["togli*", "leva*", "riduci*", "accorci*", "anticip*", "meno", "shorten", "subtract", "off", "less", "earlier"]);
    let more = has(w, &["altri", "altre", "ancora", "prolung*", "allung*", "rimand*", "posticip*", "more", "extra", "extend*", "postpone*", "delay*", "snooze*", "push"])
        || (has(w, &["aggiung*", "add"]) && has(w, &["al", "alla", "allo", "ai", "to", "on"]));
    let minutes = secs as f64 / 60.0;
    if moved && matches!(before, Some("tra" | "fra" | "in" | "entro")) {
        let reply = if it { format!("Fatto: tra {span}.") } else { format!("Done: in {span}.") };
        return Some((json!({"in_minutes": minutes}), reply));
    }
    if less {
        let reply = if it { format!("Fatto: {span} in meno.") } else { format!("Done: {span} less.") };
        return Some((json!({"add_minutes": -minutes}), reply));
    }
    if more {
        let reply = if it { format!("Fatto: {span} in più.") } else { format!("Done: {span} more.") };
        return Some((json!({"add_minutes": minutes}), reply));
    }
    if changed && !reminder {
        let reply = if it { format!("Timer reimpostato a {span}.") } else { format!("Timer set to {span}.") };
        return Some((json!({"seconds": secs}), reply));
    }
    None
}

fn is_stop(w: &[String]) -> bool {
    has(w, &["stop", "basta", "ferma", "fermati", "silenzio", "zitto", "zitta", "smettila", "spegni", "ok", "okay", "grazie", "thanks", "enough", "quiet", "dismiss", "shut"])
}

/// "Mancano 4 minuti" / "4 minutes left", for the one ending first.
fn time_left(timers: &[TimerInfo], it: bool) -> String {
    let Some(t) = timers.iter().find(|t| t.state != "ring") else {
        return if it { "Non ci sono timer attivi." } else { "There are no timers running." }.into();
    };
    // Minutes are enough from two minutes up.
    let left = if t.left_s >= 120 { (t.left_s + 30) / 60 * 60 } else { t.left_s };
    let span = timers::span_words(left, it);
    let paused = t.state == "pause";
    match (it, t.label.as_deref(), t.kind) {
        (true, _, timers::Kind::Work) => format!("Mancano {span} alla pausa."),
        (false, _, timers::Kind::Work) => format!("{span} to the break."),
        (true, Some(l), timers::Kind::Reminder) => format!("Mancano {span} al promemoria {l}."),
        (true, Some(l), _) if paused => format!("Il timer {l} è in pausa: mancano {span}."),
        (true, Some(l), _) => format!("Al timer {l} mancano {span}."),
        (true, None, _) if paused => format!("Il timer è in pausa: mancano {span}."),
        (true, None, _) => format!("Mancano {span}."),
        (false, Some(l), _) if paused => format!("The {l} timer is paused with {span} left."),
        (false, Some(l), _) => format!("{span} left on the {l} timer."),
        (false, None, _) => format!("{span} left{}.", if paused { ", paused" } else { "" }),
    }
}

/// Timers, reminders and the pomodoro.
fn timer_plan(text: &str, w: &[String], it: bool, ctx: &Context) -> Option<Plan> {
    let failure = if it { "Non c'è nessun timer così." } else { "There's no timer like that." };
    let with_failure = |mut p: Plan| {
        p.failure = failure.into();
        p
    };
    let cancel = has(w, &["annulla", "cancella", "elimina", "togli", "rimuovi", "cancel", "delete", "remove", "clear"]) || is_stop(w);
    let all = has(w, &["tutti", "tutte", "all", "every"]);
    let said_timer = has(w, &["timer*", "countdown*", "cronometro", "sveglia"]) || has_pair(w, &[("conto", "alla")]);

    // "Stop!" while it rings.
    if ctx.alarm && is_stop(w) && w.len() <= 4 {
        return Some(with_failure(plan(vec![tool("control_timer", json!({"action": "cancel"}))], it, "Ok.".into())));
    }

    if has(w, &["pomodor*"]) {
        if cancel || has(w, &["termina", "interrompi", "finisci", "end", "finish"]) {
            let reply = if it { "Pomodoro fermato." } else { "Pomodoro stopped." };
            return Some(with_failure(plan(vec![tool("pomodoro", json!({"action": "stop"}))], it, reply.into())));
        }
        let work = duration(w).map(|(s, ..)| (s / 60).clamp(1, 120)).unwrap_or(25);
        let reply = if it { format!("Pomodoro avviato: {} di lavoro.", timers::span_words(work * 60, true)) } else { format!("Pomodoro started: {} of work.", timers::span_words(work * 60, false)) };
        return Some(plan(vec![tool("pomodoro", json!({"action": "start", "work_minutes": work}))], it, reply));
    }

    if has(w, &["ricordami", "ricorda", "ricordarmi", "promemoria", "remind*"]) {
        if cancel {
            let which = if all { "all" } else { "reminder" };
            let reply = if it { "Promemoria annullato." } else { "Reminder cancelled." };
            return Some(with_failure(plan(vec![tool("control_timer", json!({"action": "cancel", "which": which}))], it, reply.into())));
        }
        if !has(w, &["ricordami", "ricordarmi", "remind"])
            && let Some((mut args, reply)) = timer_change(w, it, true)
        {
            args["which"] = json!("reminder");
            return Some(with_failure(plan(vec![tool("update_timer", args)], it, reply)));
        }
        let what = reminder_text(text);
        let in_time = duration(w).filter(|&(_, from, _)| from > 0 && matches!(w[from - 1].as_str(), "tra" | "fra" | "in" | "entro"));
        let at = clock_time(w);
        let (args, due) = match (in_time, at) {
            (Some((secs, ..)), _) => (json!({"in_minutes": secs as f64 / 60.0}), Local::now() + chrono::Duration::seconds(secs as i64)),
            (None, Some(t)) => {
                let now = Local::now();
                let today = now.date_naive().and_time(t);
                let due = today.and_local_timezone(Local).earliest().filter(|d| *d > now).unwrap_or_else(|| (today + chrono::Duration::days(1)).and_local_timezone(Local).earliest().unwrap_or(now));
                (json!({"at": timers::clock_text(t)}), due)
            }
            (None, None) => {
                let reply = if it { "Quando te lo devo ricordare? Dimmi per esempio: tra 10 minuti, o alle 17." } else { "When should I remind you? Say for example: in 10 minutes, or at 5 pm." };
                return Some(plan(vec![], it, reply.into()));
            }
        };
        let Some(what) = what else {
            let reply = if it { "Cosa ti devo ricordare?" } else { "What should I remind you of?" };
            return Some(plan(vec![], it, reply.into()));
        };
        let mut args = args;
        args["text"] = json!(what);
        let at = due.format("%H:%M");
        let reply = if it { format!("Va bene, te lo ricordo alle {at}.") } else { format!("OK, I'll remind you at {at}.") };
        return Some(with_failure(plan(vec![tool("set_reminder", args)], it, reply)));
    }

    let how_long = has_pair(w, &[("quanto", "manca"), ("quanto", "tempo"), ("how", "long"), ("how", "much"), ("time", "left")]) || has(w, &["mancano", "manca"]);
    if how_long && (said_timer || !ctx.timers.is_empty()) && !has_pair(w, &[("che", "or")]) {
        return Some(plan(vec![], it, time_left(&ctx.timers, it)));
    }
    // Changing one: by name, or while one rings (or just rang), or while some run.
    if (said_timer || ctx.alarm || !ctx.timers.is_empty())
        && let Some((mut args, reply)) = timer_change(w, it, false)
    {
        if let Some(l) = timer_label(w, it) {
            args["which"] = json!(l);
        }
        return Some(with_failure(plan(vec![tool("update_timer", args)], it, reply)));
    }
    if !said_timer {
        return None;
    }
    let label = timer_label(w, it);
    let which = if all { Some("all".to_string()) } else { label.clone() };
    let which_arg = |action: &str| match &which {
        Some(w) => json!({"action": action, "which": w}),
        None => json!({"action": action}),
    };
    if has(w, &["riprendi", "continua", "riparti", "resume", "unpause", "restart"]) {
        let reply = if it { "Timer ripartito." } else { "Timer resumed." };
        return Some(with_failure(plan(vec![tool("control_timer", which_arg("resume"))], it, reply.into())));
    }
    if has(w, &["pausa", "sospendi", "pause"]) {
        let reply = if it { "Timer in pausa." } else { "Timer paused." };
        return Some(with_failure(plan(vec![tool("control_timer", which_arg("pause"))], it, reply.into())));
    }
    if cancel {
        let reply = match (it, all) {
            (true, true) => "Tutti i timer annullati.",
            (true, false) => "Timer annullato.",
            (false, true) => "All timers cancelled.",
            (false, false) => "Timer cancelled.",
        };
        return Some(with_failure(plan(vec![tool("control_timer", which_arg("cancel"))], it, reply.into())));
    }
    let Some((secs, ..)) = duration(w) else {
        if how_long || has(w, &["quanto", "how"]) {
            return Some(plan(vec![], it, time_left(&ctx.timers, it)));
        }
        let reply = if it { "Di quanto lo imposto? Dimmi per esempio: timer 10 minuti." } else { "How long for? Say for example: timer 10 minutes." };
        return Some(plan(vec![], it, reply.into()));
    };
    let span = timers::span_words(secs, it);
    let reply = match (it, &label) {
        (true, Some(l)) => format!("Timer {l} di {span} avviato."),
        (true, None) => format!("Timer di {span} avviato."),
        (false, Some(l)) => format!("{} timer set for {span}.", capitalize(l)),
        (false, None) => format!("Timer set for {span}."),
    };
    let mut args = json!({"seconds": secs});
    if let Some(l) = &label {
        args["label"] = json!(l);
    }
    Some(plan(vec![tool("set_timer", args)], it, reply))
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// Whether `text` asks the time, the day or the date.
pub fn asks_time(text: &str) -> bool {
    let w = words(text);
    has(&w, &["orario", "clock"])
        || has_pair(&w, &[("che", "or"), ("che", "giorn"), ("che", "data"), ("what", "time"), ("the", "time"), ("what", "day"), ("what", "date"), ("the", "date"), ("e", "oggi")])
}

/// `it` or `en`, whichever `text` looks more like (for text not heard but
/// typed, which Whisper didn't label).
pub fn guess_language(text: &str) -> &'static str {
    const IT: &[&str] = &[
        "il", "lo", "la", "gli", "le", "di", "che", "e", "un", "una", "per", "non", "sono", "ciao", "questo", "come", "sei", "ho", "del",
        "della", "con", "sullo", "sulla", "sui", "al", "alla", "ai", "gradi", "fatto", "ecco", "schermo", "schermi", "faccia", "posso",
        "ti", "mi", "si", "anche", "ancora", "piu", "ora", "adesso", "sinistra", "destra", "sinistro", "destro", "entrambi",
    ];
    const EN: &[&str] = &[
        // Not "a" or "i": Italian has them too.
        "the", "an", "of", "and", "is", "are", "to", "you", "it", "this", "hello", "what", "in", "on", "with", "for", "not", "i'm",
        "done", "screen", "screens", "degrees", "turned", "set", "can", "your", "my", "here", "now", "left", "right", "both", "sorry",
    ];
    let w = words(text);
    let count = |list: &[&str]| w.iter().filter(|x| list.contains(&x.as_str())).count();
    if count(IT) > count(EN) || w.iter().any(|x| x.ends_with("zione") || x.ends_with("mente")) && count(EN) == 0 { "it" } else { "en" }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calls(text: &str, lang: &str) -> Vec<(String, Value)> {
        understand(text, lang, &Context::default()).calls
    }

    #[test]
    fn brightness_and_volume() {
        assert_eq!(calls("Luminosità al 40%", "it"), [tool("set_brightness", json!({"percent": 40}))]);
        assert_eq!(calls("Set the brightness to fifty", "en"), [tool("set_brightness", json!({"percent": 50}))]);
        assert_eq!(calls("Volume al 30", "it"), [tool("set_volume", json!({"percent": 30}))]);
        let ctx = Context { volume: Some(50), ..Default::default() };
        assert_eq!(understand("Turn it up", "en", &ctx).calls, [tool("set_volume", json!({"percent": 70}))]);
        assert_eq!(understand("Parla più piano", "it", &ctx).calls, [tool("set_volume", json!({"percent": 30}))]);
    }

    #[test]
    fn time_questions() {
        for q in ["Che ore sono?", "Che giorno è oggi?", "What time is it?", "What's the date today?", "Tell me the time"] {
            assert!(asks_time(q), "{q}");
        }
        for q in ["Ora metti un timer", "Put a timer on", "Quanto è calda la CPU?"] {
            assert!(!asks_time(q), "{q}");
        }
    }

    #[test]
    fn timers_by_rules() {
        assert_eq!(calls("Alexa, timer 10 minuti", "it"), [tool("set_timer", json!({"seconds": 600}))]);
        assert_eq!(calls("Imposta un timer di un'ora e mezza", "it"), [tool("set_timer", json!({"seconds": 5400}))]);
        assert_eq!(calls("Metti un timer di mezz'ora per la pasta", "it"), [tool("set_timer", json!({"seconds": 1800, "label": "pasta"}))]);
        assert_eq!(calls("Timer di venticinque minuti", "it"), [tool("set_timer", json!({"seconds": 1500}))]);
        assert_eq!(calls("Un timer di un minuto e mezzo", "it"), [tool("set_timer", json!({"seconds": 90}))]);
        assert_eq!(calls("Set a pasta timer for 8 minutes", "en"), [tool("set_timer", json!({"seconds": 480, "label": "pasta"}))]);
        assert_eq!(calls("Set a timer for twenty five minutes", "en"), [tool("set_timer", json!({"seconds": 1500}))]);
        assert_eq!(calls("Start a timer for two and a half minutes", "en"), [tool("set_timer", json!({"seconds": 150}))]);
        assert_eq!(calls("Timer 1 ora e 30 minuti", "it"), [tool("set_timer", json!({"seconds": 5400}))]);
        let p = understand("Timer 10 minuti", "it", &Context::default());
        assert_eq!(p.reply, "Timer di 10 minuti avviato.");
        assert_eq!(calls("Annulla il timer", "it"), [tool("control_timer", json!({"action": "cancel"}))]);
        assert_eq!(calls("Cancel all timers", "en"), [tool("control_timer", json!({"action": "cancel", "which": "all"}))]);
        assert_eq!(calls("Metti in pausa il timer", "it"), [tool("control_timer", json!({"action": "pause"}))]);
        assert_eq!(calls("Riprendi il timer della pasta", "it")[0].1["action"], "resume");
        let ringing = Context { alarm: true, ..Default::default() };
        assert_eq!(understand("Stop!", "en", &ringing).calls, [tool("control_timer", json!({"action": "cancel"}))]);
        assert!(understand("Stop!", "en", &Context::default()).calls.is_empty());
    }

    #[test]
    fn timers_changed_by_rules() {
        assert_eq!(calls("Aggiungi 5 minuti al timer della pasta", "it"), [tool("update_timer", json!({"add_minutes": 5.0, "which": "pasta"}))]);
        assert_eq!(calls("Add 2 minutes to the timer", "en"), [tool("update_timer", json!({"add_minutes": 2.0}))]);
        assert_eq!(calls("Togli 2 minuti al timer", "it"), [tool("update_timer", json!({"add_minutes": -2.0}))]);
        assert_eq!(calls("Take 30 seconds off the tea timer", "en"), [tool("update_timer", json!({"add_minutes": -0.5, "which": "tea"}))]);
        assert_eq!(calls("Cambia il timer a 10 minuti", "it"), [tool("update_timer", json!({"seconds": 600}))]);
        let c = calls("Sposta il promemoria alle 18", "it");
        assert_eq!(c, [tool("update_timer", json!({"at": "18:00", "which": "reminder"}))]);
        let c = calls("Rimanda il promemoria di 10 minuti", "it");
        assert_eq!(c, [tool("update_timer", json!({"add_minutes": 10.0, "which": "reminder"}))]);
        let c = calls("Move the reminder to 6 pm", "en");
        assert_eq!(c, [tool("update_timer", json!({"at": "18:00", "which": "reminder"}))]);
        let c = calls("Sposta il promemoria tra mezz'ora", "it");
        assert_eq!(c, [tool("update_timer", json!({"in_minutes": 30.0, "which": "reminder"}))]);
        // While one rings, or just rang: a snooze.
        let ringing = Context { alarm: true, ..Default::default() };
        assert_eq!(understand("Altri 5 minuti", "it", &ringing).calls, [tool("update_timer", json!({"add_minutes": 5.0}))]);
        assert_eq!(understand("Five more minutes", "en", &ringing).reply, "Done: 5 minutes more.");
        // New ones stay new; "togli il timer" still cancels.
        assert_eq!(calls("Aggiungi un timer di 5 minuti", "it"), [tool("set_timer", json!({"seconds": 300}))]);
        assert_eq!(calls("Set another timer for 5 minutes", "en"), [tool("set_timer", json!({"seconds": 300}))]);
        assert_eq!(calls("Togli il timer", "it"), [tool("control_timer", json!({"action": "cancel"}))]);
        assert_eq!(calls("Ricordami tra 20 minuti di chiamare Marco", "it")[0].0, "set_reminder");
        assert!(understand("Altri 5 minuti", "it", &Context::default()).calls.is_empty());
    }

    #[test]
    fn reminders_and_pomodoro_by_rules() {
        let c = calls("Ricordami tra 20 minuti di chiamare Marco", "it");
        assert_eq!(c, [tool("set_reminder", json!({"in_minutes": 20.0, "text": "chiamare Marco"}))]);
        let c = calls("Ricordami di chiamare Marco tra 20 minuti.", "it");
        assert_eq!(c[0].1["text"], "chiamare Marco");
        let c = calls("Remind me to stretch at 5 pm", "en");
        assert_eq!(c, [tool("set_reminder", json!({"at": "17:00", "text": "stretch"}))]);
        let c = calls("Ricordami alle 17 e 30 di uscire", "it");
        assert_eq!(c, [tool("set_reminder", json!({"at": "17:30", "text": "uscire"}))]);
        assert!(calls("Ricordami di bere", "it").is_empty());
        assert_eq!(calls("Avvia un pomodoro", "it"), [tool("pomodoro", json!({"action": "start", "work_minutes": 25}))]);
        assert_eq!(calls("Start a 50 minute pomodoro", "en"), [tool("pomodoro", json!({"action": "start", "work_minutes": 50}))]);
        assert_eq!(calls("Ferma il pomodoro", "it"), [tool("pomodoro", json!({"action": "stop"}))]);
    }

    #[test]
    fn time_left_on_timers() {
        let t = TimerInfo { id: 1, kind: timers::Kind::Timer, label: Some("pasta".into()), total_s: 600, left_s: 272, state: "run".into(), ends_at: None };
        let ctx = Context { timers: vec![t], ..Default::default() };
        assert_eq!(understand("Quanto manca al timer?", "it", &ctx).reply, "Al timer pasta mancano 5 minuti.");
        assert_eq!(understand("How long is left?", "en", &ctx).reply, "5 minutes left on the pasta timer.");
        assert_eq!(understand("Quanto manca?", "it", &Context::default()).reply, "Non ho capito.");
        assert_eq!(number_word("ventuno"), Some(21));
        assert_eq!(number_word("trentotto"), Some(38));
        assert_eq!(number_word("quarantacinque"), Some(45));
        assert_eq!(number_word("sei"), Some(6));
    }

    #[test]
    fn guesses_the_language_of_typed_text() {
        assert_eq!(guess_language("Ciao, sono il tuo Yoru"), "it");
        assert_eq!(guess_language("Hello, this is your Yoru"), "en");
    }


}
