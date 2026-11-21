//! Yoru's notes, the second brain (docs/agent.md, The notes): plain
//! Markdown files in `brain/` in the data folder, one per person, project
//! or topic, with YAML frontmatter and `[[wikilinks]]`, so the folder also
//! opens in Obsidian. Every change is a commit in a git repository inside
//! it, so a bad change is undone and the Memory page can show what changed.
//!
//! `me.md` (who the user is) and the index (a line per note, made from each
//! note's `description`) go into every request; the rest is read when needed.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use git2::{IndexAddOption, Repository, Signature};

/// The folders a note lives in, and their title in the index.
pub const FOLDERS: &[(&str, &str)] = &[("people", "People"), ("projects", "Projects"), ("topics", "Topics")];
/// Characters of `me.md` in the prompt, at most.
const ME_MAX: usize = 4000;
/// Characters of the index in the prompt, at most.
const INDEX_MAX: usize = 6000;
/// Notes a search gives.
const FOUND: usize = 8;
/// Characters a note read gives at once.
const READ_MAX: usize = 20_000;

const ME_TEMPLATE: &str = "---
type: me
description: Who the user is, what they care about most, and what not to tell them
updated: {today}
---

# Me

## Who I am

## What matters to me

## Tell me about

## Don't tell me about
";

/// A note as listed: its path in `brain/` and its frontmatter.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NoteInfo {
    pub path: String,
    pub kind: String,
    pub description: String,
    pub aliases: Vec<String>,
    pub updated: String,
    /// Health, money, intimate matters: never sent to a cloud brain.
    pub private: bool,
}

pub struct Notes {
    dir: Option<PathBuf>,
    /// One change at a time: each is a commit.
    lock: Mutex<()>,
}

impl std::fmt::Debug for Notes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Notes").field("dir", &self.dir).finish()
    }
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

impl Notes {
    /// The notes in `data/brain`, made the first time (with `me.md` and a
    /// repository); without a folder there are none (in the tests, a
    /// folder of their own).
    pub fn open(data: Option<&Path>) -> Self {
        let mut dir = data.map(|d| d.join("brain"));
        if cfg!(test) && dir.is_none() {
            dir = Some(std::env::temp_dir().join(format!("yoru-brain-{}-{}", std::process::id(), crate::assistant::chats::new_id())));
        }
        let notes = Self { dir, lock: Mutex::new(()) };
        if let Err(e) = notes.init() {
            eprintln!("brain: {e}");
        }
        notes
    }

    /// Whether the old memory (`memory.db`) was written out as notes.
    pub fn migrated(&self) -> bool {
        self.dir.as_ref().is_none_or(|d| d.join(".migrated").exists())
    }

    /// Write out the old memory once: `notes` (path, text), then the mark.
    pub fn migrate(&self, notes: &[(String, String)]) -> Result<usize, String> {
        let dir = self.dir()?;
        let _lock = self.lock.lock().unwrap();
        for (path, text) in notes {
            let file = self.file(path)?;
            fs::write(&file, stamp(text.trim_end()) + "\n").map_err(|e| e.to_string())?;
        }
        fs::write(dir.join(".migrated"), today()).map_err(|e| e.to_string())?;
        self.save_index()?;
        self.commit("Write out the old memory as notes")?;
        Ok(notes.len())
    }

