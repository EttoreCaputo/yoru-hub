//! The app's own tools (docs/agent.md, The agent): what lets the agent read
//! and change Yoru itself: the meetings and their to-dos, the conversations,
//! the settings, and recording.
//! They are found like a service's tools (service "Yoru"), so a request that
//! doesn't need them doesn't pay for them. What they change is told to the
//! app ([`AppChange`]), so its pages show it at once; recording is done by
//! the app, through the same listener.

use serde_json::{Value, json};

use super::{Assistant, ExternalTool, Host, SavedChat, Settings};
use crate::meetings::{self, Meeting, Summary, Transcript};

/// The service the app's tools are found under.
pub const SERVICE: &str = "Yoru";
/// Characters of transcript, or of a conversation, one read answers with.
const READ_CHARS: usize = 15_000;

/// Something an app tool changed, for the app's pages.
#[derive(Debug, Clone)]
pub enum AppChange {
    /// A meeting's transcript reads differently (a speaker renamed).
    Transcript(Meeting),
    /// A meeting has a new summary.
    Summary(Meeting, Summary),
    /// A to-do was ticked off or opened again.
    Todos,
    /// The assistant's settings changed.
    Settings,
    /// Start recording everything played and the default microphone; the
    /// app answers with what it started.
    Record { title: Option<String> },
    /// Stop the recording that runs.
    StopRecording,
}

