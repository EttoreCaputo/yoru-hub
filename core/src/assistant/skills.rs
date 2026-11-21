//! Skills (docs/agent.md, The agent): instructions for a particular job, as
//! Markdown with a name and a one-line description in its frontmatter. Only
//! the names and descriptions are in the prompt; the model reads a skill's
//! body with `load_skill` when a request needs it, so the prompt stays short
//! however many there are.
//!
//! Yoru's own skills are built in (`skills/*.md`). A file in the data
//! folder's `skills/` adds one, or replaces the built-in of the same name.

use std::fs;

use serde_json::{Value, json};

/// One skill: what the prompt lists, and the instructions.
#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub body: String,
}

/// The skills that come with Yoru.
const BUILT_IN: &[&str] = &[
    include_str!("skills/summarize-meeting.md"),
    include_str!("skills/process-meeting.md"),
    include_str!("skills/people-profile.md"),
    include_str!("skills/dream.md"),
];

/// Every skill, by name: the built-in ones and the user's, theirs first
/// where both have a name.
pub fn all() -> Vec<Skill> {
    let mut skills: Vec<Skill> = user_skills();
    for text in BUILT_IN {
        if let Some(s) = parse(text)
            && !skills.iter().any(|u| u.name == s.name)
        {
            skills.push(s);
        }
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// The skill named `name`.
pub fn get(name: &str) -> Option<Skill> {
    all().into_iter().find(|s| s.name == name.trim())
}

fn user_skills() -> Vec<Skill> {
    let Some(dir) = crate::paths::data_dir().map(|d| d.join("skills")) else { return Vec::new() };
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .filter_map(|p| fs::read_to_string(p).ok())
        .filter_map(|t| parse(&t))
        .collect()
}

/// A skill file: `name:` and `description:` between `---` lines, then the body.
fn parse(text: &str) -> Option<Skill> {
    let rest = text.trim_start().strip_prefix("---")?;
    let (head, body) = rest.split_once("\n---")?;
    let field = |key: &str| head.lines().find_map(|l| l.trim().strip_prefix(key)?.strip_prefix(':').map(|v| v.trim().trim_matches('"').to_string()));
    let name = field("name").filter(|n| !n.is_empty())?;
    Some(Skill { name, description: field("description").unwrap_or_default(), body: body.trim().to_string() })
}

/// The prompt's list of skills: a line each.
pub fn prompt_section(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = skills.iter().map(|s| format!("- {}: {}", s.name, s.description)).collect();
    format!(
        "\n\n# Skills\nInstructions for particular jobs. When a request is one of these, first read its skill with load_skill, then follow it:\n{}",
        lines.join("\n")
    )
}

/// `load_skill`, as a tool.
pub fn tool() -> Value {
    json!({
        "name": "load_skill",
        "description": "Read the instructions of a skill listed under Skills, before doing that job.",
        "input_schema": {"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]},
    })
}

/// What `load_skill` answers: the instructions, or the skills there are.
pub fn load(input: &Value) -> Result<String, String> {
    let name = input["name"].as_str().unwrap_or_default();
    get(name).map(|s| s.body).ok_or_else(|| {
        let names: Vec<String> = all().into_iter().map(|s| s.name).collect();
        format!("no skill named {name:?}; the skills are: {}", names.join(", "))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_skills_read() {
        for text in BUILT_IN {
            let s = parse(text).expect("a built-in skill has a name");
            assert!(!s.description.is_empty() && !s.body.is_empty() && !s.body.starts_with("---"));
        }
        assert!(get("summarize-meeting").unwrap().body.contains("`interview`"));
        assert!(get("process-meeting").is_some() && get("people-profile").is_some() && get("dream").is_some());
    }

    #[test]
    fn reads_the_frontmatter() {
        let s = parse("---\nname: dream\ndescription: \"Tidy the notes\"\n---\n\nDo it.\n").unwrap();
        assert_eq!((s.name.as_str(), s.description.as_str(), s.body.as_str()), ("dream", "Tidy the notes", "Do it."));
        assert!(parse("no frontmatter").is_none());
        assert!(parse("---\ndescription: x\n---\nbody").is_none());
    }

    #[test]
    fn an_unknown_skill_lists_the_others() {
        let e = load(&json!({"name": "nope"})).unwrap_err();
        assert!(e.contains("summarize-meeting"));
    }

    #[test]
    fn the_prompt_lists_names_not_bodies() {
        let section = prompt_section(&all());
        assert!(section.contains("- summarize-meeting: ") && !section.contains("## The styles"));
    }
}