    fn init(&self) -> Result<(), String> {
        let Some(dir) = &self.dir else { return Ok(()) };
        // Yoru's own marks are not notes.
        if !dir.join(".gitignore").exists() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            fs::write(dir.join(".gitignore"), ".migrated\n.dreamed\n").map_err(|e| e.to_string())?;
        }
        for (folder, _) in FOLDERS.iter().chain(&[("archive", "")]) {
            fs::create_dir_all(dir.join(folder)).map_err(|e| e.to_string())?;
        }
        if !dir.join(".git").exists() {
            Repository::init(dir).map_err(|e| e.to_string())?;
        }
        if !dir.join("me.md").exists() {
            fs::write(dir.join("me.md"), ME_TEMPLATE.replace("{today}", &today())).map_err(|e| e.to_string())?;
            self.save_index()?;
            self.commit("The notes begin")?;
        }
        Ok(())
    }

    fn dir(&self) -> Result<&Path, String> {
        self.dir.as_deref().ok_or_else(|| "the notes have no folder".to_string())
    }

    /// `path` inside `brain/`, or why it can't be a note.
    fn file(&self, path: &str) -> Result<PathBuf, String> {
        let path = path.trim().trim_start_matches("brain/");
        let clean = Path::new(path);
        let ok = path.ends_with(".md")
            && clean.components().all(|c| matches!(c, std::path::Component::Normal(_)))
            && (path == "me.md" || FOLDERS.iter().chain(&[("archive", "")]).any(|(f, _)| path.starts_with(&format!("{f}/"))));
        if !ok {
            return Err(format!("{path}: a note is me.md or people/, projects/, topics/ or archive/<name>.md"));
        }
        Ok(self.dir()?.join(clean))
    }

    /// Note `path`'s text, from line `from` if long.
    pub fn read(&self, path: &str, private_ok: bool) -> Result<String, String> {
        let file = self.file(path)?;
        let text = fs::read_to_string(&file).map_err(|_| format!("no note {path}: find one with brain_search"))?;
        if !private_ok && parse(&text).private {
            return Err(format!("{path} is private: it isn't sent to an online brain"));
        }
        Ok(text.chars().take(READ_MAX).collect())
    }

    /// Write note `path` whole: what is true now and its timeline. Its
    /// `updated` becomes today. `why` is the commit's message.
    pub fn write(&self, path: &str, content: &str, why: &str) -> Result<String, String> {
        let file = self.file(path)?;
        if path.trim() != "me.md" && parse(content).description.is_empty() {
            return Err("start the note with its frontmatter: ---, type, description (one line: what it is about), aliases, ---".into());
        }
        let _lock = self.lock.lock().unwrap();
        let existed = file.exists();
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(&file, stamp(content.trim_end()) + "\n").map_err(|e| e.to_string())?;
        // A note moved to the archive leaves its old place.
        if let Some(name) = path.trim().strip_prefix("archive/") {
            for (folder, _) in FOLDERS {
                let _ = fs::remove_file(self.dir()?.join(folder).join(name));
            }
        }
        self.save_index()?;
        self.commit(&message(why, &format!("{} {path}", if existed { "Update" } else { "Add" })))?;
        Ok(format!("saved {path}"))
    }

    /// Add a dated line to note `path`'s timeline.
    pub fn append(&self, path: &str, entry: &str, why: &str) -> Result<String, String> {
        let line = format!("- {} {}", today(), entry.trim().trim_start_matches("- "));
        self.add_under(path, "Timeline", &line, &message(why, &format!("Note on {path}")))?;
        Ok(format!("added to {path}"))
    }

    /// «Enough» on a notification: a line in `me.md`'s "Don't tell me about",
    /// which every request carries.
    pub fn keep_quiet(&self, what: &str) -> Result<(), String> {
        self.add_under("me.md", "Don't tell me about", &format!("- {} [Source: «Enough» on a notification | {}]", what.trim(), today()), "Keep quiet about a notification the user found useless")
    }

    /// `line` at the end of section `heading` of note `path` (the section
    /// added at the end when missing), committed as `commit`.
    fn add_under(&self, path: &str, heading: &str, line: &str, commit: &str) -> Result<(), String> {
        let file = self.file(path)?;
        let _lock = self.lock.lock().unwrap();
        let text = fs::read_to_string(&file).map_err(|_| format!("no note {path}: write it first with brain_write"))?;
        let head = format!("\n## {heading}\n");
        let text = match text.find(&head) {
            Some(at) => {
                // Before the next section, if any.
                let body = at + head.len() - 1;
                let end = text[body..].find("\n## ").map_or(text.len(), |i| body + i);
                format!("{}\n{line}\n\n{}", text[..end].trim_end(), text[end..].trim_start_matches('\n'))
            }
            None => format!("{}\n\n## {heading}\n{line}\n", text.trim_end()),
        };
        fs::write(&file, stamp(text.trim_end()) + "\n").map_err(|e| e.to_string())?;
        self.save_index()?;
        self.commit(commit)
    }

    /// Every note but the archived ones, by folder.
    pub fn list(&self) -> Vec<NoteInfo> {
        let Ok(dir) = self.dir() else { return Vec::new() };
        let mut out = Vec::new();
        for (folder, _) in FOLDERS {
            let Ok(entries) = fs::read_dir(dir.join(folder)) else { continue };
            let mut found: Vec<NoteInfo> = entries
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
                .filter_map(|e| {
                    let text = fs::read_to_string(e.path()).ok()?;
                    Some(NoteInfo { path: format!("{folder}/{}", e.file_name().to_string_lossy()), ..parse(&text) })
                })
                .collect();
            found.sort_by(|a, b| a.path.cmp(&b.path));
            out.extend(found);
        }
        out
    }

    /// The notes that match `query`, best first, with the lines that match.
    pub fn search(&self, query: &str, private_ok: bool) -> String {
        let Ok(dir) = self.dir() else { return "no notes".into() };
        let words: Vec<String> = query.split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 3).map(str::to_lowercase).collect();
        let mut all: Vec<(String, String)> = vec![("me.md".into(), fs::read_to_string(dir.join("me.md")).unwrap_or_default())];
        for folder in FOLDERS.iter().map(|(f, _)| *f).chain(["archive"]) {
            let Ok(entries) = fs::read_dir(dir.join(folder)) else { continue };
            all.extend(entries.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "md")).filter_map(|e| Some((format!("{folder}/{}", e.file_name().to_string_lossy()), fs::read_to_string(e.path()).ok()?))));
        }
        let mut scored: Vec<(usize, String)> = all
            .into_iter()
            .filter_map(|(path, text)| {
                let info = parse(&text);
                if info.private && !private_ok {
                    return None;
                }
                let head = format!("{path} {} {}", info.description, info.aliases.join(" ")).to_lowercase();
                let lower = text.to_lowercase();
                let score: usize = words.iter().map(|w| if head.contains(w.as_str()) { 3 } else { 0 } + lower.matches(w.as_str()).count().min(5)).sum();
                let score = if path.starts_with("archive/") { score / 2 } else { score };
                if score == 0 {
                    return None;
                }
                let lines: Vec<&str> = text.lines().filter(|l| !l.starts_with("---") && words.iter().any(|w| l.to_lowercase().contains(w.as_str()))).take(2).collect();
                let lines: String = lines.iter().map(|l| format!("\n    {}", l.trim().chars().take(200).collect::<String>())).collect();
                Some((score, format!("{path} — {}{lines}", info.description)))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        if scored.is_empty() {
            return "no note matches: try other words, or history_search for what was said".into();
        }
        scored.into_iter().take(FOUND).map(|(_, s)| s).collect::<Vec<_>>().join("\n")
    }

    /// What every request carries: `me.md` and the index, in about `max`
    /// characters (a small brain's prompt has less room).
    pub fn prompt(&self, private_ok: bool, max: usize) -> String {
        let me = self.dir().ok().and_then(|d| fs::read_to_string(d.join("me.md")).ok()).map(|t| body(&t).trim().to_string()).unwrap_or_default();
        let me: String = me.chars().take(ME_MAX.min(max * 2 / 5)).collect();
        let room = INDEX_MAX.min(max.saturating_sub(me.chars().count()));
        let index: String = index_text(&self.list().into_iter().filter(|n| private_ok || !n.private).collect::<Vec<_>>()).chars().take(room).collect();
        format!("<me>\n{me}\n</me>\n\n<notes_index>\n{index}\n</notes_index>")
    }

    /// `index.md`: written for people and Obsidian; the prompt makes its own.
    fn save_index(&self) -> Result<(), String> {
        let text = format!("# Index\n\nWritten by Yoru: one line per note.\n\n{}\n", index_text(&self.list()));
        fs::write(self.dir()?.join("index.md"), text).map_err(|e| e.to_string())
    }

    /// Commit every change in `brain/`; nothing to commit is no error.
    fn commit(&self, message: &str) -> Result<(), String> {
        let dir = self.dir()?;
        let run = || -> Result<(), git2::Error> {
            let repo = Repository::open(dir)?;
            let mut index = repo.index()?;
            index.add_all(["*"], IndexAddOption::DEFAULT, None)?;
            index.update_all(["*"], None)?;
            index.write()?;
            let tree_id = index.write_tree()?;
            let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
            if parent.as_ref().is_some_and(|p| p.tree_id() == tree_id) {
                return Ok(());
            }
            let tree = repo.find_tree(tree_id)?;
            let me = Signature::now("Yoru", "yoru@localhost")?;
            let parents: Vec<&git2::Commit> = parent.iter().collect();
            repo.commit(Some("HEAD"), &me, &me, message, &tree, &parents)?;
            Ok(())
        };
        run().map_err(|e| format!("the notes' history: {e}"))
    }

    /// The day of the last dream (YYYY-MM-DD), empty if never.
    pub fn dreamed_on(&self) -> String {
        self.dir.as_ref().and_then(|d| fs::read_to_string(d.join(".dreamed")).ok()).unwrap_or_default().trim().to_string()
    }

    /// Start tonight's dream: its writes go to a branch of their own,
    /// `dream/<day>`, from what is there now. The branch the notes were on.
    pub fn begin_dream(&self) -> Result<String, String> {
        let dir = self.dir()?;
        let _lock = self.lock.lock().unwrap();
        fs::write(dir.join(".dreamed"), today()).map_err(|e| e.to_string())?;
        let run = || -> Result<String, git2::Error> {
            let repo = Repository::open(dir)?;
            let head = repo.head()?;
            let main = head.name().unwrap_or("refs/heads/master").to_string();
            let branch = repo.branch(&format!("dream/{}", today()), &head.peel_to_commit()?, true)?;
            repo.set_head(branch.get().name().unwrap_or_default())?;
            Ok(main)
        };
        run().map_err(|e| format!("the notes' history: {e}"))
    }

    /// End the dream: kept, `main` moves on to what it wrote (the branch
    /// stays, to see or undo the night); not kept, the notes go back to how
    /// they were.
    pub fn end_dream(&self, main: &str, keep: bool) -> Result<(), String> {
        let dir = self.dir()?;
        let _lock = self.lock.lock().unwrap();
        let run = || -> Result<(), git2::Error> {
            let repo = Repository::open(dir)?;
            let dreamt = repo.head()?.peel_to_commit()?.id();
            if keep {
                repo.reference(main, dreamt, true, "the dream, kept")?;
                repo.set_head(main)?;
            } else {
                repo.set_head(main)?;
                repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force().remove_untracked(true)))?;
            }
            Ok(())
        };
        run().map_err(|e| format!("the notes' history: {e}"))?;
        if !keep {
            self.save_index()?;
        }
        Ok(())
    }

    /// The commits, newest first: when, and what.
    pub fn history(&self, max: usize) -> Vec<(i64, String)> {
        let Ok(dir) = self.dir() else { return Vec::new() };
        let Ok(repo) = Repository::open(dir) else { return Vec::new() };
        let Ok(mut walk) = repo.revwalk() else { return Vec::new() };
        if walk.push_head().is_err() {
            return Vec::new();
        }
        walk.flatten().take(max).filter_map(|id| repo.find_commit(id).ok()).map(|c| (c.time().seconds() * 1000, c.summary().ok().flatten().unwrap_or_default().to_string())).collect()
    }
}

