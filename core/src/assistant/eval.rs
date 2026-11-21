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
    // The app's brain, and nothing else of the app's.
    let settings = crate::paths::data_dir().map(|d| d.join("assistant.json")).expect("the app's data folder");
    std::fs::copy(settings, dir.join("assistant.json")).expect("the app's settings: choose a brain first");
    let a = Assistant::open(Some(dir.clone()), Arc::new(Timers::open(None)));
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
