//! Yoru's own memory test (docs/agent.md, Measuring it): saved conversations
//! with questions about them and what the answers must say, asked to the
//! real brain. Public benchmarks are too noisy to choose by; this one is
//! about Yoru's own life. It uses the brain chosen in the app (its settings
//! and keys); run it by hand when the memory changes:
//!
//! `cargo test --lib eval -- --ignored --nocapture`

use std::sync::Arc;

use super::chats::SavedChat;
use super::tests::NoBoard;
use super::{Assistant, Channel, ChatLine, Timers};

/// A conversation `days` ago: the user's line, then Yoru's.
const CHATS: &[(u64, &str, &str)] = &[
    (10, "My mother Anna's birthday is on March 3, remind me a week before next year.", "Noted: Anna's birthday is March 3."),
    (6, "I had lunch with Marco Bianchi today. He's leaving Acme to join Stripe in January as head of design.", "Good news for Marco."),
    (3, "The Zendata invoice import must be done by December 12. Lia will check the numbers.", "Got it: December 12, Lia checks the numbers."),
    (1, "Change of plans: the Zendata deadline moved to December 19.", "Noted, December 19 now."),
];

/// A question, and the words one of which the answer must say.
const QUESTIONS: &[(&str, &[&str])] = &[
    ("Where is Marco going to work?", &["Stripe"]),
    ("When is the Zendata deadline now?", &["19"]),
    ("Who checks the Zendata numbers?", &["Lia"]),
    ("When is my mother's birthday?", &["March 3", "3 March", "March 3rd"]),
    ("What's my sister's name?", &["don't know", "do not know", "not sure", "haven't", "no record", "didn't", "don't have"]),
];

const DAY_MS: u64 = 24 * 3600 * 1000;

/// `text` asked in a new conversation, waiting out a free tier's limit for the minute.
fn ask(a: &Assistant, text: &str) -> String {
    a.new_conversation();
    for _ in 0..4 {
        match a.respond(text, Some("en"), Channel::Text, &NoBoard) {
            Ok(reply) => return reply.text,
            Err(e) if e.contains("try again") => std::thread::sleep(std::time::Duration::from_secs(30)),
            Err(e) => return format!("failed: {e}"),
        }
    }
    "failed: the brain's limit".into()
}

