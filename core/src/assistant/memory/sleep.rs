//! The memory's consolidation, called the sleep in the code (docs/memory.md,
//! Learning): after a conversation, a meeting's summary, and in the nightly
//! memory tidy, the brain rereads what happened, knowing
//! who said what, and returns the people, relations and facts worth keeping,
//! which are merged with what the graph already knows. Always the user's
//! brain: one model, never a second one for the memory.
//!
//! What it keeps is curated, not just collected: a fact must quote the
//! user's own words it rests on (else it is dropped as made up), one already
//! known in other words is not added again, and the nightly tidy prunes what
//! should never have been kept and rewrites the summaries of people.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use serde_json::{Value, json};

use super::{Episode, Memory, SECTIONS, fact_id, kin};

/// How the sleep asks the brain: a prompt and a JSON schema in, the JSON out
/// ([`crate::assistant::Assistant::complete_json`]).
pub type Ask<'a> = &'a dyn Fn(&str, &Value) -> Result<String, String>;

/// The longest episode read at once; a longer one is read from its end.
const EPISODE_MAX: usize = 24_000;
/// Facts the nightly tidy rereads at once.
const TIDY_MAX: usize = 300;
/// A conversation where the user said fewer words has nothing to keep.
const MIN_WORDS: usize = 4;
/// The share of a fact's evidence words the user must have said.
const GROUNDED: f64 = 0.8;

#[derive(Debug, Deserialize)]
struct Learnt {
    #[serde(default)]
    entities: Vec<NewEntity>,
    #[serde(default)]
    relations: Vec<NewRelation>,
    #[serde(default)]
    facts: Vec<NewFact>,
    #[serde(default)]
    close: Vec<String>,
    #[serde(default)]
    merge: Vec<Merge>,
}

#[derive(Debug, Deserialize)]
struct NewEntity {
    #[serde(rename = "ref")]
    reference: String,
    name: String,
    kind: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    gender: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    placeholder: bool,
}

#[derive(Debug, Deserialize)]
struct NewRelation {
    a: String,
    kind: String,
    b: String,
}

#[derive(Debug, Deserialize)]
struct NewFact {
    about: String,
    section: String,
    text: String,
    #[serde(default)]
    importance: f64,
    #[serde(default)]
    private: bool,
    #[serde(default)]
    replaces: Vec<String>,
    /// The user's own words the fact rests on.
    #[serde(default)]
    evidence: String,
}

#[derive(Debug, Deserialize)]
struct Merge {
    keep: String,
    drop: String,
}

fn schema() -> Value {
    let sections: Vec<&str> = SECTIONS.iter().map(|(k, _)| *k).collect();
    let refs = "e12 for an entity the memory knows, new1, new2… for one in `entities`, or user";
    json!({
        "type": "object",
        "properties": {
            "entities": {"type": "array", "items": {
                "type": "object",
                "properties": {
                    "ref": {"type": "string", "description": "e12 to update a known one, or new1, new2… for a new one"},
                    "name": {"type": "string", "description": "Its name; for someone not named yet, what they are to the user (mother-in-law)"},
                    "kind": {"type": "string", "enum": ["person", "place", "project", "organisation", "app", "topic", "thing"]},
                    "aliases": {"type": "array", "items": {"type": "string"}},
                    "gender": {"type": "string", "enum": ["f", "m", "unknown"]},
                    "summary": {"type": "string", "description": "Who or what it is, in a few words; empty to keep the one known"},
                    "placeholder": {"type": "boolean", "description": "Someone whose name isn't known yet"},
                },
                "required": ["ref", "name", "kind", "aliases", "gender", "summary", "placeholder"],
                "additionalProperties": false,
            }},
            "relations": {"type": "array", "items": {
                "type": "object",
                "properties": {
                    "a": {"type": "string", "description": refs},
                    "kind": {"type": "string", "enum": kin::KINDS, "description": "a is b's parent, child, partner, sibling, friend or colleague"},
                    "b": {"type": "string", "description": refs},
                },
                "required": ["a", "kind", "b"],
                "additionalProperties": false,
            }},
            "facts": {"type": "array", "items": {
                "type": "object",
                "properties": {
                    "about": {"type": "string", "description": refs},
                    "section": {"type": "string", "enum": sections},
                    "text": {"type": "string", "description": "One short sentence that stands on its own, with names"},
                    "importance": {"type": "number", "description": "0 to 1: 0.9 for what Yoru must never forget"},
                    "private": {"type": "boolean", "description": "Health, money, secrets, codes: kept off cloud models"},
                    "replaces": {"type": "array", "items": {"type": "string"}, "description": "Ids (f12) of known facts this one makes untrue or merges"},
                    "evidence": {"type": "string", "description": "The user's own words this fact rests on, copied exactly from the episode; empty in the nightly tidy"},
                },
                "required": ["about", "section", "text", "importance", "private", "replaces", "evidence"],
                "additionalProperties": false,
            }},
            "close": {"type": "array", "items": {"type": "string"}, "description": "Ids of known facts that are no longer true"},
            "merge": {"type": "array", "items": {
                "type": "object",
                "properties": {"keep": {"type": "string"}, "drop": {"type": "string"}},
                "required": ["keep", "drop"],
                "additionalProperties": false,
            }},
        },
        "required": ["entities", "relations", "facts", "close", "merge"],
        "additionalProperties": false,
    })
}

