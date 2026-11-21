//! A meeting's summary: what was said, decided and left to do. The agent
//! writes it from the transcript with the app's tools, following the
//! `summarize-meeting` skill (docs/agent.md); this keeps and reads it.

use std::fs;
use std::io;

use serde::{Deserialize, Serialize};

use super::{folder, get, save};

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

/// Keep `summary` as meeting `id`'s, made now; a meeting the user didn't
/// name takes its title. Whoever wrote it (the summarizer, the agent) goes
/// through here.
pub fn store(id: &str, summary: &mut Summary) -> io::Result<()> {
    summary.made_at = chrono::Local::now().timestamp_millis();
    let folder = folder(id)?;
    fs::write(folder.join("summary.json"), serde_json::to_vec_pretty(&summary)?)?;
    let mut meeting = get(id)?;
    meeting.summarized_at = Some(summary.made_at);
    // A meeting the user didn't name takes the summary's title.
    if meeting.title.is_none() && !summary.title.trim().is_empty() {
        meeting.title = Some(summary.title.trim().to_string());
    }
    save(&folder, &meeting)
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
    fn reads_a_summary() {
        let text = "{\"title\": \"Weekly\", \"overview\": \"o\", \"sections\": [{\"title\": \"Pricing\", \"items\": [\"**Monthly:** 39\"]}], \"decisions\": [\"ship\"], \"actions\": [{\"text\": \"t\", \"owner\": \"Marco\", \"due\": \"Friday\", \"date\": \"2026-11-20\", \"kind\": \"email\", \"draft\": \"Ciao\"}], \"questions\": []}";
        let s: Summary = serde_json::from_str(text).unwrap();
        assert_eq!((s.title.as_str(), s.sections[0].title.as_str(), s.decisions.len()), ("Weekly", "Pricing", 1));
        assert_eq!((s.actions[0].owner.as_str(), s.actions[0].date.as_str(), s.actions[0].kind.as_str()), ("Marco", "2026-11-20", "email"));
    }

    #[test]
    fn reads_an_older_summary() {
        // Before the sections: points, and to-dos without a date or a kind.
        let text = "{\"title\": \"Weekly\", \"overview\": \"o\", \"points\": [\"p\"], \"decisions\": [], \"actions\": [{\"text\": \"t\", \"owner\": \"\", \"due\": \"\"}], \"questions\": []}";
        let s: Summary = serde_json::from_str(text).unwrap();
        assert_eq!((s.points.len(), s.sections.len(), s.actions[0].kind.as_str(), s.style.as_str()), (1, 0, "task", ""));
    }
}
