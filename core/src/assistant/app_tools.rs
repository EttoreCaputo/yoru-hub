//! The app's own tools (docs/agent.md, The agent): what lets the agent read
//! and change Yoru itself, starting with the meetings and their to-dos.
//! They are found like a service's tools (service "Yoru"), so a request that
//! doesn't need them doesn't pay for them. What they change is told to the
//! app ([`AppChange`]), so its pages show it at once.

use serde_json::{Value, json};

use super::ExternalTool;
use crate::meetings::{self, Meeting, Summary, Transcript};

/// The service the app's tools are found under.
pub const SERVICE: &str = "Yoru";
/// Characters of transcript one `meeting_read` answers with.
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
}

/// The tools, shaped as a service's.
pub fn tools() -> Vec<ExternalTool> {
    let tool = |name: &str, description: &str, input_schema: Value| ExternalTool {
        name: name.into(),
        service: SERVICE.into(),
        description: description.into(),
        input_schema,
        acts: false,
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
    ]
}

/// Run app tool `name`; `None` when it isn't one. `you` and `person` are
/// what the user and the others are called without a name ("Tu", "Persona").
/// What changed goes to `changed`.
pub fn call(name: &str, input: &Value, you: &str, person: &str, changed: &dyn Fn(AppChange)) -> Option<Result<String, String>> {
    let id = || input["id"].as_str().unwrap_or_default().to_string();
    let result = match name {
        "meeting_list" => Ok(list(&meetings::list(), input["query"].as_str().unwrap_or_default(), input["limit"].as_u64().unwrap_or(20) as usize)),
        "meeting_read" => read_meeting(&id(), input["from"].as_u64().unwrap_or(0) as usize, you, person),
        "meeting_summary" => match meetings::summary(&id()) {
            Ok(Some(s)) => Ok(serde_json::to_string(&s).unwrap_or_default()),
            Ok(None) => Ok("it has no summary yet".into()),
            Err(e) => Err(e.to_string()),
        },
        "meeting_rename_speaker" => rename_speaker(&id(), input, you, person, changed),
        "meeting_save_summary" => save_summary(&id(), input, changed),
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

fn read_meeting(id: &str, from: usize, you: &str, person: &str) -> Result<String, String> {
    let meeting = meetings::get(id).map_err(|e| e.to_string())?;
    let transcript = meetings::transcript(id).map_err(|e| e.to_string())?.ok_or("it has no transcript yet")?;
    Ok(read(&meeting, &transcript, from, you, person))
}

/// A part of `transcript` from line `from`: a head with the meeting and its
/// speakers, the numbered lines, and where to read on.
fn read(meeting: &Meeting, transcript: &Transcript, from: usize, you: &str, person: &str) -> String {
    let labels = meetings::speaker_labels(&transcript.speakers, you, person);
    let mut out = String::new();
    if from == 0 {
        let people: Vec<String> = transcript.speakers.iter().map(|s| format!("{} ({})", labels[&s.key], s.key)).collect();
        out.push_str(&format!(
            "{} — {}, {} lines. Speakers: {}. \"{you}\" is the user.\n\n",
            meeting.title.as_deref().unwrap_or("untitled"),
            when(meeting.started_at),
            transcript.lines.len(),
            people.join(", ")
        ));
    }
    let mut next = None;
    for (i, line) in transcript.lines.iter().enumerate().skip(from) {
        let text = format!("#{i} [{}] {}: {}\n", clock(line.start_ms), labels.get(&line.who()).map_or("?", String::as_str), line.text.trim());
        if out.len() + text.len() > READ_CHARS && i > from {
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

fn save_summary(id: &str, input: &Value, changed: &dyn Fn(AppChange)) -> Result<String, String> {
    let mut fields = input.clone();
    if let Some(map) = fields.as_object_mut() {
        map.remove("id");
        map.entry("style").or_insert(json!("auto"));
        map.insert("by".into(), json!("Yoru"));
    }
    let mut summary: Summary = serde_json::from_value(fields).map_err(|e| format!("the summary can't be read: {e}"))?;
    meetings::store_summary(id, &mut summary).map_err(|e| e.to_string())?;
    let meeting = meetings::get(id).map_err(|e| e.to_string())?;
    changed(AppChange::Summary(meeting, summary));
    Ok("saved: it shows on the meeting's page".into())
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

fn when(ms: i64) -> String {
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
            speakers: vec![
                Speaker { key: "mic/0".into(), number: 0, name: None },
                Speaker { key: "others/0".into(), number: 1, name: Some("Marco".into()) },
            ],
        }
    }

    #[test]
    fn reads_in_parts_with_names() {
        let (m, t) = (meeting("20261120-100000", Some("Weekly")), transcript(200));
        let first = read(&m, &t, 0, "You", "Person");
        assert!(first.starts_with("Weekly — ") && first.contains("Speakers: You (mic/0), Marco (others/0)"));
        assert!(first.contains("#0 [0:00] You: line 0") && first.contains("#1 [0:01] Marco: line 1"));
        let next: usize = first.rsplit("from ").next().unwrap().trim_end_matches(')').parse().unwrap();
        assert!(next > 1 && first.len() <= READ_CHARS + 100);
        let last = read(&m, &t, 190, "You", "Person");
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
        for t in tools() {
            assert!(call(&t.name, &json!({}), "You", "Person", &|_| {}).is_some(), "{} isn't dispatched", t.name);
        }
        assert!(call("notify", &json!({}), "You", "Person", &|_| {}).is_none());
    }
}