const RULES: &str = "Worth keeping is what will still help you serve the user in a month: \
who the people in their life are and how they are related; what they like, dislike and prefer, how they want you to talk to them included; \
their work, projects and goals; habits and routines; plans and appointments with a date; decisions taken; what they want to hear about or not.\n\
Keep only what the user said or plainly confirmed. Never keep: small talk and greetings; requests already carried out \
(a timer, a reminder, the weather, a search, a message sent, a task handed to an agent); what you said or what you can do; \
how the user talks to you (by voice, on the board, in the app); words the transcription got wrong or that make no sense; \
feelings guessed from a single remark. When unsure, leave it out: a missing fact costs less than a wrong one.\n\
Each fact is one atomic sentence that stands on its own: one piece of knowledge, names in place of pronouns \
(\"Sofia likes tulips\", not \"she likes them\"), the date in place of words like today or next Monday (the episode has its date), \
in the language the known facts are in, else the user's.\n\
`evidence`: the user's own words the fact rests on, copied exactly from the episode (a few words). A fact without them is dropped.\n\
`importance`: 0.9 for what must never be forgotten (who the user is, their family, health limits, what they asked you to remember), \
0.7 for lasting preferences and active projects, 0.5 for useful details, 0.3 for minor ones.\n\
Against what you already know: a fact already known, even in other words, is left out. A fact that updates or corrects a known one \
puts the known one's id in `replaces`. A known fact the episode shows is no longer true: its id in `close`. \
A person known under another name, or a placeholder (name not known yet) that is now someone named: `merge` them, keeping the named one.\n\
Sections: about (who the user is), matters (people, projects, topics that matter to them), tell (what they want to hear about), \
quiet (what they don't want to hear about), notes (anything else).\n\
Nothing worth keeping is a fine answer: return empty lists.";

/// The people and things the memory knows, one a line.
fn people(memory: &Memory) -> String {
    let entities = memory.entities();
    let people: Vec<String> = entities
        .iter()
        .map(|e| {
            let mut l = format!("- [e{}] {}", e.id, if e.is_user { format!("{} (the user)", e.name) } else { e.name.clone() });
            if let Some(label) = &e.label {
                l.push_str(&format!(", {label}"));
            }
            if e.placeholder {
                l.push_str(", name not known yet");
            }
            if !e.aliases.is_empty() {
                l.push_str(&format!(", also {}", e.aliases.join(", ")));
            }
            if !e.summary.is_empty() {
                l.push_str(&format!(": {}", e.summary));
            }
            l
        })
        .collect();
    people.join("\n")
}

/// What the memory knows that the episode may touch: the people, the core,
/// and the facts a search finds.
fn known(memory: &Memory, text: &str) -> String {
    let core = memory.core(4000, false);
    let found = memory.recalled(text, None, 6000, false);
    format!("<people>\n{}\n</people>\n<core>\n{core}\n</core>\n<related>\n{found}\n</related>", people(memory))
}

