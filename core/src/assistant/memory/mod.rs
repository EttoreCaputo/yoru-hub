//! What Yoru knows: a graph in one SQLite file (`memory.db` in the data
//! folder), docs/memory.md.
//!
//! - **Episodes**: what it was learnt from (a conversation, a meeting, a
//!   `remember`, the Memory page), with date and source.
//! - **Entities**: the user, the people, places, projects and things in
//!   their life, each once, with aliases. A person mentioned without a name
//!   ("my mother-in-law") is a placeholder until the sleep can tell who it is.
//! - **Relations**: a few canonical ones between people ([`kin`]); words
//!   like mother-in-law are computed from them.
//! - **Facts**: one short line about an entity, in one of [`SECTIONS`], with
//!   the time it became true and, once contradicted, the time it stopped.
//!
//! The prompt gets the core ([`Memory::core`]: who the user is, what to tell
//! them and not) and, each turn, the facts found for what was said
//! ([`Memory::recalled`], [`search`]). The model reads and writes it by id
//! (`[f12]`), so a fact is changed or forgotten exactly.

pub mod kin;
mod search;
pub mod sync;
pub mod verdicts;
pub mod voice;

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use kin::{Edge, Step};

/// The sections a fact is in, by the name the model uses, and their title.
pub const SECTIONS: &[(&str, &str)] = &[
    ("about", "About you"),
    ("matters", "What matters to you"),
    ("tell", "Tell me about"),
    ("quiet", "Don't tell me about"),
    ("notes", "Notes"),
];

/// The core: the facts always in the prompt.
pub(crate) const CORE_SQL: &str = "f.section IN ('about', 'tell', 'quiet') OR f.importance >= 0.8";
/// The longest fact.
const MAX_LINE: usize = 300;
/// The longest line the board gets: its prompt has room for little.
const BOARD_LINE: usize = 160;
/// Facts found for a turn, at most.
const RECALLED: usize = 30;
/// Two facts this close in meaning are the same.
const SAME_COSINE: f32 = 0.93;

