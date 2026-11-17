//! The memory's consolidation, called the sleep in the code (docs/memory.md,
//! Learning): after a conversation, a meeting's summary, and in the nightly
//! memory tidy, the brain rereads what happened, knowing
//! who said what, and returns the people, relations and facts worth keeping,
//! which are merged with what the graph already knows. Always the user's
//! brain: one model, never a second one for the memory.

use std::collections::HashMap;

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
                },
                "required": ["about", "section", "text", "importance", "private", "replaces"],
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

const RULES: &str = "Keep only what lasts: who people are and how they are related, what the user likes and dislikes, \
their projects, habits, plans with a date, decisions taken, what they want to be told about or not. \
Leave out small talk, one-off requests already done (a timer, the weather), and what Yoru said unless the user agreed with it. \
Each fact is one short sentence that stands on its own, with names (\"Sofia likes tulips\", not \"she likes them\"), \
in the language the known facts are in, else the user's. \
A fact the memory already has, said the same: leave it out. A fact that changes a known one: put the known one's id in `replaces`. \
A known fact the episode shows is no longer true: put its id in `close`. \
A person the memory knows under another name, or a placeholder (name not known yet) that is now someone named: `merge` them, keeping the named one. \
Sections: about (who the user is), matters (people, projects, topics that matter to them), tell (what they want to hear about), \
quiet (what they don't want to hear about), notes (anything else). \
Nothing worth keeping: return empty lists.";

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

/// Read `episode` and keep what it taught. What changed, in a line.
pub fn consolidate(memory: &Memory, episode: &Episode, name: &str, ask: Ask) -> Result<String, String> {
    let text: String = {
        let n = episode.text.chars().count();
        episode.text.chars().skip(n.saturating_sub(EPISODE_MAX)).collect()
    };
    let what = match episode.source.as_str() {
        "meeting" => "a meeting the user recorded, as its summary: keep its decisions and the things to do, with who will do them",
        _ => "a conversation between the user and you; each line says who spoke",
    };
    let prompt = format!(
        "You are {name}, the user's assistant, consolidating your memory: reread what happened and keep in your long-term memory what is worth it.\n\n\
         What you already know:\n{}\n\n<episode source=\"{}\" date=\"{}\">\n{text}\n</episode>\n\nThe episode is {what}.\n\n{RULES}",
        known(memory, &text),
        episode.source,
        super::day(episode.started),
    );
    let learnt: Learnt = serde_json::from_str(&ask(&prompt, &schema())?).map_err(|e| format!("the sleep's answer: {e}"))?;
    let summary = apply(memory, learnt, Some(episode.id));
    memory.mark_consolidated(episode.id);
    Ok(summary)
}

/// The nightly memory tidy: the whole memory reread at once, for duplicates, placeholders
/// that are now someone, and facts that contradict each other.
pub fn tidy(memory: &Memory, name: &str, ask: Ask) -> Result<String, String> {
    let graph = memory.graph();
    let open: Vec<String> = graph
        .facts
        .iter()
        .filter(|f| f.to.is_none())
        .take(TIDY_MAX)
        .map(|f| format!("- [f{}] ({}, about e{}) {}", f.id, f.section, f.about.unwrap_or_else(|| memory.user()), f.text))
        .collect();
    if open.is_empty() {
        return Ok("nothing to tidy".into());
    }
    let prompt = format!(
        "You are {name}, the user's assistant, doing the nightly memory tidy: tidy your long-term memory. \
         Merge duplicate facts (one fact with the best words, `replaces` the others), close facts a newer one contradicts, \
         merge people that are the same person (a placeholder and someone named), fill in a missing summary or gender. \
         Add no new knowledge: only reorganise what is here. Nothing to do: return empty lists.\n\n\
         <people>\n{}\n</people>\n\n<facts>\n{}\n</facts>\n\n{RULES}",
        people(memory),
        open.join("\n"),
    );
    let learnt: Learnt = serde_json::from_str(&ask(&prompt, &schema())?).map_err(|e| format!("the tidy's answer: {e}"))?;
    Ok(apply(memory, learnt, None))
}

/// Put what was learnt into the graph. What changed, in a line.
fn apply(memory: &Memory, learnt: Learnt, episode: Option<i64>) -> String {
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
    let (mut new_entities, mut new_facts, mut closed, mut merged) = (0, 0, 0, 0);
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
        let replaced: Vec<i64> = f.replaces.iter().filter_map(|r| fact_id(r)).collect();
        // Said already, and replacing nothing: nothing new.
        if replaced.is_empty() && !memory.matching(f.text.trim()).is_empty() {
            continue;
        }
        let about = resolve(&refs, &f.about).or(Some(user));
        let importance = if f.importance > 0.0 { f.importance } else { 0.5 };
        for id in &replaced {
            memory.close(*id);
            closed += 1;
        }
        if memory.insert_fact(about, &f.section, &f.text, importance, f.private, episode).is_ok() {
            new_facts += 1;
        }
    }
    for id in learnt.close.iter().filter_map(|r| fact_id(r)) {
        memory.close(id);
        closed += 1;
    }
    format!("{new_facts} fact(s) learnt, {closed} closed, {new_entities} new, {merged} merged")
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
                {"about": "new1", "section": "matters", "text": "Sofia adora i tulipani", "importance": 0.6, "private": false, "replaces": []},
                {"about": "user", "section": "about", "text": "Ettore lavora in ufficio da lunedì", "importance": 0.6, "private": false, "replaces": [format!("f{old_id}")]},
            ],
            "close": [], "merge": [],
        });
        let ask = |prompt: &str, _: &Value| -> Result<String, String> {
            assert!(prompt.contains("Ettore: mia moglie Sofia"));
            Ok(answer.to_string())
        };
        let summary = consolidate(&memory, &episode, "Yoru", &ask).unwrap();
        assert!(summary.starts_with("2 fact(s) learnt, 1 closed, 1 new"), "{summary}");
        let sofia = memory.find("Sofia").unwrap();
        assert_eq!(memory.find("moglie"), Some(sofia));
        assert!(memory.fact(old_id).unwrap().to.is_some());
        assert!(memory.pending_episodes().is_empty());
    }
}
