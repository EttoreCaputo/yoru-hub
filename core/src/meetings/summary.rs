//! A meeting's summary: what was said, decided and left to do, made from
//! its transcript by a language model the caller hands over (the local one
//! by default, or the assistant's brain). A transcript longer than the
//! model reads at once is summarized in parts first, then the parts together.

use std::fs;
use std::io;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{folder, get, save, transcript};

/// Something someone has to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionItem {
    pub text: String,
    /// Who, as the transcript calls them; empty when nobody was named.
    #[serde(default)]
    pub owner: String,
    /// When, as said ("Friday"); empty when no date was said.
    #[serde(default)]
    pub due: String,
    /// The day it is due, `YYYY-MM-DD`, worked out from `due` and the
    /// meeting's day; empty when no day can be told.
    #[serde(default)]
    pub date: String,
    /// What kind of doing: `task`, `reminder` (at a time), `message` or `email`.
    #[serde(default = "task")]
    pub kind: String,
    /// For a message or an email, the text ready to send; empty otherwise.
    #[serde(default)]
    pub draft: String,
}

fn task() -> String {
    "task".into()
}

/// A part of the summary, named for what is in it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub title: String,
    #[serde(default)]
    pub items: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub title: String,
    /// A few sentences on what the meeting was about and how it went.
    pub overview: String,
    /// The body, laid out by the style: sections the model names.
    #[serde(default)]
    pub sections: Vec<Section>,
    /// The main things said, in summaries made before the sections.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<String>,
    #[serde(default)]
    pub decisions: Vec<String>,
    #[serde(default)]
    pub actions: Vec<ActionItem>,
    #[serde(default)]
    pub questions: Vec<String>,
    /// The style it was written in (`STYLES`); empty in older summaries.
    #[serde(default)]
    pub style: String,
    /// The model that wrote it, for people.
    #[serde(default)]
    pub by: String,
    /// Unix milliseconds.
    #[serde(default)]
    pub made_at: i64,
}

impl Summary {
    /// What the memory's sleep reads of it: the gist, the decisions and who does what.
    pub fn for_memory(&self) -> String {
        let mut out = vec![self.overview.trim().to_string()];
        out.extend(self.decisions.iter().map(|d| format!("Decided: {d}")));
        out.extend(self.actions.iter().map(|a| {
            let who = if a.owner.is_empty() { String::new() } else { format!(" ({})", a.owner) };
            let when = if !a.date.is_empty() { &a.date } else { &a.due };
            let due = if when.is_empty() { String::new() } else { format!(", by {when}") };
            format!("To do{who}: {}{due}", a.text)
        }));
        out.join("\n")
    }
}

/// A JSON answer to a prompt, matching a schema: the model's text.
pub type Complete<'a> = dyn Fn(&str, &Value) -> Result<String, String> + 'a;

/// The ways a summary can be laid out, like the note templates of meeting
/// recorders (Pocket's free ones): an id and how the sections go. The rest
/// of the summary (decisions, things to do, open questions) is the same in each.
pub const STYLES: [(&str, &str); 6] = [
    (
        "auto",
        "First tell what this is (a meeting, a call, an interview, a lesson, a talk, a voice note, a chat), then lay out the sections that fit it: two to five, each named for what is in it (never \"Points\" or \"Summary\"), in the order that reads best.",
    ),
    ("meeting", "One section per topic discussed, named for the topic, with what was said on it and by whom."),
    ("call", "A short call note: a section \"Background\" (why the call, who was on it), then one section with what was discussed. Keep each item short."),
    ("interview", "One section per question asked, titled with the question, its items the key points of the answer; then a section \"Impressions\" if any were said."),
    ("lecture", "Study notes: one section per topic of the lesson, in order, with its key ideas, definitions and examples; then a section \"To review\"."),
    ("note", "One person speaking to themselves (a voice note, a dictation): one section per idea, in their words, without the fillers and repeats."),
];

/// How style `id` lays out the sections; an unknown one is `auto`.
fn layout(style: &str) -> (&'static str, &'static str) {
    STYLES.iter().copied().find(|(id, _)| *id == style).unwrap_or(STYLES[0])
}

fn schema() -> Value {
    let list = json!({"type": "array", "items": {"type": "string"}});
    let text = json!({"type": "string"});
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["title", "overview", "sections", "decisions", "actions", "questions"],
        "properties": {
            "title": text,
            "overview": text,
            "sections": {"type": "array", "items": {
                "type": "object",
                "additionalProperties": false,
                "required": ["title", "items"],
                "properties": {"title": text, "items": list},
            }},
            "decisions": list,
            "actions": {"type": "array", "items": {
                "type": "object",
                "additionalProperties": false,
                "required": ["text", "owner", "due", "date", "kind", "draft"],
                "properties": {
                    "text": text,
                    "owner": text,
                    "due": text,
                    "date": text,
                    "kind": {"type": "string", "enum": ["task", "reminder", "message", "email"]},
                    "draft": text,
                },
            }},
            "questions": list,
        },
    })
}