/// The vectors the memory searches with: EmbeddingGemma's
/// ([`crate::intent::IntentModel`]).
pub trait Embedder: Send + Sync {
    /// Unit vectors for `texts`, in order; `query`: they are what is
    /// searched with, else what is searched for.
    fn embed(&self, texts: &[String], query: bool) -> Result<Vec<Vec<f32>>, String>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Fact {
    pub id: i64,
    /// The entity it is about. Always stored; `None` only in a new fact
    /// from the Memory page, which is then the user's.
    pub about: Option<i64>,
    pub section: String,
    pub text: String,
    /// Unix ms: since when it is true.
    pub from: i64,
    /// Unix ms: when it stopped being true.
    pub to: Option<i64>,
    pub importance: f64,
    pub private: bool,
    pub episode: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entity {
    pub id: i64,
    pub name: String,
    /// person, place, project, organisation, app, topic, thing.
    pub kind: String,
    pub aliases: Vec<String>,
    pub summary: String,
    /// `f`, `m`, or none: for the words of kinship.
    pub gender: Option<String>,
    /// Someone mentioned without a name, to be merged once known.
    pub placeholder: bool,
    pub is_user: bool,
    /// What it is to the user ("your wife"), computed.
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Relation {
    pub id: i64,
    pub a: i64,
    pub kind: String,
    pub b: i64,
    pub from: i64,
    pub to: Option<i64>,
}

/// The whole graph, for the Memory page.
#[derive(Debug, Clone, Serialize)]
pub struct Graph {
    pub entities: Vec<Entity>,
    pub facts: Vec<Fact>,
    pub relations: Vec<Relation>,
    /// The entities whose voice Yoru knows.
    pub voices: Vec<i64>,
}

pub struct Memory {
    db: Mutex<Connection>,
    embedder: Mutex<Option<Arc<dyn Embedder>>>,
}

impl std::fmt::Debug for Memory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Memory").finish_non_exhaustive()
    }
}

pub(crate) fn now_ms() -> i64 {
    super::inbox::now_ms() as i64
}

pub(crate) fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub(crate) fn from_blob(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

/// One line, trimmed and bounded.
fn line(text: &str) -> String {
    text.trim().replace('\n', " ").chars().take(MAX_LINE).collect()
}

/// `f12` → 12.
pub fn fact_id(s: &str) -> Option<i64> {
    s.trim().trim_start_matches('[').trim_end_matches(']').trim_start_matches(['f', 'F']).parse().ok()
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT);
CREATE TABLE IF NOT EXISTS episodes(
    id INTEGER PRIMARY KEY, source TEXT NOT NULL, text TEXT NOT NULL DEFAULT '',
    started INTEGER NOT NULL, updated INTEGER NOT NULL, ended INTEGER, consolidated INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS entities(
    id INTEGER PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL DEFAULT 'person', aliases TEXT NOT NULL DEFAULT '[]',
    summary TEXT NOT NULL DEFAULT '', gender TEXT, placeholder INTEGER NOT NULL DEFAULT 0, is_user INTEGER NOT NULL DEFAULT 0,
    created INTEGER NOT NULL, voiceprint BLOB, voice TEXT);
CREATE TABLE IF NOT EXISTS relations(
    id INTEGER PRIMARY KEY, a INTEGER NOT NULL, kind TEXT NOT NULL, b INTEGER NOT NULL,
    valid_from INTEGER NOT NULL, valid_to INTEGER, episode INTEGER);
CREATE TABLE IF NOT EXISTS facts(
    id INTEGER PRIMARY KEY, about INTEGER, section TEXT NOT NULL DEFAULT 'notes', text TEXT NOT NULL,
    valid_from INTEGER NOT NULL, valid_to INTEGER, episode INTEGER, importance REAL NOT NULL DEFAULT 0.5,
    private INTEGER NOT NULL DEFAULT 0, embedding BLOB, created INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS facts_about ON facts(about);
CREATE VIRTUAL TABLE IF NOT EXISTS facts_fts USING fts5(text, content='facts', content_rowid='id', tokenize='unicode61 remove_diacritics 2');
CREATE TRIGGER IF NOT EXISTS facts_ai AFTER INSERT ON facts BEGIN
    INSERT INTO facts_fts(rowid, text) VALUES (new.id, new.text); END;
CREATE TRIGGER IF NOT EXISTS facts_ad AFTER DELETE ON facts BEGIN
    INSERT INTO facts_fts(facts_fts, rowid, text) VALUES ('delete', old.id, old.text); END;
CREATE TRIGGER IF NOT EXISTS facts_au AFTER UPDATE OF text ON facts BEGIN
    INSERT INTO facts_fts(facts_fts, rowid, text) VALUES ('delete', old.id, old.text);
    INSERT INTO facts_fts(rowid, text) VALUES (new.id, new.text); END;
CREATE TABLE IF NOT EXISTS verdicts(
    id INTEGER PRIMARY KEY, at INTEGER NOT NULL, source TEXT NOT NULL, by TEXT, title TEXT NOT NULL, useful INTEGER NOT NULL);
";

fn fact_row(r: &rusqlite::Row) -> rusqlite::Result<Fact> {
    Ok(Fact {
        id: r.get(0)?,
        about: r.get(1)?,
        section: r.get(2)?,
        text: r.get(3)?,
        from: r.get(4)?,
        to: r.get(5)?,
        importance: r.get(6)?,
        private: r.get(7)?,
        episode: r.get(8)?,
    })
}
const FACT_COLS: &str = "f.id, f.about, f.section, f.text, f.valid_from, f.valid_to, f.importance, f.private, f.episode";

fn entity_row(r: &rusqlite::Row) -> rusqlite::Result<Entity> {
    let aliases: String = r.get(3)?;
    Ok(Entity {
        id: r.get(0)?,
        name: r.get(1)?,
        kind: r.get(2)?,
        aliases: serde_json::from_str(&aliases).unwrap_or_default(),
        summary: r.get(4)?,
        gender: r.get(5)?,
        placeholder: r.get(6)?,
        is_user: r.get(7)?,
        label: None,
    })
}
const ENTITY_COLS: &str = "id, name, kind, aliases, summary, gender, placeholder, is_user";

impl Memory {
    /// The memory in `dir` (`memory.db`); `None`, or a file that won't open:
    /// in memory only.
    pub fn open(dir: Option<&Path>) -> Self {
        let conn = dir
            .and_then(|d| {
                fs::create_dir_all(d).ok()?;
                Connection::open(d.join("memory.db")).map_err(|e| eprintln!("memory.db: {e}")).ok()
            })
            .unwrap_or_else(|| Connection::open_in_memory().expect("an SQLite in memory"));
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
        if let Err(e) = conn.execute_batch(SCHEMA) {
            eprintln!("memory.db schema: {e}");
        }
        if let Err(e) = sync::install(&conn, dir.map(|d| crate::sync::device(Some(d))).as_deref()) {
            eprintln!("memory.db sync: {e}");
        }
        let memory = Self { db: Mutex::new(conn), embedder: Mutex::default() };
        memory.user();
        memory
    }

    pub fn set_embedder(&self, embedder: Option<Arc<dyn Embedder>>) {
        *self.embedder.lock().unwrap() = embedder;
    }

    pub fn has_embedder(&self) -> bool {
        self.embedder.lock().unwrap().is_some()
    }

    fn embed(&self, texts: &[String], query: bool) -> Option<Vec<Vec<f32>>> {
        let embedder = self.embedder.lock().unwrap().clone()?;
        embedder.embed(texts, query).map_err(|e| eprintln!("memory: embedding: {e}")).ok()
    }

    /// Give a vector to the facts without one (made before the embedder
    /// ran): a few dozen at a time. How many got one.
    pub fn embed_missing(&self) -> usize {
        if !self.has_embedder() {
            return 0;
        }
        let missing: Vec<(i64, String)> = {
            let db = self.db.lock().unwrap();
            let Ok(mut stmt) = db.prepare("SELECT id, text FROM facts WHERE embedding IS NULL AND valid_to IS NULL LIMIT 32") else { return 0 };
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map(|rows| rows.flatten().collect()).unwrap_or_default()
        };
        if missing.is_empty() {
            return 0;
        }
        let texts: Vec<String> = missing.iter().map(|(_, t)| t.clone()).collect();
        let Some(vectors) = self.embed(&texts, false) else { return 0 };
        let db = self.db.lock().unwrap();
        for ((id, _), v) in missing.iter().zip(vectors) {
            let _ = db.execute("UPDATE facts SET embedding = ?1 WHERE id = ?2", params![to_blob(&v), id]);
        }
        missing.len()
    }

    // ---- entities ----

    /// The user's node, made the first time.
    pub fn user(&self) -> i64 {
        let db = self.db.lock().unwrap();
        if let Ok(id) = db.query_row("SELECT id FROM entities WHERE is_user = 1", [], |r| r.get(0)) {
            return id;
        }
        let _ = db.execute("INSERT INTO entities(name, kind, is_user, created) VALUES ('User', 'person', 1, ?1)", [now_ms()]);
        let id = db.last_insert_rowid();
        // A blank User loses to the one another Mac already knows (docs/icloud.md).
        let _ = db.execute_batch(
            "UPDATE entities SET changed = 1 WHERE is_user = 1;
             UPDATE sync_outbox SET changed = 1 WHERE kind = 'entity' AND uid = 'user';",
        );
        id
    }

    fn edges(db: &Connection) -> Vec<Edge> {
        let Ok(mut stmt) = db.prepare("SELECT a, kind, b FROM relations WHERE valid_to IS NULL") else { return Vec::new() };
        stmt.query_map([], |r| Ok(Edge { a: r.get(0)?, kind: r.get(1)?, b: r.get(2)? })).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    fn all_entities(db: &Connection) -> Vec<Entity> {
        let Ok(mut stmt) = db.prepare(&format!("SELECT {ENTITY_COLS} FROM entities ORDER BY is_user DESC, name")) else { return Vec::new() };
        stmt.query_map([], entity_row).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    /// Every entity, with what it is to `from` (the user when `None`).
    pub fn entities(&self) -> Vec<Entity> {
        let db = self.db.lock().unwrap();
        Self::labelled(&db, None)
    }

    fn labelled(db: &Connection, from: Option<i64>) -> Vec<Entity> {
        let mut all = Self::all_entities(db);
        let edges = Self::edges(db);
        let user = all.iter().find(|e| e.is_user).map(|e| e.id).unwrap_or(0);
        let from = from.unwrap_or(user);
        let owner = if from == user { "your".to_string() } else { all.iter().find(|e| e.id == from).map(|e| format!("{}'s", e.name)).unwrap_or_default() };
        let genders: Vec<(i64, Option<String>)> = all.iter().map(|e| (e.id, e.gender.clone())).collect();
        let gender = |id: i64| genders.iter().find(|(i, _)| *i == id).and_then(|(_, g)| g.clone());
        for e in &mut all {
            e.label = kin::label(&edges, from, e.id, &owner, &gender);
        }
        all
    }

    pub fn entity(&self, id: i64) -> Option<Entity> {
        self.entities().into_iter().find(|e| e.id == id)
    }

    /// The entity called `name` (its name, an alias, or what it is to the
    /// user: "wife", "my mother-in-law").
    pub fn find(&self, name: &str) -> Option<i64> {
        let db = self.db.lock().unwrap();
        Self::find_in(&db, name)
    }

    fn find_in(db: &Connection, name: &str) -> Option<i64> {
        let wanted = name.trim().to_lowercase();
        if wanted.is_empty() {
            return None;
        }
        let all = Self::labelled(db, None);
        if matches!(wanted.as_str(), "user" | "the user" | "me" | "you" | "io" | "tu") {
            return all.iter().find(|e| e.is_user).map(|e| e.id);
        }
        let named = all.iter().find(|e| e.name.to_lowercase() == wanted || e.aliases.iter().any(|a| a.to_lowercase() == wanted));
        if let Some(e) = named {
            return Some(e.id);
        }
        let user = all.iter().find(|e| e.is_user)?.id;
        let (steps, gender) = kin::parse(&wanted)?;
        Self::follow(db, user, &steps, gender)
    }

    /// The person reached from `from` along `steps` (the last one of
    /// `gender`, when known).
    fn follow(db: &Connection, from: i64, steps: &[Step], gender: Option<&str>) -> Option<i64> {
        let edges = Self::edges(db);
        let all = Self::all_entities(db);
        let (edges, all) = (&edges, &all);
        let mut at = vec![from];
        for (i, step) in steps.iter().enumerate() {
            let last = i + 1 == steps.len();
            at = at
                .iter()
                .flat_map(|&p| {
                    all.iter().filter(move |e| e.id != p).filter(move |e| kin::path(edges, p, e.id).is_some_and(|w| w.len() == 1 && w[0].1 == *step)).map(|e| e.id)
                })
                .filter(|id| !last || gender.is_none() || all.iter().any(|e| e.id == *id && (e.gender.is_none() || e.gender.as_deref() == gender)))
                .collect();
        }
        at.first().copied()
    }

    /// The entity called `name`, made if there is none: a kinship word
    /// ("my wife") makes the people on the way, as placeholders.
    pub fn resolve(&self, name: &str, kind: Option<&str>) -> Option<i64> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        if let Some(id) = self.find(name) {
            return Some(id);
        }
        // A role a cloud brain was given in place of a name ("your friend").
        let low = name.to_lowercase();
        let roles: Vec<i64> = self.entities().into_iter().filter(|e| e.label.as_deref().is_some_and(|l| l.to_lowercase() == low)).map(|e| e.id).collect();
        if let [id] = roles[..] {
            return Some(id);
        }
        let db = self.db.lock().unwrap();
        if let Some((steps, gender)) = kin::parse(name) {
            let user = db.query_row("SELECT id FROM entities WHERE is_user = 1", [], |r| r.get(0)).ok()?;
            let mut at: i64 = user;
            for (i, step) in steps.iter().enumerate() {
                let last = i + 1 == steps.len();
                if let Some(next) = Self::follow(&db, at, &[*step], if last { gender } else { None }) {
                    at = next;
                    continue;
                }
                // Someone not named yet: "your wife", "your wife's mother".
                let word = if last { name.trim_start_matches("my ").to_string() } else { format!("{step:?}").to_lowercase() };
                let _ = db.execute(
                    "INSERT INTO entities(name, kind, gender, placeholder, created) VALUES (?1, 'person', ?2, 1, ?3)",
                    params![word, if last { gender } else { None }, now_ms()],
                );
                let next = db.last_insert_rowid();
                let _ = Self::relate_in(&db, at, step_kind(*step), next, None);
                at = next;
            }
            return Some(at);
        }
        let kind = kind.unwrap_or(if name.chars().next().is_some_and(char::is_uppercase) { "person" } else { "topic" });
        let _ = db.execute("INSERT INTO entities(name, kind, created) VALUES (?1, ?2, ?3)", params![name, kind, now_ms()]);
        Some(db.last_insert_rowid())
    }

    /// `a` is `kind` of `b` (`child` and `parent` from either side).
    pub fn relate(&self, a: i64, kind: &str, b: i64, episode: Option<i64>) -> Result<String, String> {
        if a == b {
            return Err("a person isn't related to themself".into());
        }
        let db = self.db.lock().unwrap();
        Self::relate_in(&db, a, kind, b, episode)
    }

    fn relate_in(db: &Connection, a: i64, kind: &str, b: i64, episode: Option<i64>) -> Result<String, String> {
        let (kind, swap) = Step::stored(kind).ok_or_else(|| format!("unknown relation {kind}: one of {}", kin::KINDS.join(", ")))?;
        let (a, b) = if swap { (b, a) } else { (a, b) };
        let known: bool = db
            .query_row(
                "SELECT 1 FROM relations WHERE kind = ?1 AND valid_to IS NULL AND ((a = ?2 AND b = ?3) OR (?1 != 'parent' AND a = ?3 AND b = ?2))",
                params![kind, a, b],
                |_| Ok(true),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .unwrap_or(false);
        if known {
            return Ok("already known".into());
        }
        db.execute("INSERT INTO relations(a, kind, b, valid_from, episode) VALUES (?1, ?2, ?3, ?4, ?5)", params![a, kind, b, now_ms(), episode])
            .map_err(|e| e.to_string())?;
        Ok("related".into())
    }

    /// The `relate` tool: `person` is `relation` of `of` (the user when
    /// `None`). A placeholder already in that place ("your wife") becomes
    /// them.
    pub fn relate_tool(&self, person: &str, relation: &str, of: Option<&str>, gender: Option<&str>) -> Result<String, String> {
        let step = match relation {
            "parent" => Step::Parent,
            "child" => Step::Child,
            "partner" => Step::Partner,
            "sibling" => Step::Sibling,
            "friend" => Step::Friend,
            "colleague" => Step::Colleague,
            other => return Err(format!("unknown relation {other}: one of {}", kin::KINDS.join(", "))),
        };
        let gender = gender.filter(|g| matches!(*g, "f" | "m"));
        let b = match of.map(str::trim).filter(|o| !o.is_empty()) {
            Some(name) => self.resolve(name, Some("person")).ok_or("say whose")?,
            None => self.user(),
        };
        let a = self.resolve(person, Some("person")).ok_or("say who")?;
        if let Some(g) = gender
            && let Some(mut e) = self.entity(a)
            && e.gender.is_none()
        {
            e.gender = Some(g.to_string());
            self.put_entity(&e)?;
        }
        let waiting = {
            let db = self.db.lock().unwrap();
            Self::follow(&db, b, &[step], gender)
        };
        if let Some(p) = waiting.filter(|p| *p != a)
            && self.entity(p).is_some_and(|e| e.placeholder)
        {
            self.merge(a, p)?;
            return Ok(format!("[e{a}] {person} is the one you knew as their {relation}"));
        }
        self.relate(a, relation, b, None).map(|r| format!("[e{a}] {person}: {r}"))
    }

    /// Change an entity (the Memory page, the sleep).
    pub fn put_entity(&self, e: &Entity) -> Result<(), String> {
        let db = self.db.lock().unwrap();
        db.execute(
            "UPDATE entities SET name = ?1, kind = ?2, aliases = ?3, summary = ?4, gender = ?5, placeholder = ?6 WHERE id = ?7",
            params![line(&e.name), e.kind, serde_json::to_string(&e.aliases).unwrap_or_default(), line(&e.summary), e.gender, e.placeholder, e.id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Remove an entity with its facts and relations (the user's node stays).
    pub fn remove_entity(&self, id: i64) -> Result<(), String> {
        let db = self.db.lock().unwrap();
        db.execute("DELETE FROM facts WHERE about = ?1", [id]).map_err(|e| e.to_string())?;
        db.execute("DELETE FROM relations WHERE a = ?1 OR b = ?1", [id]).map_err(|e| e.to_string())?;
        db.execute("DELETE FROM entities WHERE id = ?1 AND is_user = 0", [id]).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// `drop` is `keep` under another name (a placeholder now known): one
    /// node, with both's facts, relations and names.
    pub fn merge(&self, keep: i64, drop: i64) -> Result<(), String> {
        if keep == drop {
            return Ok(());
        }
        let all = self.entities();
        let (Some(k), Some(d)) = (all.iter().find(|e| e.id == keep).cloned(), all.iter().find(|e| e.id == drop).cloned()) else {
            return Err("no such entity".into());
        };
        let (keep, drop, mut k, d) = if d.is_user { (drop, keep, d, k) } else { (keep, drop, k, d) };
        // A placeholder takes the name it was waiting for.
        if k.placeholder && !d.placeholder {
            k.aliases.push(k.name.clone());
            k.name = d.name.clone();
            k.placeholder = false;
        } else {
            k.aliases.push(d.name.clone());
        }
        k.aliases.extend(d.aliases.iter().cloned());
        let mut seen = HashSet::new();
        k.aliases.retain(|a| a.to_lowercase() != k.name.to_lowercase() && seen.insert(a.to_lowercase()));
        k.gender = k.gender.or(d.gender);
        if k.summary.is_empty() {
            k.summary = d.summary;
        }
        self.put_entity(&k)?;
        let db = self.db.lock().unwrap();
        db.execute("UPDATE facts SET about = ?1 WHERE about = ?2", params![keep, drop]).map_err(|e| e.to_string())?;
        db.execute("UPDATE relations SET a = ?1 WHERE a = ?2", params![keep, drop]).map_err(|e| e.to_string())?;
        db.execute("UPDATE relations SET b = ?1 WHERE b = ?2", params![keep, drop]).map_err(|e| e.to_string())?;
        db.execute("DELETE FROM relations WHERE a = b", []).map_err(|e| e.to_string())?;
        // The same relation twice: the newer goes.
        db.execute(
            "DELETE FROM relations WHERE id IN (SELECT r2.id FROM relations r1 JOIN relations r2 ON r1.kind = r2.kind AND r1.id < r2.id \
             AND ((r1.a = r2.a AND r1.b = r2.b) OR (r1.kind != 'parent' AND r1.a = r2.b AND r1.b = r2.a)))",
            [],
        )
        .map_err(|e| e.to_string())?;
        if let (Some(kv), Some(dv)) = (Self::blob(&db, keep, "voiceprint"), Self::blob(&db, drop, "voiceprint")) {
            let mean: Vec<f32> = kv.iter().zip(&dv).map(|(a, b)| (a + b) / 2.0).collect();
            let _ = db.execute("UPDATE entities SET voiceprint = ?1 WHERE id = ?2", params![to_blob(&voice::unit(mean)), keep]);
        } else if Self::blob(&db, keep, "voiceprint").is_none() {
            let _ = db.execute("UPDATE entities SET voiceprint = (SELECT voiceprint FROM entities WHERE id = ?2) WHERE id = ?1", params![keep, drop]);
        }
        db.execute("DELETE FROM entities WHERE id = ?1", [drop]).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn blob(db: &Connection, id: i64, column: &str) -> Option<Vec<f32>> {
        db.query_row(&format!("SELECT {column} FROM entities WHERE id = ?1"), [id], |r| r.get::<_, Option<Vec<u8>>>(0)).ok().flatten().map(|b| from_blob(&b))
    }

    /// A JSON column of an entity (its voice).
    pub(crate) fn json<T: serde::de::DeserializeOwned + Default>(&self, id: i64, column: &str) -> T {
        let db = self.db.lock().unwrap();
        db.query_row(&format!("SELECT {column} FROM entities WHERE id = ?1"), [id], |r| r.get::<_, Option<String>>(0))
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub(crate) fn set_json<T: Serialize>(&self, id: i64, column: &str, value: &T) {
        let db = self.db.lock().unwrap();
        let _ = db.execute(&format!("UPDATE entities SET {column} = ?1 WHERE id = ?2"), params![serde_json::to_string(value).unwrap_or_default(), id]);
    }

    // ---- facts ----

    /// Insert a fact as it is. Its id.
    pub(crate) fn insert_fact(&self, about: i64, section: &str, text: &str, importance: f64, private: bool, episode: Option<i64>) -> Result<i64, String> {
        let text = line(text);
        if text.is_empty() {
            return Err("nothing to remember".into());
        }
        let section = if SECTIONS.iter().any(|(k, _)| *k == section) { section } else { "notes" };
        let vector = self.embed(std::slice::from_ref(&text), false).and_then(|mut v| v.pop());
        let db = self.db.lock().unwrap();
        let now = now_ms();
        db.execute(
            "INSERT INTO facts(about, section, text, valid_from, episode, importance, private, embedding, created) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?4)",
            params![about, section, text, now, episode, importance.clamp(0.0, 1.0), private, vector.as_deref().map(to_blob)],
        )
        .map_err(|e| e.to_string())?;
        Ok(db.last_insert_rowid())
    }

    /// The open fact that already says `text`: the same words, or the same
    /// meaning.
    pub(crate) fn same_as(&self, text: &str) -> Option<i64> {
        let text = line(text);
        {
            let db = self.db.lock().unwrap();
            if let Ok(id) = db.query_row("SELECT id FROM facts WHERE valid_to IS NULL AND lower(text) = lower(?1)", [&text], |r| r.get(0)) {
                return Some(id);
            }
        }
        let v = self.embed(&[text], false)?.pop()?;
        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare("SELECT id, embedding FROM facts WHERE valid_to IS NULL AND embedding IS NOT NULL").ok()?;
        let best = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))
            .ok()?
            .flatten()
            .map(|(id, b)| (id, search::cosine(&v, &from_blob(&b))))
            .max_by(|a, b| a.1.total_cmp(&b.1))?;
        (best.1 >= SAME_COSINE).then_some(best.0)
    }

    /// Learn `text` about `about` (a name, a kinship word; the user when
    /// `None`), in `section`. Says the fact's id, or the one that already
    /// said it.
    pub fn remember(&self, text: &str, about: Option<&str>, section: &str, important: bool, private: bool, episode: Option<i64>) -> Result<String, String> {
        if SECTIONS.iter().all(|(k, _)| *k != section) {
            return Err(format!("unknown section {section}"));
        }
        if line(text).is_empty() {
            return Err("nothing to remember".into());
        }
        if let Some(id) = self.same_as(text) {
            return Ok(format!("already known as [f{id}]"));
        }
        let about = about.and_then(|name| self.resolve(name, None)).unwrap_or_else(|| self.user());
        let episode = episode.or_else(|| Some(self.add_episode("remember", text)));
        let id = self.insert_fact(about, section, text, if important { 0.9 } else { 0.5 }, private, episode)?;
        Ok(format!("remembered as [f{id}]"))
    }

    pub fn fact(&self, id: i64) -> Option<Fact> {
        let db = self.db.lock().unwrap();
        db.query_row(&format!("SELECT {FACT_COLS} FROM facts f WHERE f.id = ?1"), [id], fact_row).ok()
    }

    /// The open facts that contain `needle`.
    pub fn matching(&self, needle: &str) -> Vec<Fact> {
        let db = self.db.lock().unwrap();
        let Ok(mut stmt) = db.prepare(&format!("SELECT {FACT_COLS} FROM facts f WHERE f.valid_to IS NULL AND instr(lower(f.text), lower(?1)) > 0")) else {
            return Vec::new();
        };
        stmt.query_map([needle.trim()], fact_row).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    /// The fact `which` names: its id (`f12`), or the one open fact with
    /// that text in it.
    fn which(&self, which: &str) -> Result<Fact, String> {
        if let Some(id) = fact_id(which) {
            return self.fact(id).filter(|f| f.to.is_none()).ok_or_else(|| format!("no open fact [f{id}]"));
        }
        if which.trim().chars().count() < 3 {
            return Err("say which fact, by its id ([f12])".into());
        }
        let mut found = self.matching(which);
        match found.len() {
            0 => Err("nothing like that is in the memory: use remember".into()),
            1 => Ok(found.remove(0)),
            _ => Err(format!("{} facts match, say which by id: {}", found.len(), listed(&found))),
        }
    }

    /// `which` is no longer true and `text` is: the old fact is closed (the
    /// graph keeps what was true before), the new one takes its place.
    pub fn revise(&self, which: &str, text: &str) -> Result<String, String> {
        let old = self.which(which)?;
        if line(text).is_empty() {
            return Err("nothing to write: use forget to remove the fact".into());
        }
        self.close(old.id);
        let id = self.insert_fact(old.about.unwrap_or_else(|| self.user()), &old.section, text, old.importance, old.private, old.episode)?;
        Ok(format!("[f{}] closed, now [f{id}]", old.id))
    }

    /// The fact stopped being true now.
    pub fn close(&self, id: i64) {
        let db = self.db.lock().unwrap();
        let _ = db.execute("UPDATE facts SET valid_to = ?1, embedding = NULL WHERE id = ?2 AND valid_to IS NULL", params![now_ms(), id]);
    }

    /// Remove the facts `ids` for good.
    pub fn forget(&self, ids: &[i64]) -> Result<String, String> {
        let db = self.db.lock().unwrap();
        let mut n = 0;
        for id in ids {
            n += db.execute("DELETE FROM facts WHERE id = ?1", [id]).map_err(|e| e.to_string())?;
        }
        if n == 0 {
            return Ok("nothing like that was in the memory".into());
        }
        Ok(format!("forgot {n} fact(s)"))
    }

    /// The `forget` tool: by ids, or by text, which removes only a fact it
    /// matches alone and otherwise shows the candidates.
    pub fn forget_tool(&self, ids: &[i64], text: &str) -> Result<String, String> {
        if !ids.is_empty() {
            return self.forget(ids);
        }
        if text.trim().chars().count() < 3 {
            return Err("say what to forget, by id ([f12]) or with a few words".into());
        }
        let found = self.matching(text);
        match found.len() {
            0 => Ok("nothing like that was in the memory".into()),
            1 => self.forget(&[found[0].id]),
            n => Ok(format!("{n} facts match, nothing removed yet: call forget again with the ids of those to remove: {}", listed(&found))),
        }
    }

    /// A fact from the Memory page: new (`id` 0) or changed in place (the
    /// user correcting it).
    pub fn put_fact(&self, f: &Fact) -> Result<i64, String> {
        if f.id == 0 {
            let episode = self.add_episode("page", &f.text);
            return self.insert_fact(f.about.unwrap_or_else(|| self.user()), &f.section, &f.text, f.importance, f.private, Some(episode));
        }
        let text = line(&f.text);
        let about = f.about.unwrap_or_else(|| self.user());
        let vector = self.embed(std::slice::from_ref(&text), false).and_then(|mut v| v.pop());
        let db = self.db.lock().unwrap();
        db.execute(
            "UPDATE facts SET about = ?1, section = ?2, text = ?3, importance = ?4, private = ?5, embedding = ?6 WHERE id = ?7",
            params![about, f.section, text, f.importance.clamp(0.0, 1.0), f.private, vector.as_deref().map(to_blob), f.id],
        )
        .map_err(|e| e.to_string())?;
        Ok(f.id)
    }

    /// The whole graph, for the Memory page: closed facts too.
    pub fn graph(&self) -> Graph {
        let entities = self.entities();
        let db = self.db.lock().unwrap();
        let facts = db
            .prepare(&format!("SELECT {FACT_COLS} FROM facts f ORDER BY f.valid_to IS NOT NULL, f.section, f.importance DESC, f.valid_from"))
            .and_then(|mut s| s.query_map([], fact_row).map(|rows| rows.flatten().collect()))
            .unwrap_or_default();
        let relations = db
            .prepare("SELECT id, a, kind, b, valid_from, valid_to FROM relations WHERE valid_to IS NULL")
            .and_then(|mut s| {
                s.query_map([], |r| Ok(Relation { id: r.get(0)?, a: r.get(1)?, kind: r.get(2)?, b: r.get(3)?, from: r.get(4)?, to: r.get(5)? }))
                    .map(|rows| rows.flatten().collect())
            })
            .unwrap_or_default();
        let voices = db
            .prepare("SELECT id FROM entities WHERE voiceprint IS NOT NULL")
            .and_then(|mut s| s.query_map([], |r| r.get(0)).map(|rows| rows.flatten().collect()))
            .unwrap_or_default();
        Graph { entities, facts, relations, voices }
    }

    /// Remove a relation (the Memory page).
    pub fn unrelate(&self, id: i64) -> Result<(), String> {
        let db = self.db.lock().unwrap();
        db.execute("DELETE FROM relations WHERE id = ?1", [id]).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// The open facts, as the old Markdown file: for an export, and to read
    /// at a glance.
    pub fn text(&self) -> String {
        let graph = self.graph();
        let mut out = String::from("# What Yoru knows about you\n");
        for (key, title) in SECTIONS {
            out.push_str(&format!("\n## {title}\n\n"));
            for f in graph.facts.iter().filter(|f| f.to.is_none() && f.section == *key) {
                out.push_str(&format!("- {}\n", f.text));
            }
        }
        out
    }

    // ---- the prompt ----

    /// The core, always in the prompt: who the user is, what to tell them
    /// and not, what matters most. `cloud`: private facts stay home.
    pub fn core(&self, max: usize, cloud: bool) -> String {
        let user = self.entity(self.user());
        let db = self.db.lock().unwrap();
        let private = if cloud { " AND f.private = 0" } else { "" };
        let Ok(mut stmt) = db.prepare(&format!(
            "SELECT {FACT_COLS} FROM facts f WHERE f.valid_to IS NULL AND ({CORE_SQL}){private} ORDER BY f.importance DESC, f.valid_from DESC"
        )) else {
            return String::new();
        };
        let facts: Vec<Fact> = stmt.query_map([], fact_row).map(|rows| rows.flatten().collect()).unwrap_or_default();
        drop(stmt);
        drop(db);
        let mut out = String::new();
        if let Some(u) = user.filter(|u| u.name != "User") {
            out.push_str(&format!("The user is {}{}.\n", u.name, if u.summary.is_empty() { String::new() } else { format!(": {}", u.summary) }));
        }
        for (key, title) in SECTIONS {
            let lines: Vec<String> = facts.iter().filter(|f| f.section == *key).map(|f| format!("- [f{}] {}", f.id, f.text)).collect();
            if lines.is_empty() {
                continue;
            }
            let block = format!("## {title}\n{}\n", lines.join("\n"));
            if out.len() + block.len() > max {
                // What fits of it, most important first.
                let mut part = format!("## {title}\n");
                for l in lines {
                    if out.len() + part.len() + l.len() + 1 > max {
                        break;
                    }
                    part.push_str(&l);
                    part.push('\n');
                }
                out.push_str(&part);
                break;
            }
            out.push_str(&block);
        }
        let out = out.trim_end().to_string();
        if cloud { self.veil(&out, &[]) } else { out }
    }

    /// What the board knows when it talks without the computer: the graph in
    /// short, within `max` characters, whole lines only. Who the user is and
    /// their core facts, then the people and things around them, most known
    /// first, each with what matters most of it. A cloud brain reads it: no
    /// private facts, and people with a role go by it.
    pub fn board(&self, max: usize) -> String {
        let graph = self.graph();
        let user = graph.entities.iter().find(|e| e.is_user);
        let uid = user.map(|u| u.id);
        // Open and public; the graph has them by section, most important first.
        let open: Vec<&Fact> = graph.facts.iter().filter(|f| f.to.is_none() && !f.private).collect();
        let veils = self.veils(&[]);
        let veiled: Vec<i64> = veils.iter().map(|(id, _, _)| *id).collect();
        // Each line by role before it is measured: a role is longer than a name.
        let line = |text: String| clip(&hide(&text, &veils), BOARD_LINE);

        // The others, by how much Yoru knows of them.
        let mut others: Vec<(f64, String)> = graph
            .entities
            .iter()
            .filter(|e| !e.is_user && (!e.placeholder || e.label.is_some()))
            .filter_map(|e| {
                let mut facts: Vec<&&Fact> = open.iter().filter(|f| f.about == Some(e.id)).collect();
                facts.sort_by(|a, b| b.importance.total_cmp(&a.importance));
                if facts.is_empty() && e.label.is_none() && e.summary.is_empty() {
                    return None;
                }
                let score = facts.iter().map(|f| f.importance).sum::<f64>() + if e.label.is_some() { 1.0 } else { 0.0 };
                let head = match (&e.label, veiled.contains(&e.id) || e.placeholder) {
                    (Some(label), true) => label.clone(),
                    (Some(label), false) => format!("{}, {label}", e.name),
                    (None, _) => format!("{} ({})", e.name, e.kind),
                };
                let mut what: Vec<&str> = Vec::new();
                if !e.summary.is_empty() {
                    what.push(&e.summary);
                }
                what.extend(facts.iter().take(2).map(|f| f.text.as_str()));
                Some((score, line(if what.is_empty() { format!("- {head}") } else { format!("- {head}: {}", what.join("; ")) })))
            })
            .collect();
        others.sort_by(|a, b| b.0.total_cmp(&a.0));
        let others: Vec<String> = others.into_iter().map(|(_, l)| l).collect();

        // The user's own core first, leaving the people a third at least.
        let theirs = others.iter().map(|l| l.len() + 1).sum::<usize>();
        let room = max.saturating_sub(theirs.min(max / 3));
        let mut out = String::new();
        if let Some(u) = user.filter(|u| u.name != "User") {
            let summary = if u.summary.is_empty() { String::new() } else { format!(": {}", u.summary) };
            out.push_str(&line(format!("The user is {}{summary}.", u.name)));
            out.push('\n');
        }
        let mine = |f: &&&Fact| f.about == uid;
        let core = |f: &&&Fact| matches!(f.section.as_str(), "about" | "tell" | "quiet") || f.importance >= 0.8;
        for (key, title) in SECTIONS {
            let lines: Vec<String> = open.iter().filter(mine).filter(core).filter(|f| f.section == *key).map(|f| line(format!("- {}", f.text))).collect();
            let mut part = format!("{title}:\n");
            let mut any = false;
            for l in lines {
                if out.len() + part.len() + l.len() + 1 > room {
                    break;
                }
                part.push_str(&l);
                part.push('\n');
                any = true;
            }
            if any {
                out.push_str(&part);
            }
        }
        let title = "People and things in their life:\n";
        let mut part = String::from(title);
        for l in others {
            if out.len() + part.len() + l.len() + 1 > max {
                break;
            }
            part.push_str(&l);
            part.push('\n');
        }
        if part.len() > title.len() {
            out.push_str(&part);
        }
        out.trim_end().to_string()
    }

    /// The people who have a role for the user that no one else has ("your
    /// wife", "your friend" when there is one), with their names and aliases:
    /// a cloud brain reads them by role (docs/memory.md, Privacy). `keep`:
    /// the people the message itself names, who keep their names.
    fn veils(&self, keep: &[i64]) -> Vec<(i64, Vec<String>, String)> {
        let all = self.entities();
        all.iter()
            .filter(|e| e.kind == "person" && !e.is_user && !e.placeholder && !keep.contains(&e.id))
            .filter_map(|e| {
                let label = e.label.clone()?;
                if all.iter().filter(|o| o.label.as_deref() == Some(label.as_str())).count() > 1 {
                    return None;
                }
                // Aliases that are kin words ("wife") already say the role.
                let names = std::iter::once(&e.name).chain(e.aliases.iter().filter(|a| kin::parse(a).is_none())).filter(|n| n.chars().count() >= 3).cloned().collect();
                Some((e.id, names, label))
            })
            .collect()
    }

    /// `text` with the names of the people who have a role for the user
    /// replaced by it, but those of `keep`: what a cloud brain reads.
    pub fn veil(&self, text: &str, keep: &[i64]) -> String {
        hide(text, &self.veils(keep))
    }

    /// The entities `text` calls by name or alias (not as kin): those a
    /// cloud brain may read by name.
    pub fn called_in(&self, text: &str) -> Vec<i64> {
        let low = text.to_lowercase();
        let words: Vec<&str> = low.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '\'')).filter(|w| !w.is_empty()).collect();
        let has = |name: &str| {
            let name = name.to_lowercase();
            name.chars().count() >= 3 && if name.contains(' ') { low.contains(&name) } else { words.contains(&name.as_str()) }
        };
        // An alias that is a kin word ("wife") names a role, not the person.
        self.entities().into_iter().filter(|e| !e.is_user && (has(&e.name) || e.aliases.iter().any(|a| kin::parse(a).is_none() && has(a)))).map(|e| e.id).collect()
    }

    /// The entities `text` names: by name or alias, or as kin ("my wife").
    pub fn named_in(&self, text: &str) -> Vec<i64> {
        let mut out = self.called_in(text);
        let low = text.to_lowercase();
        let words: Vec<&str> = low.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '\'')).filter(|w| !w.is_empty()).collect();
        let db = self.db.lock().unwrap();
        let user = db.query_row("SELECT id FROM entities WHERE is_user = 1", [], |r| r.get(0)).unwrap_or(0);
        for w in &words {
            if let Some((steps, gender)) = kin::parse(w)
                && let Some(id) = Self::follow(&db, user, &steps, gender)
                && !out.contains(&id)
            {
                out.push(id);
            }
        }
        out
    }

    /// The facts found for `query` (and about `speaker`), with the people
    /// they are about, within `max` characters: what the turn adds to the
    /// prompt. Empty when nothing is found.
    pub fn recalled(&self, query: &str, speaker: Option<i64>, max: usize, cloud: bool) -> String {
        let mut named = self.named_in(query);
        named.extend(speaker.filter(|s| *s != self.user()));
        let vector = self.embed(&[query.to_string()], true).and_then(|mut v| v.pop());
        let ids = {
            let db = self.db.lock().unwrap();
            search::run(&db, &search::Query { text: query, vector, named: &named, closed: false, skip_core: true, now_ms: now_ms() })
        };
        self.render(&ids.iter().map(|(id, _)| *id).take(RECALLED).collect::<Vec<_>>(), &named, &self.called_in(query), max, cloud)
    }

    /// The `recall` tool: a search on purpose, closed facts too, with dates.
    pub fn recall(&self, query: &str, cloud: bool) -> String {
        let named = self.named_in(query);
        let vector = self.embed(&[query.to_string()], true).and_then(|mut v| v.pop());
        let ids = {
            let db = self.db.lock().unwrap();
            search::run(&db, &search::Query { text: query, vector, named: &named, closed: true, skip_core: false, now_ms: now_ms() })
        };
        let out = self.render(&ids.iter().map(|(id, _)| *id).take(40).collect::<Vec<_>>(), &named, &self.called_in(query), 8000, cloud);
        if out.is_empty() { "nothing about that in the memory".into() } else { out }
    }

    /// Facts `ids` (best first) and the people they're about. `cloud`: the
    /// private facts stay home and the people with a role are called by it,
    /// but those `keep` (the message calls them by name).
    fn render(&self, ids: &[i64], named: &[i64], keep: &[i64], max: usize, cloud: bool) -> String {
        let facts: Vec<Fact> = ids.iter().filter_map(|id| self.fact(*id)).filter(|f| !(cloud && f.private)).collect();
        let entities = self.entities();
        let mut people: Vec<i64> = named.to_vec();
        for f in &facts {
            if let Some(a) = f.about
                && !people.contains(&a)
            {
                people.push(a);
            }
        }
        let mut out = String::new();
        let veiled: Vec<i64> = if cloud { self.veils(keep).into_iter().map(|(id, _, _)| id).collect() } else { Vec::new() };
        let who: Vec<String> = people
            .iter()
            .filter_map(|id| entities.iter().find(|e| e.id == *id && !e.is_user))
            .map(|e| {
                let hidden = veiled.contains(&e.id);
                let label = if hidden { String::new() } else { e.label.clone().unwrap_or_default() };
                let what = [label, e.summary.clone(), if e.placeholder { "name not known yet".into() } else { String::new() }]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("; ");
                let name = if hidden { e.label.clone().unwrap_or_default() } else { e.name.clone() };
                format!("- [e{}] {name}{}", e.id, if what.is_empty() { String::new() } else { format!(": {what}") })
            })
            .collect();
        if !who.is_empty() {
            out.push_str(&format!("People and things:\n{}\n", who.join("\n")));
        }
        let mut lines = Vec::new();
        let mut used = out.len() + 7;
        for f in &facts {
            let when = match f.to {
                Some(to) => format!(" (true from {} to {})", day(f.from), day(to)),
                None => String::new(),
            };
            let l = format!("- [f{}] {}{when}", f.id, f.text);
            if used + l.len() + 1 > max {
                break;
            }
            used += l.len() + 1;
            lines.push(l);
        }
        if !lines.is_empty() {
            out.push_str(&format!("Facts:\n{}\n", lines.join("\n")));
        }
        let out = out.trim_end().to_string();
        if cloud { self.veil(&out, keep) } else { out }
    }

    // ---- episodes ----

    /// A finished episode (a meeting's summary, a `remember`). Its id.
    pub fn add_episode(&self, source: &str, text: &str) -> i64 {
        let db = self.db.lock().unwrap();
        let now = now_ms();
        let _ = db.execute("INSERT INTO episodes(source, text, started, updated, ended) VALUES (?1, ?2, ?3, ?3, ?3)", params![source, text, now]);
        db.last_insert_rowid()
    }


    /// Run `f` on the database (the sleep, the voices, the verdicts).
    pub(crate) fn with_db<T>(&self, f: impl FnOnce(&Connection) -> T) -> T {
        f(&self.db.lock().unwrap())
    }
}

fn step_kind(step: Step) -> &'static str {
    match step {
        Step::Parent => "child",
        Step::Child => "parent",
        Step::Partner => "partner",
        Step::Sibling => "sibling",
        Step::Friend => "friend",
        Step::Colleague => "colleague",
    }
}

fn listed(facts: &[Fact]) -> String {
    facts.iter().take(12).map(|f| format!("[f{}] {}", f.id, f.text)).collect::<Vec<_>>().join("; ")
}

/// A date as the prompt says it.
pub(crate) fn day(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string()).unwrap_or_default()
}

/// `text` with every whole-word `word` (any case) replaced by `with`.
fn replace_word(text: &str, word: &str, with: &str) -> String {
    let lower: Vec<char> = word.chars().flat_map(char::to_lowercase).collect();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let starts = i == 0 || !chars[i - 1].1.is_alphanumeric();
        let n = lower.len();
        let matches = starts
            && i + n <= chars.len()
            && chars[i..i + n].iter().map(|(_, c)| *c).flat_map(char::to_lowercase).eq(lower.iter().copied())
            && chars.get(i + n).is_none_or(|(_, c)| !c.is_alphanumeric());
        if matches {
            out.push_str(with);
            i += n;
        } else {
            out.push(chars[i].1);
            i += 1;
        }
    }
    out
}

/// `text` with each veiled person's names replaced by their role.
fn hide(text: &str, veils: &[(i64, Vec<String>, String)]) -> String {
    let mut out = text.to_string();
    for (_, names, label) in veils {
        for name in names {
            out = replace_word(&out, name, label);
        }
    }
    out
}

/// `text` cut to `max` bytes at a character, with an ellipsis if cut.
fn clip(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut at = max.saturating_sub('…'.len_utf8());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    format!("{}…", text[..at].trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn veils_names_for_a_cloud_brain() {
        let memory = Memory::open(None);
        memory.relate_tool("Sofia", "partner", None, Some("f")).unwrap();
        memory.relate_tool("Luca", "friend", None, None).unwrap();
        memory.relate_tool("Anna", "friend", None, None).unwrap();
        memory.remember("Sofia loves tulips", Some("Sofia"), "notes", false, false, None).unwrap();
        memory.remember("Married to Sofia since 2015", None, "about", false, false, None).unwrap();
        let core = memory.core(4000, true);
        assert!(core.contains("Married to your wife since 2015") && !core.contains("Sofia"), "{core}");
        assert!(memory.core(4000, false).contains("Married to Sofia"));
        assert_eq!(memory.veil("Luca and Anna", &[]), "Luca and Anna");
        let found = memory.recalled("what does my wife like?", None, 4000, true);
        assert!(found.contains("your wife") && !found.contains("Sofia"), "{found}");
        let found = memory.recalled("what does Sofia like?", None, 4000, true);
        assert!(found.contains("Sofia loves tulips"), "{found}");
        assert_eq!(memory.resolve("your wife", None), memory.find("Sofia"));
        assert_eq!(replace_word("Sofiaa, sofia! Sofia", "Sofia", "X"), "Sofiaa, X! X");
    }

    /// The board gets the graph in short: the user, then the people by
    /// role, whole lines within its room, nothing private.
    #[test]
    fn the_board_gets_the_graph_in_short() {
        let memory = Memory::open(None);
        memory.relate_tool("Sofia", "partner", None, Some("f")).unwrap();
        memory.relate_tool("Luca", "friend", None, None).unwrap();
        memory.relate_tool("Anna", "friend", None, None).unwrap();
        memory.remember("Sofia loves tulips", Some("Sofia"), "notes", false, false, None).unwrap();
        memory.remember("Luca plays the drums", Some("Luca"), "notes", false, false, None).unwrap();
        memory.remember("Lives in Milan", None, "about", false, false, None).unwrap();
        memory.remember("Sees a therapist", None, "about", false, true, None).unwrap();
        for i in 0..40 {
            memory.remember(&format!("Likes board game number {i} a lot"), None, "about", false, false, None).unwrap();
        }
        let board = memory.board(1200);
        assert!(board.len() <= 1200, "{}", board.len());
        assert!(board.contains("Lives in Milan") && !board.contains("therapist"), "{board}");
        assert!(board.contains("- your wife: your wife loves tulips") && !board.contains("Sofia"), "{board}");
        assert!(board.contains("Luca plays the drums"), "{board}");
        assert!(board.lines().all(|l| !l.ends_with('…') || l.len() <= BOARD_LINE + 3), "{board}");
    }

    #[test]
    fn remembers_revises_and_forgets_by_id() {
        let memory = Memory::open(None);
        let r = memory.remember("Lives in Rome", None, "about", false, false, None).unwrap();
        assert!(r.starts_with("remembered as [f"), "{r}");
        assert!(memory.remember("lives in rome", None, "about", false, false, None).unwrap().starts_with("already known"));
        memory.remember("Works on ESP32 boards", None, "about", false, false, None).unwrap();
        memory.remember("Likes ESP32 dev kits", None, "notes", false, false, None).unwrap();
        assert!(memory.revise("esp32", "x").unwrap_err().contains("2 facts match"));
        let out = memory.revise("rome", "Lives in Milan since 2026").unwrap();
        assert!(out.contains("closed"), "{out}");
        let graph = memory.graph();
        assert_eq!(graph.facts.iter().filter(|f| f.to.is_some()).count(), 1);
        assert!(memory.text().contains("- Lives in Milan since 2026") && !memory.text().contains("Rome"));
        assert!(memory.forget_tool(&[], "esp32").unwrap().contains("2 facts match"));
        let id = memory.matching("dev kits")[0].id;
        assert_eq!(memory.forget_tool(&[id], "").unwrap(), "forgot 1 fact(s)");
        assert!(memory.remember("x", None, "nope", false, false, None).is_err());
    }

    #[test]
    fn my_wife_and_sofia_are_one_node() {
        let memory = Memory::open(None);
        memory.remember("The user's wife loves tulips", Some("my wife"), "matters", false, false, None).unwrap();
        let wife = memory.find("wife").unwrap();
        let mut e = memory.entity(wife).unwrap();
        assert!(e.placeholder);
        assert_eq!(e.label.as_deref(), Some("your wife"));
        e.name = "Sofia".into();
        e.placeholder = false;
        memory.put_entity(&e).unwrap();
        memory.remember("Sofia's mother bakes bread", Some("mother-in-law"), "notes", false, false, None).unwrap();
        let mil = memory.find("suocera").unwrap();
        assert_eq!(memory.entity(mil).unwrap().label.as_deref(), Some("your mother-in-law"));
        assert_eq!(memory.named_in("cosa piace a Sofia?"), vec![wife]);
        let recalled = memory.recalled("what does Sofia like", None, 2000, true);
        assert!(recalled.contains("Sofia: your wife") && recalled.contains("tulips"), "{recalled}");
        // Anna turns out to be the mother-in-law.
        let anna = memory.resolve("Anna", Some("person")).unwrap();
        memory.merge(mil, anna).unwrap();
        let merged = memory.entity(mil).unwrap();
        assert_eq!(merged.name, "Anna");
        assert!(merged.aliases.contains(&"mother-in-law".to_string()));
    }

    #[test]
    fn the_core_holds_preferences_and_search_the_rest() {
        let memory = Memory::open(None);
        memory.remember("Doesn't want news about football", None, "quiet", false, false, None).unwrap();
        memory.remember("The garage door code changes in May", None, "notes", false, true, None).unwrap();
        memory.remember("Likes espresso", None, "notes", false, false, None).unwrap();
        let core = memory.core(2000, true);
        assert!(core.contains("football") && !core.contains("espresso"), "{core}");
        assert!(memory.recalled("un espresso?", None, 2000, true).contains("Likes espresso"));
        assert!(!memory.recalled("garage code", None, 2000, true).contains("garage"));
        assert!(memory.recalled("garage code", None, 2000, false).contains("garage"));
    }
}