/// The tools, shaped as a service's.
pub fn tools() -> Vec<ExternalTool> {
    let tool = |name: &str, description: &str, input_schema: Value| ExternalTool {
        name: name.into(),
        service: SERVICE.into(),
        description: description.into(),
        input_schema,
        acts: name == "settings_change",
    };
    let id = json!({"type": "string", "description": "The meeting's id, from meeting_list"});
    let list = json!({"type": "array", "items": {"type": "string"}});
    vec![
        tool(
            "meeting_list",
            "The meetings, calls and voice notes Yoru recorded, newest first: id, title, when, how long, whether they have a transcript and a summary. `query` keeps those whose title has it.",
            json!({"type": "object", "properties": {"query": {"type": "string"}, "limit": {"type": "integer", "description": "At most this many (default 20)"}}}),
        ),
        tool(
            "meeting_read",
            "Read a meeting's transcript, a part at a time: each line numbered, with the time, who spoke and what they said. Call again with `from` to read on.",
            json!({"type": "object", "properties": {"id": id, "from": {"type": "integer", "description": "The line to start at (default 0)"}}, "required": ["id"]}),
        ),
        tool(
            "meeting_summary",
            "A meeting's summary, if it has one: overview, sections, decisions, to-dos and open questions.",
            json!({"type": "object", "properties": {"id": id}, "required": ["id"]}),
        ),
        tool(
            "meeting_rename_speaker",
            "Give someone heard in a meeting their name (\"Person 2 is Marco\"); an empty name gives them their number back.",
            json!({"type": "object", "properties": {
                "id": id,
                "speaker": {"type": "string", "description": "Who, as meeting_read calls them (Person 2) or by their key"},
                "name": {"type": "string"},
            }, "required": ["id", "speaker", "name"]}),
        ),
        tool(
            "meeting_save_summary",
            "Save a meeting's summary, replacing any it had (follow the summarize-meeting skill). A meeting without a title takes the summary's.",
            json!({"type": "object", "properties": {
                "id": id,
                "style": {"type": "string", "enum": meetings::STYLES.iter().map(|(s, _)| *s).collect::<Vec<_>>()},
                "title": {"type": "string"},
                "overview": {"type": "string"},
                "sections": {"type": "array", "items": {"type": "object", "properties": {"title": {"type": "string"}, "items": list}, "required": ["title", "items"]}},
                "decisions": list,
                "actions": {"type": "array", "items": {"type": "object", "properties": {
                    "text": {"type": "string"},
                    "owner": {"type": "string"},
                    "due": {"type": "string"},
                    "date": {"type": "string", "description": "YYYY-MM-DD"},
                    "kind": {"type": "string", "enum": ["task", "reminder", "message", "email"]},
                    "draft": {"type": "string"},
                }, "required": ["text"]}},
                "questions": list,
            }, "required": ["id", "title", "overview", "sections"]}),
        ),
        tool(
            "todo_list",
            "The to-dos from the meetings' summaries: what, who, by when, from which meeting, and whether done.",
            json!({"type": "object", "properties": {"open_only": {"type": "boolean", "description": "Only those not done (default true)"}}}),
        ),
        tool(
            "todo_set_done",
            "Tick a meeting's to-do off, or open it again.",
            json!({"type": "object", "properties": {
                "meeting": {"type": "string", "description": "The meeting's id"},
                "text": {"type": "string", "description": "The to-do's text, exactly as todo_list gives it"},
                "done": {"type": "boolean"},
            }, "required": ["meeting", "text", "done"]}),
        ),
        tool(
            "meeting_record",
            "Start recording a meeting or a call: everything the computer plays and the default microphone. Only when the user asks for it.",
            json!({"type": "object", "properties": {"title": {"type": "string", "description": "Its name, if the user gave one"}}}),
        ),
        tool("meeting_stop", "Stop the recording that runs; it is then transcribed as the settings say.", json!({"type": "object", "properties": {}})),
        tool(
            "conversation_list",
            "The user's conversations with Yoru (typed and spoken), newest first: id, title or first words, when, how many lines. `query` keeps those whose title or first words have it.",
            json!({"type": "object", "properties": {"query": {"type": "string"}, "limit": {"type": "integer", "description": "At most this many (default 20)"}}}),
        ),
        tool(
            "conversation_read",
            "Read a past conversation, a part at a time: each line numbered, with when, who (the user or Yoru) and what was said. Call again with `from` to read on.",
            json!({"type": "object", "properties": {"id": {"type": "string"}, "from": {"type": "integer", "description": "The line to start at (default 0)"}}, "required": ["id"]}),
        ),
        tool(
            "settings_read",
            "Yoru's settings: the assistant's (name, brain and models, effort, web search, the morning recap and quiet hours under routines) and the meetings' (when to transcribe, language, speakers, summary style, meeting detection).",
            json!({"type": "object", "properties": {}}),
        ),
        tool(
            "settings_change",
            "Change some of Yoru's settings, as settings_read shows them: `area` assistant or meetings, `changes` only the fields that change (routines' fields inside \"routines\"). The user is asked to allow it first.",
            json!({"type": "object", "properties": {
                "area": {"type": "string", "enum": ["assistant", "meetings"]},
                "changes": {"type": "object"},
            }, "required": ["area", "changes"]}),
        ),
    ]
}

/// What the user and the others are called without a name, as the app
/// does: by the reply language.
pub(super) fn names(a: &Assistant) -> (&'static str, &'static str) {
    let language = a.reply_language(None).unwrap_or_else(|| crate::intents::system_language().into());
    if language.starts_with("it") { ("Tu", "Persona") } else { ("You", "Person") }
}