/// The commit message: the model's reason, else what was done.
fn message(why: &str, what: &str) -> String {
    let why = why.trim();
    if why.is_empty() { what.to_string() } else { format!("{what}: {why}") }
}

/// The index's lines, by folder.
fn index_text(notes: &[NoteInfo]) -> String {
    let mut out = Vec::new();
    for (folder, title) in FOLDERS {
        let lines: Vec<String> = notes
            .iter()
            .filter(|n| n.path.starts_with(&format!("{folder}/")))
            .map(|n| {
                let aliases = if n.aliases.is_empty() { String::new() } else { format!(" (also {})", n.aliases.join(", ")) };
                format!("- [[{}]]{aliases}: {}", n.path.trim_end_matches(".md"), n.description)
            })
            .collect();
        if !lines.is_empty() {
            out.push(format!("## {title}\n{}", lines.join("\n")));
        }
    }
    if out.is_empty() { "(no notes yet)".into() } else { out.join("\n\n") }
}

/// The frontmatter's fields; a note without one has none.
pub fn parse(text: &str) -> NoteInfo {
    let mut info = NoteInfo::default();
    let Some(front) = frontmatter(text) else { return info };
    for line in front.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = value.trim().trim_matches('"');
        match key.trim() {
            "type" => info.kind = value.into(),
            "description" => info.description = value.into(),
            "updated" => info.updated = value.into(),
            "private" => info.private = value == "true",
            "aliases" => info.aliases = list(value),
            _ => {}
        }
    }
    info
}