const RULES: &str = "Be faithful: only what was said, no advice and nothing made up; a transcript made by speech recognition has mistakes, so read past them. \
`title`: what it was about, at most eight words (never just \"meeting\" or \"summary\"). \
`overview`: two to four sentences, the gist a reader needs if they read nothing else. \
`sections`: the body, as the layout below says; each item one line, starting with its subject in **bold** when that helps (\"**Pricing:** 39 euro a month\"), with who said it when it matters. \
`decisions`: what was agreed. \
`actions`: what someone has to do, a short imperative each: `owner` as the transcript names them (or empty), `due` as said (or empty), `date` the day it is due as YYYY-MM-DD worked out from `due` and the day it was recorded (or empty), `kind` \"email\" for writing an email, \"message\" for sending a message (chat, WhatsApp, Slack), \"reminder\" for something at a time (a call, an appointment), else \"task\"; `draft`, for an email or a message only, the text ready to send, short, in the speakers' language (else empty). \
`questions`: what was left open. \
Leave a list empty rather than pad it.";

/// Summarize meeting `id` with `complete`, in `style` (`STYLES`), reading at
/// most `chunk` characters of transcript at once. The speakers are called by
/// their names, else `you` or `person` and their number. `by` names the
/// model in the summary. `on_progress` hears how far it got, 0 to 1.
#[allow(clippy::too_many_arguments)]
pub fn summarize(id: &str, style: &str, you: &str, person: &str, by: &str, chunk: usize, complete: &Complete, mut on_progress: impl FnMut(f64)) -> io::Result<Summary> {
    let transcript = transcript(id)?.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no transcript yet"))?;
    let labels = super::transcript::labels(&transcript.speakers, you, person);
    let lines: Vec<String> = transcript
        .lines
        .iter()
        .map(|l| format!("[{}] {}: {}", clock(l.start_ms), labels.get(&l.who()).map_or("?", String::as_str), l.text))
        .collect();
    if lines.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "the transcript is empty"));
    }
    let (style, sections) = layout(style);
    // The day it was recorded turns "Friday" into a date.
    let day = chrono::DateTime::from_timestamp_millis(get(id)?.started_at).map(|t| t.with_timezone(&chrono::Local).format("%A %-d %B %Y").to_string()).unwrap_or_default();
    // Small models answer in English unless told the language by name.
    let you_are = format!(
        "It was recorded on {day}. The person who recorded it is called \"{you}\" in it. Write every field in {}. Layout of `sections`: {sections}",
        language_of(&transcript.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join(" "))
    );
    let parts = split(&lines, chunk);
    let mut summary = if parts.len() == 1 {
        on_progress(0.1);
        ask(complete, &format!("Summarize this meeting transcript. {you_are} {RULES}\n\nTranscript:\n{}", parts[0]))?
    } else {
        // Each part on its own, then the parts' notes into one summary.
        let mut notes = Vec::new();
        for (i, part) in parts.iter().enumerate() {
            on_progress(i as f64 / (parts.len() + 1) as f64);
            let prompt = format!(
                "This is part {} of {} of a meeting transcript. Take notes on this part only, in the fields below; `title` and `overview` are about this part. {you_are} {RULES}\n\nTranscript, part {}:\n{part}",
                i + 1,
                parts.len(),
                i + 1
            );
            notes.push(serde_json::to_string(&ask(complete, &prompt)?)?);
        }
        on_progress(parts.len() as f64 / (parts.len() + 1) as f64);
        let prompt = format!(
            "These are notes on the {} parts of one meeting, in order. Merge them into the summary of the whole meeting: one title, one overview, the sections laid out again for the whole, and each list without repeats. {you_are} {RULES}\n\nNotes:\n{}",
            parts.len(),
            notes.join("\n")
        );
        ask(complete, &prompt)?
    };
    summary.style = style.to_string();
    summary.by = by.to_string();
    summary.made_at = chrono::Local::now().timestamp_millis();
    let folder = folder(id)?;
    fs::write(folder.join("summary.json"), serde_json::to_vec_pretty(&summary)?)?;
    let mut meeting = get(id)?;
    meeting.summarized_at = Some(summary.made_at);
    // A meeting the user didn't name takes the summary's title.
    if meeting.title.is_none() && !summary.title.trim().is_empty() {
        meeting.title = Some(summary.title.trim().to_string());
    }
    save(&folder, &meeting)?;
    on_progress(1.0);
    Ok(summary)
}