/// Run app tool `name` for `a`; `None` when it isn't one. `host` is asked
/// before a setting changes.
pub(super) fn call(a: &Assistant, host: &dyn Host, name: &str, input: &Value) -> Option<Result<String, String>> {
    let (you, person) = names(a);
    let changed = |change: AppChange| {
        let _ = a.app_change(change);
    };
    let changed: &dyn Fn(AppChange) = &changed;
    let id = || input["id"].as_str().unwrap_or_default().to_string();
    let from = input["from"].as_u64().unwrap_or(0) as usize;
    let limit = input["limit"].as_u64().unwrap_or(20) as usize;
    let settings = a.settings();
    // A part fits what the brain takes in one tool result, so Groq's
    // smaller results aren't cut (and the rest lost).
    let part = match settings.brain() {
        Some(brain) if brain != super::Brain::Anthropic => super::compat::budget(brain).result_max.min(READ_CHARS) - 200,
        _ => READ_CHARS,
    };
    let by = settings.brain().map(|b| format!("{} {}", b.name(), settings.model_of(b))).unwrap_or_default();
    let result = match name {
        "meeting_record" => a.app_change(AppChange::Record { title: input["title"].as_str().filter(|t| !t.trim().is_empty()).map(String::from) }),
        "meeting_stop" => a.app_change(AppChange::StopRecording),
        "conversation_list" => Ok(conversations(&a.conversations().list, input["query"].as_str().unwrap_or_default(), limit)),
        "conversation_read" => a.conversation(&id()).ok_or_else(|| "no conversation with that id".to_string()).map(|c| read_chat(&c, from, part)),
        "settings_read" => Ok(json!({"assistant": a.settings(), "meetings": meetings::settings()}).to_string()),
        "settings_change" => change_settings(a, host, input),
        "meeting_list" => Ok(list(&meetings::list(), input["query"].as_str().unwrap_or_default(), limit)),
        "meeting_read" => read_meeting(&id(), from, part, you, person),
        "meeting_summary" => match meetings::summary(&id()) {
            Ok(Some(s)) => Ok(serde_json::to_string(&s).unwrap_or_default()),
            Ok(None) => Ok("it has no summary yet".into()),
            Err(e) => Err(e.to_string()),
        },
        "meeting_rename_speaker" => rename_speaker(&id(), input, you, person, changed),
        "meeting_save_summary" => save_summary(&id(), input, &by, changed),
        "todo_list" => Ok(todos(input["open_only"].as_bool().unwrap_or(true))),
        "todo_set_done" => {
            let (meeting, text, done) = (input["meeting"].as_str().unwrap_or_default(), input["text"].as_str().unwrap_or_default(), input["done"].as_bool().unwrap_or(true));
            meetings::set_todo_done(meeting, text, done).map_err(|e| e.to_string()).map(|_| {
                changed(AppChange::Todos);
                if done { "ticked off".to_string() } else { "open again".to_string() }
            })
        }
        _ => return None,
    };
    Some(result)
}

/// The meetings as lines: newest first, `query` in the title, `limit` at most.
fn list(all: &[Meeting], query: &str, limit: usize) -> String {
    let query = query.trim().to_lowercase();
    let lines: Vec<String> = all
        .iter()
        .filter(|m| query.is_empty() || m.title.as_deref().unwrap_or_default().to_lowercase().contains(&query))
        .take(limit.max(1))
        .map(|m| {
            format!(
                "{}: {} ({}, {} min){}{}",
                m.id,
                m.title.as_deref().unwrap_or("untitled"),
                when(m.started_at),
                m.duration_ms / 60_000,
                if m.transcribed_at.is_some() { ", transcript" } else { ", no transcript" },
                if m.summarized_at.is_some() { ", summary" } else { "" },
            )
        })
        .collect();
    if lines.is_empty() { "no meetings".into() } else { lines.join("\n") }
}

pub(super) fn read_meeting(id: &str, from: usize, part: usize, you: &str, person: &str) -> Result<String, String> {
    let meeting = meetings::get(id).map_err(|e| e.to_string())?;
    let transcript = meetings::transcript(id).map_err(|e| e.to_string())?.ok_or("it has no transcript yet")?;
    Ok(read(&meeting, &transcript, from, part, you, person))
}