/// `[a, b]` or `a, b` as a list.
fn list(value: &str) -> Vec<String> {
    value.trim_start_matches('[').trim_end_matches(']').split(',').map(|s| s.trim().trim_matches('"').to_string()).filter(|s| !s.is_empty()).collect()
}

fn frontmatter(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    Some(&rest[..end])
}

/// The text after the frontmatter.
fn body(text: &str) -> &str {
    match (text.strip_prefix("---\n"), frontmatter(text)) {
        (Some(rest), Some(front)) => rest[front.len() + "\n---".len()..].trim_start_matches('\n'),
        _ => text,
    }
}

/// `text` with `updated:` today, in the frontmatter (added if missing).
fn stamp(text: &str) -> String {
    let Some(front) = frontmatter(text) else { return text.to_string() };
    let mut lines: Vec<String> = front.lines().filter(|l| !l.trim_start().starts_with("updated:")).map(String::from).collect();
    lines.push(format!("updated: {}", today()));
    format!("---\n{}\n---\n{}", lines.join("\n"), body(text))
}

// ---- The Memory page (docs/agent.md, From today's memory) ----

/// A note as the Memory page draws it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct NoteView {
    pub path: String,
    pub kind: String,
    pub title: String,
    pub description: String,
    pub aliases: Vec<String>,
    pub private: bool,
    pub updated: String,
    /// What is true now (not the timeline), a line each with its section:
    /// `me.md`'s by key (about, matters, tell, quiet, notes), else "about".
    pub lines: Vec<(String, String)>,
    /// The notes it links to, as paths.
    pub links: Vec<String>,
}