/// Text reduced to its words, lowercase, one space apart: what evidence is
/// compared on, so punctuation and case don't matter.
fn words(text: &str) -> String {
    text.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What the user said in `episode`: its lines but the assistant's (a
/// meeting's summary is all the user's).
fn said_by_user(episode: &Episode, name: &str) -> String {
    if episode.source != "conversation" {
        return words(&episode.text);
    }
    let mine = [format!("{name}:"), "Yoru:".to_string()];
    let lines: Vec<&str> = episode.text.lines().filter(|l| !mine.iter().any(|m| l.trim_start().starts_with(m.as_str()))).collect();
    words(&lines.join("\n"))
}

/// `evidence` was said: its words, nearly all of them, are in `said`.
fn grounded(evidence: &str, said: &str) -> bool {
    let evidence = words(evidence);
    if evidence.is_empty() {
        return false;
    }
    if said.contains(&evidence) {
        return true;
    }
    let heard: HashSet<&str> = said.split(' ').collect();
    let all: Vec<&str> = evidence.split(' ').collect();
    let found = all.iter().filter(|w| heard.contains(*w)).count();
    found as f64 / all.len() as f64 >= GROUNDED
}

/// The JSON object in a model's answer, without fences or words around it.
fn object_in(answer: &str) -> &str {
    match (answer.find('{'), answer.rfind('}')) {
        (Some(a), Some(b)) if b > a => &answer[a..=b],
        _ => answer,
    }
}

/// Read `episode` and keep what it taught. What changed, in a line.
pub fn consolidate(memory: &Memory, episode: &Episode, name: &str, ask: Ask) -> Result<String, String> {
    let said = said_by_user(episode, name);
    if said.split(' ').filter(|w| !w.is_empty()).count() < MIN_WORDS {
        memory.mark_consolidated(episode.id);
        return Ok("nothing to learn (too short)".into());
    }
    let text: String = {
        let n = episode.text.chars().count();
        episode.text.chars().skip(n.saturating_sub(EPISODE_MAX)).collect()
    };
    let what = match episode.source.as_str() {
        "meeting" => "a meeting the user recorded, as its summary: keep its decisions and the things to do, with who will do them",
        _ => "a conversation between the user and you, transcribed from speech; each line says who spoke",
    };
    let prompt = format!(
        "You are {name}, the user's assistant, consolidating your memory: reread what happened and keep in your long-term memory what is worth it.\n\n\
         What you already know:\n{}\n\n<episode source=\"{}\" date=\"{}\">\n{text}\n</episode>\n\nThe episode is {what}.\n\n{RULES}",
        known(memory, &text),
        episode.source,
        super::day(episode.started),
    );
    let answer = ask(&prompt, &schema())?;
    let learnt: Learnt = serde_json::from_str(object_in(&answer)).map_err(|e| format!("the sleep's answer: {e}"))?;
    let summary = apply(memory, learnt, Some((episode.id, &said)), &HashSet::new());
    memory.mark_consolidated(episode.id);
    Ok(summary)
}

/// The open facts the user gave on purpose (told Yoru to remember, or
/// wrote on the Memory page): the tidy closes them only for a newer one.
fn told(memory: &Memory) -> HashSet<i64> {
    memory.with_db(|db| {
        let Ok(mut stmt) = db.prepare(
            "SELECT f.id FROM facts f JOIN episodes e ON e.id = f.episode WHERE f.valid_to IS NULL AND e.source IN ('remember', 'page')",
        ) else {
            return HashSet::new();
        };
        stmt.query_map([], |r| r.get(0)).map(|rows| rows.flatten().collect()).unwrap_or_default()
    })
}

/// The nightly memory tidy, the memory's own curation: the whole memory
/// reread at once, for duplicates, contradictions, what should never have
/// been kept, placeholders that are now someone, and summaries rewritten
/// from what is known.
pub fn tidy(memory: &Memory, name: &str, ask: Ask) -> Result<String, String> {
    let graph = memory.graph();
    let told = told(memory);
    let open: Vec<String> = graph
        .facts
        .iter()
        .filter(|f| f.to.is_none())
        .take(TIDY_MAX)
        .map(|f| {
            let mark = if told.contains(&f.id) { ", told" } else { "" };
            format!("- [f{}] ({}, about e{}, importance {:.1}{mark}) {}", f.id, f.section, f.about.unwrap_or_else(|| memory.user()), f.importance, f.text)
        })
        .collect();
    if open.is_empty() {
        return Ok("nothing to tidy".into());
    }
    let prompt = format!(
        "You are {name}, the user's assistant, doing the nightly memory tidy: your long-term memory should be short, true and useful. Go through it:\n\
         - Duplicates: write one fact with the best words, its `replaces` naming them all.\n\
         - Contradictions: close the older fact.\n\
         - Facts the rules below would never keep (small talk, requests already done, things about you or how the user talks to you, \
         transcription noise): close them. A fact marked told is one the user gave on purpose: close it only for a newer one that replaces it.\n\
         - People: merge the same person (a placeholder and someone named), fill in a missing gender, and write the summary of each person \
         or thing afresh from their facts and relations: a few words on who or what they are to the user.\n\
         Add no new knowledge: every fact you write rewords or joins the facts listed here, with `evidence` empty. Nothing to do: return empty lists.\n\n\
         <people>\n{}\n</people>\n\n<facts>\n{}\n</facts>\n\n{RULES}",
        people(memory),
        open.join("\n"),
    );
    let answer = ask(&prompt, &schema())?;
    let learnt: Learnt = serde_json::from_str(object_in(&answer)).map_err(|e| format!("the tidy's answer: {e}"))?;
    Ok(apply(memory, learnt, None, &told))
}

/// Put what was learnt into the graph. What changed, in a line.
/// `episode`: the episode read and what the user said in it, which every
/// new fact must quote; `None` in the tidy, which only rewords. `told`: the
/// facts a `close` leaves alone.
fn apply(memory: &Memory, learnt: Learnt, episode: Option<(i64, &str)>, told: &HashSet<i64>) -> String {
    let (episode, said) = (episode.map(|e| e.0), episode.map(|e| e.1));
    let mut refs: HashMap<String, i64> = HashMap::new();
    let user = memory.user();
    refs.insert("user".into(), user);
    let resolve = |refs: &HashMap<String, i64>, r: &str| -> Option<i64> {
        let r = r.trim().trim_start_matches('[').trim_end_matches(']');
        if let Some(id) = refs.get(r) {
            return Some(*id);
        }
        let id: i64 = r.strip_prefix('e')?.parse().ok()?;
        memory.entity(id).map(|e| e.id)
    };
    let (mut new_entities, mut new_facts, mut closed, mut merged, mut unfounded) = (0, 0, 0, 0, 0);
    for e in &learnt.entities {
        let id = match resolve(&refs, &e.reference) {
            Some(id) => id,
            None => {
                // A new one, unless it's known by that name already.
                let Some(id) = memory.find(&e.name).or_else(|| memory.resolve(&e.name, Some(&e.kind))) else { continue };
                new_entities += 1;
                id
            }
        };
        refs.insert(e.reference.trim().to_string(), id);
        if let Some(mut known) = memory.entity(id) {
            if !known.is_user && !e.name.trim().is_empty() && known.placeholder && !e.placeholder {
                known.aliases.push(known.name.clone());
                known.name = e.name.trim().to_string();
                known.placeholder = false;
            }
            for a in &e.aliases {
                if !a.trim().is_empty() && !known.aliases.iter().any(|k| k.eq_ignore_ascii_case(a)) && !a.eq_ignore_ascii_case(&known.name) {
                    known.aliases.push(a.trim().to_string());
                }
            }
            if matches!(e.gender.as_str(), "f" | "m") {
                known.gender = Some(e.gender.clone());
            }
            if !e.summary.trim().is_empty() {
                known.summary = e.summary.trim().to_string();
            }
            if !known.is_user && known.kind != e.kind {
                known.kind = e.kind.clone();
            }
            let _ = memory.put_entity(&known);
        }
    }
    for m in &learnt.merge {
        if let (Some(keep), Some(drop)) = (resolve(&refs, &m.keep), resolve(&refs, &m.drop))
            && memory.merge(keep, drop).is_ok()
        {
            merged += 1;
            refs.insert(m.drop.trim().to_string(), keep);
        }
    }
    for r in &learnt.relations {
        if let (Some(a), Some(b)) = (resolve(&refs, &r.a), resolve(&refs, &r.b)) {
            let _ = memory.relate(a, &r.kind, b, episode);
        }
    }
    for f in &learnt.facts {
        if f.text.trim().is_empty() {
            continue;
        }
        // Not in the user's words: made up, or Yoru's own.
        if said.is_some_and(|said| !grounded(&f.evidence, said)) {
            unfounded += 1;
            continue;
        }
        let replaced: Vec<i64> = f.replaces.iter().filter_map(|r| fact_id(r)).collect();
        // Said already, in these words or others, and replacing nothing: nothing new.
        if replaced.is_empty() && (!memory.matching(f.text.trim()).is_empty() || memory.same_as(&f.text).is_some()) {
            continue;
        }
        let about = resolve(&refs, &f.about).unwrap_or(user);
        let importance = if f.importance > 0.0 { f.importance } else { 0.5 };
        for id in &replaced {
            memory.close(*id);
            closed += 1;
        }
        if memory.insert_fact(about, &f.section, &f.text, importance, f.private, episode).is_ok() {
            new_facts += 1;
        }
    }
    for id in learnt.close.iter().filter_map(|r| fact_id(r)).filter(|id| !told.contains(id)) {
        memory.close(id);
        closed += 1;
    }
    let mut summary = format!("{new_facts} fact(s) learnt, {closed} closed, {new_entities} new, {merged} merged");
    if unfounded > 0 {
        summary.push_str(&format!(", {unfounded} dropped for not being what the user said"));
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_conversation_becomes_people_and_facts() {
        let memory = Memory::open(None);
        let old = memory.remember("Ettore works from home", None, "about", false, false, None).unwrap();
        let old_id = fact_id(old.trim_start_matches("remembered as ")).unwrap();
        memory.log_turn("Ettore", "mia moglie Sofia adora i tulipani, e da lunedì lavoro in ufficio", "Bello!");
        memory.end_episode();
        let episode = memory.pending_episodes().remove(0);
        let answer = json!({
            "entities": [{"ref": "new1", "name": "Sofia", "kind": "person", "aliases": [], "gender": "f", "summary": "the user's wife", "placeholder": false}],
            "relations": [{"a": "new1", "kind": "partner", "b": "user"}],
            "facts": [
                {"about": "new1", "section": "matters", "text": "Sofia adora i tulipani", "importance": 0.6, "private": false, "replaces": [], "evidence": "Sofia adora i tulipani"},
                {"about": "user", "section": "about", "text": "Ettore lavora in ufficio da lunedì", "importance": 0.6, "private": false, "replaces": [format!("f{old_id}")], "evidence": "da lunedì lavoro in ufficio"},
                {"about": "user", "section": "notes", "text": "Ettore usa il canale vocale", "importance": 0.5, "private": false, "replaces": [], "evidence": "Bello!"},
            ],
            "close": [], "merge": [],
        });
        let ask = |prompt: &str, _: &Value| -> Result<String, String> {
            assert!(prompt.contains("Ettore: mia moglie Sofia"));
            Ok(answer.to_string())
        };
        let summary = consolidate(&memory, &episode, "Yoru", &ask).unwrap();
        assert!(summary.starts_with("2 fact(s) learnt, 1 closed, 1 new"), "{summary}");
        // Yoru's own words are no evidence.
        assert!(summary.ends_with("1 dropped for not being what the user said"), "{summary}");
        assert!(memory.matching("canale vocale").is_empty());
        let sofia = memory.find("Sofia").unwrap();
        assert_eq!(memory.find("moglie"), Some(sofia));
        assert!(memory.fact(old_id).unwrap().to.is_some());
        assert!(memory.pending_episodes().is_empty());
    }

    #[test]
    fn a_greeting_is_not_sent_to_the_brain() {
        let memory = Memory::open(None);
        memory.log_turn("User", "Ciao", "Ciao! Come stai?");
        memory.end_episode();
        let episode = memory.pending_episodes().remove(0);
        let ask = |_: &str, _: &Value| -> Result<String, String> { panic!("asked the brain about a greeting") };
        assert_eq!(consolidate(&memory, &episode, "Yoru", &ask).unwrap(), "nothing to learn (too short)");
        assert!(memory.pending_episodes().is_empty());
    }

    #[test]
    fn evidence_is_matched_on_words_not_punctuation() {
        let said = words("User: Mia moglie, Sofia, adora i tulipani!");
        assert!(grounded("mia moglie Sofia adora i tulipani", &said));
        assert!(grounded("Sofia adora tulipani", &said));
        assert!(!grounded("Sofia odia le rose", &said));
        assert!(!grounded("", &said));
    }

    #[test]
    fn the_tidy_prunes_but_keeps_what_the_user_told() {
        let memory = Memory::open(None);
        let told = memory.remember("Ettore beve il caffè senza zucchero", None, "about", false, false, None).unwrap();
        let told = fact_id(told.trim_start_matches("remembered as ")).unwrap();
        memory.log_turn("Ettore", "imposta un timer di tre minuti per la pasta", "Fatto.");
        memory.end_episode();
        let user = memory.user();
        let junk = memory.insert_fact(user, "notes", "Ettore ha impostato un timer", 0.5, false, None).unwrap();
        let answer = json!({"entities": [], "relations": [], "facts": [], "close": [format!("f{told}"), format!("f{junk}")], "merge": []});
        let ask = |prompt: &str, _: &Value| -> Result<String, String> {
            assert!(prompt.contains(&format!("[f{told}] (about, about e{user}, importance 0.5, told)")), "{prompt}");
            Ok(format!("```json\n{answer}\n```"))
        };
        assert!(tidy(&memory, "Yoru", &ask).unwrap().starts_with("0 fact(s) learnt, 1 closed"));
        assert!(memory.fact(told).unwrap().to.is_none());
        assert!(memory.fact(junk).unwrap().to.is_some());
    }
}