/// A part of `transcript` from line `from`: a head with the meeting and its
/// speakers, the numbered lines, and where to read on.
fn read(meeting: &Meeting, transcript: &Transcript, from: usize, part: usize, you: &str, person: &str) -> String {
    let labels = meetings::speaker_labels(&transcript.speakers, you, person);
    let mut out = String::new();
    if from == 0 {
        let people: Vec<String> = transcript.speakers.iter().map(|s| format!("{} ({})", labels[&s.key], s.key)).collect();
        out.push_str(&format!(
            "{} — {}, {} lines. Speakers: {}. \"{you}\" is the user.{}\n\n",
            meeting.title.as_deref().unwrap_or("untitled"),
            when(meeting.started_at),
            transcript.lines.len(),
            people.join(", "),
            if transcript.timed { "" } else { " A simple transcript: paragraphs without times or speakers." }
        ));
    }
    let mut next = None;
    for (i, line) in transcript.lines.iter().enumerate().skip(from) {
        let text = if transcript.timed {
            format!("#{i} [{}] {}: {}\n", clock(line.start_ms), labels.get(&line.who()).map_or("?", String::as_str), line.text.trim())
        } else {
            format!("#{i} {}\n", line.text.trim())
        };
        if out.len() + text.len() > part && i > from {
            next = Some(i);
            break;
        }
        out.push_str(&text);
    }
    match next {
        Some(i) => out.push_str(&format!("\n(more: call meeting_read with from {i})")),
        None => out.push_str("\n(end of the transcript)"),
    }
    out
}

fn rename_speaker(id: &str, input: &Value, you: &str, person: &str, changed: &dyn Fn(AppChange)) -> Result<String, String> {
    let transcript = meetings::transcript(id).map_err(|e| e.to_string())?.ok_or("it has no transcript yet")?;
    let asked = input["speaker"].as_str().unwrap_or_default().trim();
    let labels = meetings::speaker_labels(&transcript.speakers, you, person);
    let key = transcript
        .speakers
        .iter()
        .find(|s| s.key == asked || labels[&s.key].eq_ignore_ascii_case(asked))
        .map(|s| s.key.clone())
        .ok_or_else(|| format!("no speaker {asked:?}; they are: {}", labels.values().cloned().collect::<Vec<_>>().join(", ")))?;
    let name = input["name"].as_str().unwrap_or_default();
    meetings::rename_speaker(id, &key, name).map_err(|e| e.to_string())?;
    if let Ok(meeting) = meetings::get(id) {
        changed(AppChange::Transcript(meeting));
    }
    Ok(format!("{asked} is now {}", if name.trim().is_empty() { "unnamed" } else { name.trim() }))
}

fn save_summary(id: &str, input: &Value, by: &str, changed: &dyn Fn(AppChange)) -> Result<String, String> {
    let mut fields = input.clone();
    if let Some(map) = fields.as_object_mut() {
        map.remove("id");
        map.entry("style").or_insert(json!("auto"));
        map.insert("by".into(), json!(by));
    }
    let mut summary: Summary = serde_json::from_value(fields).map_err(|e| format!("the summary can't be read: {e}"))?;
    meetings::store_summary(id, &mut summary).map_err(|e| e.to_string())?;
    let meeting = meetings::get(id).map_err(|e| e.to_string())?;
    changed(AppChange::Summary(meeting, summary));
    Ok("saved: it shows on the meeting's page".into())
}

/// The conversations as lines: newest first, `query` in the title or first words.
fn conversations(all: &[super::ChatSummary], query: &str, limit: usize) -> String {
    let query = query.trim().to_lowercase();
    let lines: Vec<String> = all
        .iter()
        .filter(|c| query.is_empty() || c.title.to_lowercase().contains(&query) || c.preview.to_lowercase().contains(&query))
        .take(limit.max(1))
        .map(|c| {
            let name = if c.title.is_empty() { format!("«{}»", c.preview) } else { c.title.clone() };
            format!("{}: {name} ({}, {} lines)", c.id, when(c.updated as i64), c.lines)
        })
        .collect();
    if lines.is_empty() { "no conversations".into() } else { lines.join("\n") }
}

/// A part of conversation `chat` from line `from`, and where to read on.
pub(super) fn read_chat(chat: &SavedChat, from: usize, part: usize) -> String {
    let mut out = String::new();
    if from == 0 && !chat.title.text.is_empty() {
        out.push_str(&format!("{}\n\n", chat.title.text));
    }
    let mut next = None;
    for (i, line) in chat.lines.iter().enumerate().skip(from) {
        let who = if line.from == "you" { "user" } else { "Yoru" };
        let text = format!("#{i} [{}] {who}: {}\n", when(line.at as i64), line.text.trim());
        if out.len() + text.len() > part && i > from {
            next = Some(i);
            break;
        }
        out.push_str(&text);
    }
    match next {
        Some(i) => out.push_str(&format!("\n(more: call conversation_read with from {i})")),
        None => out.push_str("\n(end of the conversation)"),
    }
    out
}

