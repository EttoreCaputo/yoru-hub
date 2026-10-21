//! What Yoru knows about you: a Markdown file (`memory.md` in the data
//! folder) the assistant reads at the start of each conversation and writes
//! with its `remember`, `revise_memory` and `forget` tools. You read and edit the same file
//! in the app's Memory page.
//!
//! The file is a few sections (`## About you`, `## What matters to you`,
//! `## Tell me about`, `## Don't tell me about`, `## Notes`), each a list of
//! short lines. Anything else in it is kept as it is.

use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;

/// The sections the tools write to, by the name the model uses.
pub const SECTIONS: &[(&str, &str)] = &[
    ("about", "About you"),
    ("matters", "What matters to you"),
    ("tell", "Tell me about"),
    ("quiet", "Don't tell me about"),
    ("notes", "Notes"),
];

/// The longest memory the assistant reads, in characters: it goes into
/// every conversation.
const MAX_CHARS: usize = 12_000;
/// The longest line `remember` writes.
const MAX_LINE: usize = 300;

const TEMPLATE: &str = "# What Yoru knows about you\n\n## About you\n\n## What matters to you\n\n## Tell me about\n\n## Don't tell me about\n\n## Notes\n";

pub struct Memory {
    file: Option<PathBuf>,
    /// Without a file (tests, no data folder), it lives here.
    text: Mutex<String>,
}

impl Memory {
    pub fn open(file: Option<PathBuf>) -> Self {
        let text = file.as_ref().and_then(|f| fs::read_to_string(f).ok()).unwrap_or_else(|| TEMPLATE.to_string());
        Self { file, text: Mutex::new(text) }
    }

    /// The whole file, as the app shows it.
    pub fn text(&self) -> String {
        self.text.lock().unwrap().clone()
    }

    /// Replace the whole file (the app's editor).
    pub fn set_text(&self, text: &str) -> io::Result<()> {
        let mut current = self.text.lock().unwrap();
        *current = text.to_string();
        self.save(&current)
    }

    /// What the assistant reads: the file, cut if it grew too long.
    pub fn for_prompt(&self) -> String {
        self.for_prompt_within(MAX_CHARS)
    }

    /// The same, cut at `max` characters: a brain with a small budget.
    pub fn for_prompt_within(&self, max: usize) -> String {
        let text = self.text();
        if text.chars().count() <= max {
            return text;
        }
        let cut: String = text.chars().take(max).collect();
        format!("{cut}\n[… the rest of the memory is left out: it is too long]")
    }

    /// Add `line` under `section` (one of [`SECTIONS`]'s keys).
    pub fn remember(&self, section: &str, line: &str) -> Result<String, String> {
        let title = SECTIONS.iter().find(|(key, _)| *key == section).map(|(_, t)| *t).ok_or_else(|| format!("unknown section {section}"))?;
        let line: String = line.trim().replace('\n', " ").chars().take(MAX_LINE).collect();
        if line.is_empty() {
            return Err("nothing to remember".into());
        }
        let mut text = self.text.lock().unwrap();
        if text.lines().any(|l| l.trim_start_matches("- ").trim() == line) {
            return Ok("already known".into());
        }
        *text = insert_under(&text, title, &format!("- {line}"));
        self.save(&text).map_err(|e| e.to_string())?;
        Ok(format!("remembered under {title}"))
    }

    /// Remove the lines that contain `text` (case-insensitive).
    pub fn forget(&self, needle: &str) -> Result<String, String> {
        let needle = needle.trim().to_lowercase();
        if needle.len() < 3 {
            return Err("say more precisely what to forget".into());
        }
        let mut text = self.text.lock().unwrap();
        let before = text.lines().count();
        let kept: Vec<&str> = text.lines().filter(|l| !(l.starts_with("- ") && l.to_lowercase().contains(&needle))).collect();
        let removed = before - kept.len();
        if removed == 0 {
            return Ok("nothing like that was in the memory".into());
        }
        *text = kept.join("\n") + "\n";
        self.save(&text).map_err(|e| e.to_string())?;
        Ok(format!("forgot {removed} line(s)"))
    }