/// One change to a note, as the page lists it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Change {
    pub at: i64,
    pub what: String,
}

/// `me.md`'s headings, by the keys the page colours them with.
const ME_KEYS: [(&str, &str); 5] = [("Who I am", "about"), ("What matters to me", "matters"), ("Tell me about", "tell"), ("Don't tell me about", "quiet"), ("Notes", "notes")];

fn view(path: &str, text: &str) -> NoteView {
    let info = parse(text);
    let body = body(text);
    let title = body.lines().find_map(|l| l.strip_prefix("# ")).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let stem = Path::new(path).file_stem().map(|s| s.to_string_lossy().replace('-', " ")).unwrap_or_default();
    let mut lines = Vec::new();
    let mut section = "about";
    let mut timeline = false;
    for line in body.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            timeline = heading.trim() == "Timeline";
            section = ME_KEYS.iter().find(|(h, _)| *h == heading.trim()).map_or("about", |(_, k)| *k);
            continue;
        }
        if let Some(item) = line.strip_prefix("- ").filter(|_| !timeline) {
            let shown = item.split(" [Source:").next().unwrap_or(item).trim();
            if !shown.is_empty() {
                lines.push((section.to_string(), shown.to_string()));
            }
        }
    }
    let mut links: Vec<String> = Vec::new();
    for piece in body.split("[[").skip(1) {
        let Some(inner) = piece.split("]]").next() else { continue };
        let target = inner.split('|').next().unwrap_or_default().trim();
        let target = if target.ends_with(".md") { target.to_string() } else { format!("{target}.md") };
        if target != path && !links.contains(&target) {
            links.push(target);
        }
    }
    NoteView {
        path: path.into(),
        kind: if path == "me.md" { "me".into() } else { info.kind },
        title: title.unwrap_or(stem),
        description: info.description,
        aliases: info.aliases,
        private: info.private,
        updated: info.updated,
        lines,
        links,
    }
}

impl Notes {
    /// Every note but the archived ones, `me.md` first, as the page draws them.
    pub fn views(&self) -> Vec<NoteView> {
        let Ok(dir) = self.dir() else { return Vec::new() };
        std::iter::once("me.md".to_string())
            .chain(self.list().into_iter().map(|n| n.path))
            .filter_map(|path| fs::read_to_string(dir.join(&path)).ok().map(|text| view(&path, &text)))
            .collect()
    }

    /// Note `path` whole, private or not: for the user's own page.
    pub fn text(&self, path: &str) -> Result<String, String> {
        fs::read_to_string(self.file(path)?).map_err(|_| format!("no note {path}"))
    }

    /// Move note `path` to the archive: no longer true, never deleted.
    pub fn archive(&self, path: &str, why: &str) -> Result<String, String> {
        let name = path.trim().rsplit('/').next().unwrap_or_default().to_string();
        if path.trim() == "me.md" || path.trim().starts_with("archive/") {
            return Err(format!("{path} can't be archived"));
        }
        let text = self.text(path)?;
        self.write(&format!("archive/{name}"), &text, why)
    }

    /// The changes to note `path`, newest first.
    pub fn changes(&self, path: &str, max: usize) -> Vec<Change> {
        let Ok(dir) = self.dir() else { return Vec::new() };
        let Ok(repo) = Repository::open(dir) else { return Vec::new() };
        let Ok(mut walk) = repo.revwalk() else { return Vec::new() };
        if walk.push_head().is_err() {
            return Vec::new();
        }
        let blob = |c: &git2::Commit| c.tree().ok().and_then(|t| t.get_path(Path::new(path)).ok()).map(|e| e.id());
        walk.flatten()
            .take(2000)
            .filter_map(|id| repo.find_commit(id).ok())
            .filter(|c| blob(c) != c.parent(0).ok().and_then(|p| blob(&p)))
            .take(max)
            .map(|c| Change { at: c.time().seconds() * 1000, what: c.summary().ok().flatten().unwrap_or_default().to_string() })
            .collect()
    }

    /// A new note about `name`, a `kind` (person, project, or anything
    /// else: a topic): its path. Refused when there is one already.
    pub fn create(&self, kind: &str, name: &str) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("a name is needed".into());
        }
        let folder = match kind {
            "person" => "people",
            "project" => "projects",
            _ => "topics",
        };
        let path = format!("{folder}/{}.md", slug(name));
        if self.file(&path)?.exists() {
            return Err(format!("{path} is there already"));
        }
        self.write(&path, &format!("---\ntype: {kind}\ndescription: {name}\naliases: []\n---\n\n# {name}\n\n## Now\n\n## Timeline\n"), "Added on the Memory page")?;
        Ok(path)
    }

    /// The notes' folder, to open in the Finder or Obsidian.
    pub fn folder(&self) -> Option<PathBuf> {
        self.dir.clone()
    }
}