/// `changes` laid over `current`, objects merged field by field.
fn merged(current: &Value, changes: &Value) -> Value {
    match (current, changes) {
        (Value::Object(a), Value::Object(b)) => {
            let mut out = a.clone();
            for (k, v) in b {
                let next = out.get(k).map_or_else(|| v.clone(), |old| merged(old, v));
                out.insert(k.clone(), next);
            }
            Value::Object(out)
        }
        (_, new) => new.clone(),
    }
}

/// Change the settings of `area` by `changes`, once the user allows it.
fn change_settings(a: &Assistant, host: &dyn Host, input: &Value) -> Result<String, String> {
    let changes = &input["changes"];
    if !changes.as_object().is_some_and(|c| !c.is_empty()) {
        return Err("changes must name the fields that change".into());
    }
    let area = input["area"].as_str().unwrap_or_default();
    let current = match area {
        "assistant" => json!(a.settings()),
        "meetings" => json!(meetings::settings()),
        _ => return Err("area is assistant or meetings".into()),
    };
    let next = merged(&current, changes);
    // Read back first, so a wrong field or value fails before anyone is asked.
    let check = |e: serde_json::Error| format!("those settings can't be set: {e}");
    let detail = serde_json::to_string_pretty(changes).unwrap_or_default();
    let ask = || match host.confirm(&format!("{SERVICE}: change settings"), &super::summarize(changes), &detail) {
        Some(true) => Ok(()),
        Some(false) => Err("the user said no: don't do it, and don't ask again unless they do".to_string()),
        None => Err("the user didn't answer in time: it wasn't done".to_string()),
    };
    match area {
        "assistant" => {
            let settings: Settings = serde_json::from_value(next).map_err(check)?;
            ask()?;
            a.set_settings(settings).map_err(|e| e.to_string())?;
            let _ = a.app_change(AppChange::Settings);
        }
        _ => {
            let settings: meetings::Settings = serde_json::from_value(next).map_err(check)?;
            ask()?;
            meetings::set_settings(&settings).map_err(|e| e.to_string())?;
        }
    }
    Ok("changed".into())
}

fn todos(open_only: bool) -> String {
    let lines: Vec<String> = meetings::todos()
        .into_iter()
        .filter(|t| !open_only || !t.done)
        .map(|t| {
            let who = if t.item.owner.is_empty() { String::new() } else { format!(" — {}", t.item.owner) };
            let by = if !t.item.date.is_empty() { format!(", by {}", t.item.date) } else if !t.item.due.is_empty() { format!(", by {}", t.item.due) } else { String::new() };
            format!("[{}] {}{who}{by} (meeting {}: {})", if t.done { "x" } else { " " }, t.item.text, t.meeting, t.title)
        })
        .collect();
    if lines.is_empty() { "no to-dos".into() } else { lines.join("\n") }
}

pub(super) fn when(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|t| t.with_timezone(&chrono::Local).format("%a %-d %b %Y %H:%M").to_string()).unwrap_or_default()
}