    /// Rewrite the one line that contains `needle` (case-insensitive) as
    /// `line`: a fact that changed, or two lines merged into one, keep their
    /// place in the file.
    pub fn revise(&self, needle: &str, line: &str) -> Result<String, String> {
        let needle = needle.trim().to_lowercase();
        if needle.len() < 3 {
            return Err("say more precisely which line to change".into());
        }
        let line: String = line.trim().replace('\n', " ").chars().take(MAX_LINE).collect();
        if line.is_empty() {
            return Err("nothing to write: use forget to remove the line".into());
        }
        let mut text = self.text.lock().unwrap();
        let matches = |l: &&str| l.starts_with("- ") && l.to_lowercase().contains(&needle);
        match text.lines().filter(matches).count() {
            0 => return Err("nothing like that is in the memory: use remember".into()),
            1 => {}
            n => return Err(format!("{n} lines match: say more precisely which one")),
        }
        let revised: Vec<String> = text.lines().map(|l| if matches(&l) { format!("- {line}") } else { l.to_string() }).collect();
        *text = revised.join("\n") + "\n";
        self.save(&text).map_err(|e| e.to_string())?;
        Ok("revised".into())
    }

    fn save(&self, text: &str) -> io::Result<()> {
        let Some(file) = &self.file else { return Ok(()) };
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = file.with_extension("md.tmp");
        fs::write(&tmp, text)?;
        fs::rename(tmp, file)
    }
}

/// `text` with `line` at the end of section `title`, which is added at the
/// end when it isn't there.
fn insert_under(text: &str, title: &str, line: &str) -> String {
    let heading = format!("## {title}");
    let lines: Vec<&str> = text.lines().collect();
    let Some(start) = lines.iter().position(|l| l.trim() == heading) else {
        return format!("{}\n\n{heading}\n\n{line}\n", text.trim_end());
    };
    // The section ends at the next heading; the line goes after its last non-empty line.
    let end = lines[start + 1..].iter().position(|l| l.starts_with("## ")).map_or(lines.len(), |i| start + 1 + i);
    let last = (start + 1..end).rev().find(|&i| !lines[i].trim().is_empty()).unwrap_or(start);
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    if last == start {
        out.insert(start + 1, String::new());
        out.insert(start + 2, line.to_string());
    } else {
        out.insert(last + 1, line.to_string());
    }
    out.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_under_the_right_section_and_forgets() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("memory.md");
        let memory = Memory::open(Some(file.clone()));
        memory.remember("about", "Ettore lives in Italy").unwrap();
        memory.remember("tell", "Mail from the bank").unwrap();
        memory.remember("about", "Works on ESP32 boards").unwrap();
        assert_eq!(memory.remember("about", "Works on ESP32 boards").unwrap(), "already known");
        let text = fs::read_to_string(&file).unwrap();
        let about = text.find("## About you").unwrap();
        let matters = text.find("## What matters to you").unwrap();
        let italy = text.find("- Ettore lives in Italy").unwrap();
        let esp = text.find("- Works on ESP32 boards").unwrap();
        assert!(about < italy && italy < esp && esp < matters, "{text}");
        assert!(text.find("- Mail from the bank").unwrap() > text.find("## Tell me about").unwrap());
        assert_eq!(memory.forget("bank").unwrap(), "forgot 1 line(s)");
        assert!(!Memory::open(Some(file)).text().contains("bank"));
        assert!(memory.remember("nope", "x").is_err());
    }

    #[test]
    fn revises_one_line_in_place() {
        let memory = Memory::open(None);
        memory.remember("about", "Lives in Rome").unwrap();
        memory.remember("about", "Works on ESP32 boards").unwrap();
        memory.remember("notes", "Likes ESP32 dev kits").unwrap();
        assert!(memory.revise("esp32", "x").unwrap_err().starts_with("2 lines match"));
        assert!(memory.revise("Paris", "x").is_err());
        memory.revise("rome", "Lives in Milan since 2026").unwrap();
        let text = memory.text();
        assert!(!text.contains("Rome") && text.find("- Lives in Milan since 2026").unwrap() < text.find("- Works on ESP32").unwrap(), "{text}");
    }

    #[test]
    fn a_missing_section_is_added() {
        let out = insert_under("# Memory\n", "Notes", "- one");
        assert_eq!(out, "# Memory\n\n## Notes\n\n- one\n");
    }
}