// ---- The old memory, written out once (docs/agent.md, From today's memory) ----

/// Lowercase, dashes, letters and digits: a note's file name.
fn slug(name: &str) -> String {
    let s: String = name.trim().to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    if s.is_empty() { "unnamed".into() } else { s }
}

fn day(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string()).unwrap_or_default()
}

/// Every entity of `memory.db` as a note, the user's facts as `me.md`:
/// facts still true under what is true now, closed ones in the timeline,
/// private ones in a private note of their own. `memory.db` stays as it is
/// (the voices still use it).
pub fn migrate(memory: &super::memory::Memory, notes: &Notes) -> Result<usize, String> {
    use super::memory::Fact;
    // The graph's sections, under the headings of `me.md`.
    const ME_SECTIONS: [(&str, &str); 5] =
        [("about", "Who I am"), ("matters", "What matters to me"), ("tell", "Tell me about"), ("quiet", "Don't tell me about"), ("notes", "Notes")];
    let graph = memory.graph();
    let path_of = |e: &super::memory::Entity| -> String {
        let folder = match e.kind.as_str() {
            "person" => "people",
            "project" => "projects",
            _ => "topics",
        };
        format!("{folder}/{}.md", slug(&e.name))
    };
    let link = |id: i64| -> String {
        match graph.entities.iter().find(|e| e.id == id) {
            Some(e) if e.is_user => "the user".to_string(),
            Some(e) => format!("[[{}|{}]]", path_of(e).trim_end_matches(".md"), e.name),
            None => "someone".into(),
        }
    };
    let line = |f: &Fact| format!("- {} [Source: Yoru's earlier memory | {}]", f.text.trim(), day(f.from));
    let timeline = |facts: &[&Fact]| -> String {
        let mut closed: Vec<&&Fact> = facts.iter().filter(|f| f.to.is_some()).collect();
        closed.sort_by_key(|f| f.from);
        closed.iter().map(|f| format!("- {} {} (no longer true from {})", day(f.from), f.text.trim(), day(f.to.unwrap_or(f.from)))).collect::<Vec<_>>().join("\n")
    };
    let mut out: Vec<(String, String)> = Vec::new();
    for e in &graph.entities {
        let facts: Vec<&Fact> = graph.facts.iter().filter(|f| f.about == Some(e.id) || (e.is_user && f.about.is_none())).collect();
        let (private, public): (Vec<&Fact>, Vec<&Fact>) = facts.into_iter().partition(|f| f.private);
        let relations: Vec<String> = graph.relations.iter().filter(|r| r.a == e.id && r.to.is_none()).map(|r| format!("- {} of {}", r.kind, link(r.b))).collect();
        if e.is_user {
            let mut text = "---\ntype: me\ndescription: Who the user is, what they care about most, and what not to tell them\n---\n\n# Me\n".to_string();
            if e.name != "User" {
                text.push_str(&format!("\nName: {}\n", e.name));
            }
            for (key, title) in ME_SECTIONS {
                let lines: Vec<String> = public.iter().filter(|f| f.section == *key && f.to.is_none()).map(|f| line(f)).collect();
                text.push_str(&format!("\n## {title}\n{}\n", lines.join("\n")));
            }
            if !relations.is_empty() {
                text.push_str(&format!("\n## People\n{}\n", relations.join("\n")));
            }
            let past = timeline(&public);
            if !past.is_empty() {
                text.push_str(&format!("\n## Timeline\n{past}\n"));
            }
            out.push(("me.md".into(), text));
        } else {
            let path = path_of(e);
            let what = e.label.clone().map(|l| format!(", {l}")).unwrap_or_default();
            let description = if e.summary.trim().is_empty() { format!("{}{what}", e.name) } else { e.summary.trim().replace('\n', " ") };
            let mut text = format!(
                "---\ntype: {}\ndescription: {}\naliases: [{}]\n---\n\n# {}\n\n## Now\n{}\n",
                e.kind,
                description.chars().take(200).collect::<String>(),
                e.aliases.join(", "),
                e.name,
                public.iter().filter(|f| f.to.is_none()).map(|f| line(f)).chain(relations.iter().cloned()).collect::<Vec<_>>().join("\n")
            );
            let past = timeline(&public);
            if !past.is_empty() {
                text.push_str(&format!("\n## Timeline\n{past}\n"));
            }
            out.push((path, text));
        }
        if !private.is_empty() {
            let name = if e.is_user { "me".to_string() } else { slug(&e.name) };
            let about = if e.is_user { "the user".to_string() } else { e.name.clone() };
            let text = format!(
                "---\ntype: private\ndescription: Private matters about {about} (health, money, intimate)\nprivate: true\n---\n\n# Private: {about}\n\n## Now\n{}\n\n## Timeline\n{}\n",
                private.iter().filter(|f| f.to.is_none()).map(|f| line(f)).collect::<Vec<_>>().join("\n"),
                timeline(&private)
            );
            out.push((format!("topics/{name}-private.md"), text));
        }
    }
    notes.migrate(&out)
}