fn clock(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::{Line, Speaker};

    fn meeting(id: &str, title: Option<&str>) -> Meeting {
        serde_json::from_value(json!({"id": id, "title": title, "started_at": 0, "duration_ms": 1_800_000, "state": "done", "tracks": [], "transcribed_at": 1})).unwrap()
    }

    fn transcript(lines: usize) -> Transcript {
        let line = |i: usize| Line { track: if i % 2 == 0 { "mic".into() } else { "others".into() }, kind: if i % 2 == 0 { "mic".into() } else { "others".into() }, start_ms: i as u64 * 1000, end_ms: 0, text: format!("line {i} {}", "x".repeat(200)), speaker: Some(0) };
        Transcript {
            engine: "test".into(),
            language: None,
            lines: (0..lines).map(line).collect(),
            timed: true,
            speakers: vec![
                Speaker { key: "mic/0".into(), number: 0, name: None },
                Speaker { key: "others/0".into(), number: 1, name: Some("Marco".into()) },
            ],
        }
    }

    #[test]
    fn reads_in_parts_with_names() {
        let (m, t) = (meeting("20261120-100000", Some("Weekly")), transcript(200));
        let first = read(&m, &t, 0, READ_CHARS, "You", "Person");
        assert!(first.starts_with("Weekly — ") && first.contains("Speakers: You (mic/0), Marco (others/0)"));
        assert!(first.contains("#0 [0:00] You: line 0") && first.contains("#1 [0:01] Marco: line 1"));
        let next: usize = first.rsplit("from ").next().unwrap().trim_end_matches(')').parse().unwrap();
        assert!(next > 1 && first.len() <= READ_CHARS + 100);
        let last = read(&m, &t, 190, READ_CHARS, "You", "Person");
        assert!(last.starts_with("#190 ") && last.ends_with("(end of the transcript)"));
    }

    #[test]
    fn lists_and_filters() {
        let all = [meeting("2", Some("Weekly with Marco")), meeting("1", None)];
        let text = list(&all, "", 20);
        assert!(text.contains("2: Weekly with Marco (") && text.contains("30 min), transcript") && text.contains("1: untitled"));
        assert_eq!(list(&all, "marco", 20).lines().count(), 1);
        assert_eq!(list(&all, "nothing", 20), "no meetings");
    }

    #[test]
    fn every_tool_is_called() {
        let a = crate::assistant::tests::assistant();
        let host = crate::assistant::tests::NoBoard;
        for t in tools() {
            assert!(call(&a, &host, &t.name, &json!({})).is_some(), "{} isn't dispatched", t.name);
        }
        assert!(call(&a, &host, "notify", &json!({})).is_none());
    }

    #[test]
    fn settings_change_only_when_allowed_and_valid() {
        let a = crate::assistant::tests::assistant();
        // NoBoard says no to everything.
        let no = call(&a, &crate::assistant::tests::NoBoard, "settings_change", &json!({"area": "assistant", "changes": {"web_search": false}})).unwrap();
        assert!(no.unwrap_err().contains("said no") && a.settings().web_search);
        let bad = call(&a, &crate::assistant::tests::NoBoard, "settings_change", &json!({"area": "assistant", "changes": {"web_search": "maybe"}})).unwrap();
        assert!(bad.unwrap_err().contains("can't be set"));
        struct Yes;
        impl Host for Yes {
            fn confirm(&self, _: &str, _: &str, _: &str) -> Option<bool> {
                Some(true)
            }
        }
        call(&a, &Yes, "settings_change", &json!({"area": "assistant", "changes": {"routines": {"recap_at": "07:45"}}})).unwrap().unwrap();
        let s = a.settings();
        assert_eq!((s.routines.recap_at.as_str(), s.routines.quiet), ("07:45", true), "the other fields stay");
    }

    #[test]
    fn reads_a_conversation_in_parts() {
        let line = |i: usize| super::super::ChatLine { from: if i % 2 == 0 { "you".into() } else { "yoru".into() }, text: format!("line {i} {}", "y".repeat(300)), channel: super::super::Channel::Text, actions: Vec::new(), at: 0 };
        let chat = SavedChat { id: "c1".into(), title: Default::default(), lines: (0..100).map(line).collect() };
        let first = read_chat(&chat, 0, READ_CHARS);
        assert!(first.contains("] user: line 0") && first.contains("] Yoru: line 1") && first.contains("(more: call conversation_read with from "));
        assert!(read_chat(&chat, 99, READ_CHARS).ends_with("(end of the conversation)"));
        let text = conversations(&[chat.summary()], "line 0", 20);
        assert!(text.starts_with("c1: «line 0"));
    }
}