#[test]
#[ignore = "asks the brain chosen in the app, which costs"]
fn yoru_remembers_his_own_life() {
    let dir = std::env::temp_dir().join(format!("yoru-eval-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // The app's brain, and nothing else of the app's; `EVAL_BRAIN=ollama:qwen3.5:9b` picks another.
    let settings = crate::paths::data_dir().map(|d| d.join("assistant.json")).expect("the app's data folder");
    std::fs::copy(settings, dir.join("assistant.json")).expect("the app's settings: choose a brain first");
    let a = Assistant::open(Some(dir.clone()), Arc::new(Timers::open(None)));
    if let Ok(by) = std::env::var("EVAL_BRAIN") {
        let (brain, model) = by.split_once(':').and_then(|(b, m)| Some((super::Brain::by_id(b)?, m))).expect("EVAL_BRAIN is provider:model");
        let mut settings = super::Settings { provider: Some(brain), ..a.settings() };
        settings.set_model_of(brain, model);
        a.set_settings(settings).unwrap();
    }
    let now = chrono::Utc::now().timestamp_millis() as u64;
    for (i, (days, said, answer)) in CHATS.iter().enumerate() {
        let at = now - days * DAY_MS;
        let line = |from: &str, text: &str, at| ChatLine { from: from.into(), text: text.into(), channel: Channel::Text, actions: Vec::new(), at };
        a.archive.put(SavedChat { id: format!("eval{i}"), lines: vec![line("you", said, at), line("yoru", answer, at + 2000)], ..Default::default() });
    }
    a.sync_history();

    let mut right = 0;
    for (question, any) in QUESTIONS {
        let reply = ask(&a, question);
        let ok = any.iter().any(|w| reply.to_lowercase().contains(&w.to_lowercase()));
        right += ok as usize;
        eprintln!("{} {question}\n   {}", if ok { "ok  " } else { "MISS" }, reply.replace('\n', " "));
    }

    // Writing: what lasts is kept, what must never be is not.
    let said = ask(&a, "Remember that I prefer meetings after 10am, and my IBAN is IT60X0542811101000000123456.");
    eprintln!("     {}", said.replace('\n', " "));
    let me = a.notes.read("me.md", true).unwrap_or_default();
    let kept = me.contains("10");
    let leaked = std::fs::read_dir(dir.join("brain")).is_ok() && a.notes.search("IT60X0542811101000000123456", true).contains("IT60X");
    eprintln!("{} keeps the meeting preference\n{} keeps no IBAN", if kept { "ok  " } else { "MISS" }, if leaked { "MISS" } else { "ok  " });
    right += kept as usize + !leaked as usize;

    let total = QUESTIONS.len() + 2;
    eprintln!("memory: {right} of {total}");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(right * 10 >= total * 8, "{right} of {total}");
}

/// A short meeting, as a simple transcript would hold it.
const MEETING: &[&str] = &[
    "Ciao a tutti, iniziamo. Oggi parliamo del rilascio della nuova app e del budget per il marketing.",
    "Allora, per il rilascio: Giulia ha finito i test sull'iPhone, mancano solo quelli su Android. Marco, riesci entro venerdì?",
    "Sì, entro venerdì li chiudo. Poi mando a Giulia il resoconto via email.",
    "Perfetto. Decidiamo allora di rilasciare lunedì prossimo, se i test passano.",
    "Sul budget: abbiamo diecimila euro. Proposta: seimila per le campagne social e quattromila per gli eventi.",
    "Va bene, approvato. Resta da capire se il fornitore dei video riesce a consegnare entro fine mese: lo chiedo io a Paolo domani.",
    "Ok, ci aggiorniamo giovedì alle dieci. Ciao!",
];

/// `SUMMARY_BY=google:gemini-3.6-flash,ollama:qwen3.5:9b cargo test --lib eval::summarizes -- --ignored --nocapture`:
/// each model summarizes [`MEETING`] the way the app has it done, in a
/// data folder of its own (`HOME` is moved there; the keys come from their
/// environment variables).
#[test]
#[ignore = "asks each model, which may cost"]
fn summarizes_with_each_model() {
    use crate::meetings::{Line, Meeting, State, Track, Transcript};
    let list = std::env::var("SUMMARY_BY").expect("SUMMARY_BY: provider:model,…");
    let home = std::env::temp_dir().join(format!("yoru-summary-{}", std::process::id()));
    let data = home.join("Library/Application Support/yoru-hub");
    let id = "20261124-100000";
    let folder = data.join("meetings").join(id);
    std::fs::create_dir_all(&folder).unwrap();
    // SAFETY: the test runs alone (it's ignored and named on its own).
    unsafe { std::env::set_var("HOME", &home) };
    let track = Track { name: "others".into(), source: "Pocket".into(), kind: "others".into(), file: None, duration_ms: 120_000, rate: 16_000, channels: 1 };
    let started = chrono::Local::now().timestamp_millis();
    let meeting = Meeting { id: id.into(), title: None, started_at: started, duration_ms: 120_000, state: State::Done, tracks: vec![track], transcribed_at: Some(started), summarized_at: None, speakers: Default::default(), origin: None };
    let lines = MEETING.iter().map(|t| Line { track: "others".into(), kind: "others".into(), start_ms: 0, end_ms: 0, text: t.to_string(), speaker: None }).collect();
    let transcript = Transcript { engine: "test".into(), language: Some("it".into()), lines, timed: false, speakers: vec![] };
    std::fs::write(folder.join("transcript.json"), serde_json::to_vec(&transcript).unwrap()).unwrap();
    let mut failed = Vec::new();
    for by in list.split(',').map(str::trim).filter(|b| !b.is_empty()) {
        std::fs::write(folder.join("meeting.json"), serde_json::to_vec(&meeting).unwrap()).unwrap();
        let _ = std::fs::remove_file(folder.join("summary.json"));
        let a = Assistant::open(Some(data.clone()), Arc::new(Timers::open(None)));
        let started = std::time::Instant::now();
        match a.summarize_meeting(id, "auto", Some(by)) {
            Ok(s) => {
                let actions: Vec<String> = s.actions.iter().map(|x| format!("{} ({}, {})", x.text, x.owner, x.due)).collect();
                let good = !s.overview.is_empty() && !s.decisions.is_empty() && s.actions.len() >= 2;
                eprintln!("{} {by} in {:.0?}: {}\n     {}\n     decisions: {:?}\n     actions: {actions:?}", if good { "ok  " } else { "POOR" }, started.elapsed(), s.title, s.overview, s.decisions);
                if !good {
                    failed.push(by.to_string());
                }
            }
            Err(e) => {
                eprintln!("FAIL {by} in {:.0?}: {e}", started.elapsed());
                failed.push(by.to_string());
            }
        }
    }
    let _ = std::fs::remove_dir_all(&home);
    assert!(failed.is_empty(), "{failed:?}");
}