// ---- The agent's tools ----

pub(super) fn tools() -> Vec<serde_json::Value> {
    use serde_json::json;
    let path = json!({"type": "string", "description": "me.md, or people/, projects/, topics/ or archive/<name>.md (lowercase, dashes: people/marco-rossi.md)"});
    vec![
        json!({
            "name": "brain_search",
            "description": "Search your notes (people, projects, topics, me.md, the archive) by words: names, aliases, what is in them. Gives each note's path and description with the lines that match.",
            "input_schema": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]},
        }),
        json!({
            "name": "brain_read",
            "description": "Read a note whole: what is true now and its timeline.",
            "input_schema": {"type": "object", "properties": {"path": path}, "required": ["path"]},
        }),
        json!({
            "name": "brain_write",
            "description": "Write a note whole, new or rewritten: frontmatter (type, description: one line on what it is about, aliases, private: true for health, money \
or intimate matters), then what is true now, then ## Timeline with its dated entries kept. Read it first when it exists. To retire a note no longer true, write it to archive/.",
            "input_schema": {
                "type": "object",
                "properties": {"path": path, "content": {"type": "string", "description": "The whole note, in Markdown"}, "why": {"type": "string", "description": "A few words on what changed and why"}},
                "required": ["path", "content"],
            },
        }),
        json!({
            "name": "brain_append",
            "description": "Add a dated entry to a note's timeline (the date is added): something that happened or was said, with its source.",
            "input_schema": {
                "type": "object",
                "properties": {"path": path, "entry": {"type": "string"}, "why": {"type": "string"}},
                "required": ["path", "entry"],
            },
        }),
    ]
}