/// The language a text is in, by name, from its most common short words:
/// the languages Parakeet hears that a meeting is likely in.
fn language_of(text: &str) -> &'static str {
    const WORDS: [(&str, &[&str]); 6] = [
        ("Italian", &["il", "che", "di", "non", "è", "per", "una", "sono", "anche", "questo", "perché", "della"]),
        ("English", &["the", "and", "is", "that", "of", "to", "it", "you", "this", "with", "for", "are"]),
        ("Spanish", &["el", "que", "de", "los", "es", "por", "una", "pero", "para", "esto", "también", "las"]),
        ("French", &["le", "les", "est", "que", "des", "une", "pas", "pour", "dans", "c'est", "avec", "mais"]),
        ("German", &["der", "die", "und", "ist", "das", "nicht", "ich", "mit", "ein", "auch", "sie", "wir"]),
        ("Portuguese", &["o", "que", "não", "uma", "os", "para", "com", "isso", "você", "mas", "também", "das"]),
    ];
    let words: Vec<String> = text.split_whitespace().map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'').to_lowercase()).collect();
    WORDS.iter().max_by_key(|(_, common)| words.iter().filter(|w| common.contains(&w.as_str())).count()).map_or("the transcript's language", |(name, _)| name)
}

fn ask(complete: &Complete, prompt: &str) -> io::Result<Summary> {
    let text = complete(prompt, &schema()).map_err(io::Error::other)?;
    parse(&text)
}

/// The model's JSON, forgiving a code fence or words around it.
fn parse(text: &str) -> io::Result<Summary> {
    let start = text.find('{').ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "the model gave no JSON"))?;
    let end = text.rfind('}').ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "the model gave no JSON"))?;
    serde_json::from_str(&text[start..=end]).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("the model's summary can't be read: {e}")))
}

/// The lines in parts of at most `chunk` characters, never splitting a line.
fn split(lines: &[String], chunk: usize) -> Vec<String> {
    let mut parts = vec![String::new()];
    for line in lines {
        let last = parts.last_mut().unwrap();
        if !last.is_empty() && last.len() + line.len() + 1 > chunk {
            parts.push(String::new());
        }
        let last = parts.last_mut().unwrap();
        last.push_str(line);
        last.push('\n');
    }
    parts
}

fn clock(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// The summary of meeting `id`, if it has one.
pub fn summary(id: &str) -> io::Result<Option<Summary>> {
    match fs::read(folder(id)?.join("summary.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_at_lines() {
        let lines: Vec<String> = ["aaaa", "bbbb", "cccc"].map(String::from).to_vec();
        assert_eq!(split(&lines, 10), ["aaaa\nbbbb\n", "cccc\n"]);
        assert_eq!(split(&lines, 1000).len(), 1);
        // A line longer than a part still goes in, alone.
        assert_eq!(split(&["x".repeat(20)], 10).len(), 1);
    }

    #[test]
    fn tells_the_language() {
        assert_eq!(language_of("Allora, questo è un problema perché non sono sicuro che il firmware sia pronto"), "Italian");
        assert_eq!(language_of("So the firmware is not ready and that is the problem with this release"), "English");
    }

    #[test]
    fn reads_a_fenced_answer() {
        let text = "```json\n{\"title\": \"Weekly\", \"overview\": \"o\", \"sections\": [{\"title\": \"Pricing\", \"items\": [\"**Monthly:** 39\"]}], \"decisions\": [\"ship\"], \"actions\": [{\"text\": \"t\", \"owner\": \"Marco\", \"due\": \"Friday\", \"date\": \"2026-11-20\", \"kind\": \"email\", \"draft\": \"Ciao\"}], \"questions\": []}\n```";
        let s = parse(text).unwrap();
        assert_eq!((s.title.as_str(), s.sections[0].title.as_str(), s.decisions.len()), ("Weekly", "Pricing", 1));
        assert_eq!((s.actions[0].owner.as_str(), s.actions[0].date.as_str(), s.actions[0].kind.as_str()), ("Marco", "2026-11-20", "email"));
    }

    #[test]
    fn reads_an_older_summary() {
        // Before the sections: points, and to-dos without a date or a kind.
        let text = "{\"title\": \"Weekly\", \"overview\": \"o\", \"points\": [\"p\"], \"decisions\": [], \"actions\": [{\"text\": \"t\", \"owner\": \"\", \"due\": \"\"}], \"questions\": []}";
        let s = parse(text).unwrap();
        assert_eq!((s.points.len(), s.sections.len(), s.actions[0].kind.as_str(), s.style.as_str()), (1, 0, "task", ""));
    }

    #[test]
    fn falls_back_to_auto() {
        assert_eq!(layout("interview").0, "interview");
        assert_eq!(layout("nonsense").0, "auto");
        assert_eq!(layout("").0, "auto");
    }
}