/// Run a `brain_*` tool for `a`.
pub(super) fn call(a: &super::Assistant, name: &str, input: &serde_json::Value) -> Result<String, String> {
    let private_ok = a.settings().brain() == Some(super::Brain::Ollama);
    let text = |key: &str| input[key].as_str().unwrap_or_default();
    let result = match name {
        "brain_search" => Ok(a.notes.search(text("query"), private_ok)),
        "brain_read" => a.notes.read(text("path"), private_ok),
        "brain_write" => a.notes.write(text("path"), text("content"), text("why")),
        "brain_append" => a.notes.append(text("path"), text("entry"), text("why")),
        _ => Err(format!("no tool {name}")),
    };
    if result.is_ok() && matches!(name, "brain_write" | "brain_append") {
        a.show_done("Memory", "remember");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yoru-notes-{}-{}", std::process::id(), crate::assistant::chats::new_id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    const MARCO: &str = "---\ntype: person\ndescription: Marco, head of design at Acme\naliases: [Marco Rossi]\n---\n# Marco\n\nHead of design at Acme («I moved to Acme in September»).\n";

    #[test]
    fn a_dream_is_kept_or_undone() {
        let notes = Notes::open(Some(&temp()));
        let main = notes.begin_dream().unwrap();
        notes.write("people/marco.md", MARCO, "dreamt").unwrap();
        notes.end_dream(&main, false).unwrap();
        assert!(notes.read("people/marco.md", false).is_err(), "undone");
        assert!(!notes.prompt(false, 12_000).contains("marco"));
        assert_eq!(notes.dreamed_on(), today());
        let main = notes.begin_dream().unwrap();
        notes.write("people/marco.md", MARCO, "dreamt").unwrap();
        notes.end_dream(&main, true).unwrap();
        assert!(notes.read("people/marco.md", false).is_ok());
        assert!(notes.history(1)[0].1.contains("dreamt"));
        let repo = Repository::open(notes.dir().unwrap()).unwrap();
        assert_eq!(repo.head().unwrap().name().unwrap(), main);
        assert!(repo.find_branch(&format!("dream/{}", today()), git2::BranchType::Local).is_ok());
    }

    #[test]
    fn the_page_sees_lines_links_and_changes() {
        let notes = Notes::open(Some(&temp()));
        notes.write("people/marco.md", &format!("{MARCO}\n## Now\n- Works on [[projects/pricing|Pricing]] [Source: chat:a#1 | 2026-11-20]\n\n## Timeline\n- 2026-01-01 Old\n"), "met").unwrap();
        notes.keep_quiet("football").unwrap();
        let views = notes.views();
        assert_eq!(views[0].kind, "me");
        assert!(views[0].lines.contains(&("quiet".into(), "football".into())));
        let marco = views.iter().find(|v| v.path == "people/marco.md").unwrap();
        assert_eq!(marco.title, "Marco");
        assert_eq!(marco.lines, vec![("about".to_string(), "Works on [[projects/pricing|Pricing]]".to_string())]);
        assert_eq!(marco.links, vec!["projects/pricing.md".to_string()]);
        assert_eq!(notes.changes("people/marco.md", 10).len(), 1);
        notes.archive("people/marco.md", "left").unwrap();
        assert!(notes.views().iter().all(|v| v.path != "people/marco.md"));
        assert_eq!(notes.changes("archive/marco.md", 10).len(), 1);
    }

    #[test]
    fn enough_goes_in_me_under_its_heading() {
        let notes = Notes::open(Some(&temp()));
        notes.keep_quiet("Gmail: A newsletter").unwrap();
        let me = notes.read("me.md", false).unwrap();
        let quiet = &me[me.find("## Don't tell me about").unwrap()..];
        assert!(quiet.lines().nth(1).unwrap().starts_with("- Gmail: A newsletter"), "{me}");
        assert!(notes.prompt(false, 12_000).contains("Gmail: A newsletter"));
    }

    #[test]
    fn writes_appends_indexes_and_commits() {
        let dir = temp();
        let notes = Notes::open(Some(&dir));
        assert!(!notes.migrated());
        notes.write("people/marco.md", MARCO, "met at the weekly").unwrap();
        notes.append("people/marco.md", "Started on the pricing project", "").unwrap();
        notes.append("people/marco.md", "Moved to Milan", "").unwrap();
        let text = notes.read("people/marco.md", false).unwrap();
        assert!(text.contains(&format!("updated: {}", today())));
        let timeline = &text[text.find("## Timeline").unwrap()..];
        assert!(timeline.find("pricing").unwrap() < timeline.find("Milan").unwrap());
        // The index, in the prompt and on disk.
        let prompt = notes.prompt(false, 12_000);
        assert!(prompt.contains("- [[people/marco]] (also Marco Rossi): Marco, head of design at Acme"));
        assert!(fs::read_to_string(dir.join("brain/index.md")).unwrap().contains("[[people/marco]]"));
        // Found by alias and body; every change a commit.
        assert!(notes.search("rossi", false).starts_with("people/marco.md"));
        assert!(notes.search("Milan", false).contains("Moved to Milan"));
        let history = notes.history(10);
        assert_eq!(history.len(), 4);
        notes.migrate(&[("topics/acme.md".into(), "---\ntype: topic\ndescription: Acme\n---\n".into())]).unwrap();
        assert!(notes.migrated() && notes.history(10).len() == 5);
        assert!(history[3].1.starts_with("The notes begin") && history[2].1 == "Add people/marco.md: met at the weekly");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn refuses_paths_outside_and_notes_without_a_description() {
        let dir = temp();
        let notes = Notes::open(Some(&dir));
        assert!(notes.write("../x.md", MARCO, "").is_err());
        assert!(notes.write("people/../../x.md", MARCO, "").is_err());
        assert!(notes.write("index.md", MARCO, "").is_err());
        assert!(notes.write("people/x.txt", MARCO, "").is_err());
        assert!(notes.write("people/x.md", "# X\nno frontmatter", "").is_err());
        assert!(notes.append("people/nobody.md", "x", "").is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn archiving_moves_and_private_stays_home() {
        let dir = temp();
        let notes = Notes::open(Some(&dir));
        notes.write("people/marco.md", MARCO, "").unwrap();
        notes.write("archive/marco.md", MARCO, "left Acme").unwrap();
        assert!(!dir.join("brain/people/marco.md").exists());
        assert!(!notes.prompt(false, 12_000).contains("people/marco"));
        assert!(notes.search("acme", false).starts_with("archive/marco.md"));
        notes.write("topics/health.md", "---\ntype: topic\ndescription: The user's back pain\nprivate: true\n---\nPhysio on Tuesdays.\n", "").unwrap();
        assert!(!notes.prompt(false, 12_000).contains("back pain") && notes.prompt(true, 12_000).contains("back pain"));
        assert!(notes.read("topics/health.md", false).is_err() && notes.search("physio", false).starts_with("no note"));
        let _ = fs::remove_dir_all(dir);
    }
}
