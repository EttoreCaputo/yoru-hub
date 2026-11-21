//! The transcripts searchable (docs/agent.md, The transcripts): every turn
//! of every conversation and meeting, indexed for words (SQLite FTS5) and
//! meaning (EmbeddingGemma), with its date and who said it. The transcripts
//! stay where they are and are the truth; `history.db` in the data folder
//! is a cache, filled again when deleted.
//!
//! A turn is what one speaker said without being interrupted: a chat line,
//! or a meeting's consecutive lines by the same person (a speech recognizer
//! cuts one speech in many short lines), up to [`TURN_CHARS`]. It keeps the
//! number of its first line, so it is read again in the transcript.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, params};

use super::memory::Embedder;

/// A turn's characters at most: a longer speech is cut in several.
const TURN_CHARS: usize = 1500;
/// The vector's dimensions kept: EmbeddingGemma's first 256 of 768
/// (Matryoshka), a third of the space for nearly the same quality.
const DIMS: usize = 256;
/// RRF's constant, as in the memory's search.
const RRF_K: f64 = 60.0;
/// Each list's length before fusing.
const DEPTH: usize = 60;
/// Below this cosine a meaning match is noise.
const MIN_COSINE: f32 = 0.40;
/// Turns given a vector in one call to the embedder.
const EMBED_BATCH: usize = 32;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS docs(key TEXT PRIMARY KEY, title TEXT NOT NULL, stamp TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS turns(
    id INTEGER PRIMARY KEY,
    doc TEXT NOT NULL,
    n INTEGER NOT NULL,
    at INTEGER NOT NULL,
    who TEXT NOT NULL,
    text TEXT NOT NULL,
    embedding BLOB
);
CREATE INDEX IF NOT EXISTS turns_doc ON turns(doc, n);
CREATE INDEX IF NOT EXISTS turns_at ON turns(at);
CREATE VIRTUAL TABLE IF NOT EXISTS turns_fts USING fts5(text, who, content='turns', content_rowid='id', tokenize='unicode61 remove_diacritics 2');
CREATE TRIGGER IF NOT EXISTS turns_ai AFTER INSERT ON turns BEGIN
    INSERT INTO turns_fts(rowid, text, who) VALUES (new.id, new.text, new.who);
END;
CREATE TRIGGER IF NOT EXISTS turns_ad AFTER DELETE ON turns BEGIN
    INSERT INTO turns_fts(turns_fts, rowid, text, who) VALUES ('delete', old.id, old.text, old.who);
END;
";

/// A conversation or a meeting, as the index sees it.
pub struct Doc {
    /// `chat:<id>` or `meeting:<id>`.
    pub key: String,
    pub title: String,
    /// Changes when the transcript does (lines added, a speaker named), so
    /// only those are indexed again.
    pub stamp: String,
}

/// A line of a transcript, before the lines are joined in turns.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// Its number in the transcript.
    pub n: usize,
    /// Unix ms.
    pub at: i64,
    pub who: String,
    pub text: String,
}

/// A turn found.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub doc: String,
    pub title: String,
    pub n: usize,
    pub at: i64,
    pub who: String,
    pub text: String,
}

/// What `history_search` looks for: any of them may be missing.
#[derive(Debug, Default)]
pub struct Search<'a> {
    pub query: &'a str,
    /// A part of the speaker's name.
    pub who: &'a str,
    /// Unix ms, inclusive.
    pub after: Option<i64>,
    pub before: Option<i64>,
    /// `chat` or `meeting`.
    pub source: Option<&'a str>,
    pub limit: usize,
}

pub struct History {
    db: Mutex<Connection>,
    embedder: Mutex<Option<Arc<dyn Embedder>>>,
    /// The vectors, read once: a search compares them all.
    vectors: Mutex<Option<Vec<(i64, Vec<f32>)>>>,
}

impl History {
    /// `history.db` in `dir`; in memory without one (the tests).
    pub fn open(dir: Option<&Path>) -> Self {
        let conn = dir
            .and_then(|d| {
                let _ = std::fs::create_dir_all(d);
                Connection::open(d.join("history.db")).map_err(|e| eprintln!("history.db: {e}")).ok()
            })
            .unwrap_or_else(|| Connection::open_in_memory().expect("an SQLite in memory"));
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
        if let Err(e) = conn.execute_batch(SCHEMA) {
            eprintln!("history.db schema: {e}");
        }
        Self { db: Mutex::new(conn), embedder: Mutex::default(), vectors: Mutex::default() }
    }

    pub fn set_embedder(&self, embedder: Option<Arc<dyn Embedder>>) {
        *self.embedder.lock().unwrap() = embedder;
    }

    /// Bring the index in line with `docs`: index the new and changed ones
    /// (their lines from `lines`), drop the ones gone. How many changed.
    pub fn sync(&self, docs: &[Doc], lines: impl Fn(&str) -> Option<Vec<Line>>) -> usize {
        let known: HashMap<String, String> = {
            let db = self.db.lock().unwrap();
            let Ok(mut stmt) = db.prepare("SELECT key, stamp FROM docs") else { return 0 };
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map(|rows| rows.flatten().collect()).unwrap_or_default()
        };
        let mut changed = 0;
        for doc in docs.iter().filter(|d| known.get(&d.key) != Some(&d.stamp)) {
            let Some(found) = lines(&doc.key) else { continue };
            let mut db = self.db.lock().unwrap();
            let Ok(tx) = db.transaction() else { continue };
            let _ = tx.execute("DELETE FROM turns WHERE doc = ?1", [&doc.key]);
            for t in turns(&found) {
                let _ = tx.execute("INSERT INTO turns(doc, n, at, who, text) VALUES (?1, ?2, ?3, ?4, ?5)", params![doc.key, t.n as i64, t.at, t.who, t.text]);
            }
            let _ = tx.execute("INSERT OR REPLACE INTO docs(key, title, stamp) VALUES (?1, ?2, ?3)", params![doc.key, doc.title, doc.stamp]);
            if tx.commit().is_ok() {
                changed += 1;
            }
        }
        let present: std::collections::HashSet<&str> = docs.iter().map(|d| d.key.as_str()).collect();
        let gone: Vec<&String> = known.keys().filter(|k| !present.contains(k.as_str())).collect();
        if !gone.is_empty() {
            let db = self.db.lock().unwrap();
            for key in &gone {
                let _ = db.execute("DELETE FROM turns WHERE doc = ?1", [key]);
                let _ = db.execute("DELETE FROM docs WHERE key = ?1", [key]);
            }
        }
        if changed + gone.len() > 0 {
            *self.vectors.lock().unwrap() = None;
        }
        changed + gone.len()
    }

    /// Give a vector to up to `max` turns without one. How many got one.
    pub fn embed_missing(&self, max: usize) -> usize {
        let Some(embedder) = self.embedder.lock().unwrap().clone() else { return 0 };
        let mut done = 0;
        while done < max {
            let missing: Vec<(i64, String)> = {
                let db = self.db.lock().unwrap();
                let Ok(mut stmt) = db.prepare(&format!("SELECT id, who || ': ' || text FROM turns WHERE embedding IS NULL LIMIT {EMBED_BATCH}")) else { break };
                stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map(|rows| rows.flatten().collect()).unwrap_or_default()
            };
            if missing.is_empty() {
                break;
            }
            let texts: Vec<String> = missing.iter().map(|(_, t)| t.clone()).collect();
            let vectors = match embedder.embed(&texts, false) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("history: embedding: {e}");
                    break;
                }
            };
            let db = self.db.lock().unwrap();
            for ((id, _), v) in missing.iter().zip(vectors) {
                let _ = db.execute("UPDATE turns SET embedding = ?1 WHERE id = ?2", params![to_blob(&shorten(v)), id]);
            }
            done += missing.len();
        }
        if done > 0 {
            *self.vectors.lock().unwrap() = None;
        }
        done
    }

    /// The turns that match `s`, best first: words and meaning fused; with
    /// no query, the newest that pass the filters.
    pub fn search(&self, s: &Search) -> Vec<Hit> {
        let limit = s.limit.clamp(1, 30);
        let ids: Vec<i64> = if s.query.trim().is_empty() {
            self.newest(s, limit)
        } else {
            let query_vector = self.embedder.lock().unwrap().clone().and_then(|e| e.embed(&[s.query.to_string()], true).ok()).and_then(|mut v| v.pop()).map(shorten);
            let lists = [self.words(s), query_vector.map(|v| self.meaning(&v, s)).unwrap_or_default()];
            let mut fused: HashMap<i64, f64> = HashMap::new();
            for list in &lists {
                for (rank, id) in list.iter().enumerate() {
                    *fused.entry(*id).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
                }
            }
            let mut scored: Vec<(i64, f64)> = fused.into_iter().collect();
            scored.sort_by(|a, b| b.1.total_cmp(&a.1));
            scored.into_iter().take(limit).map(|(id, _)| id).collect()
        };
        let db = self.db.lock().unwrap();
        ids.iter()
            .filter_map(|id| {
                db.query_row(
                    "SELECT t.doc, COALESCE(d.title, ''), t.n, t.at, t.who, t.text FROM turns t LEFT JOIN docs d ON d.key = t.doc WHERE t.id = ?1",
                    [id],
                    |r| Ok(Hit { doc: r.get(0)?, title: r.get(1)?, n: r.get::<_, i64>(2)? as usize, at: r.get(3)?, who: r.get(4)?, text: r.get(5)? }),
                )
                .ok()
            })
            .collect()
    }

    /// The filters as SQL on `turns t`, with their values.
    fn filters(s: &Search) -> (String, Vec<rusqlite::types::Value>) {
        use rusqlite::types::Value as V;
        let mut sql = String::new();
        let mut values: Vec<V> = Vec::new();
        if !s.who.trim().is_empty() {
            sql.push_str(" AND t.who LIKE ? ");
            values.push(V::Text(format!("%{}%", s.who.trim())));
        }
        if let Some(after) = s.after {
            sql.push_str(" AND t.at >= ? ");
            values.push(V::Integer(after));
        }
        if let Some(before) = s.before {
            sql.push_str(" AND t.at <= ? ");
            values.push(V::Integer(before));
        }
        if let Some(source) = s.source.filter(|x| !x.is_empty()) {
            sql.push_str(" AND t.doc LIKE ? ");
            values.push(V::Text(format!("{source}:%")));
        }
        (sql, values)
    }

    fn newest(&self, s: &Search, limit: usize) -> Vec<i64> {
        let (filters, values) = Self::filters(s);
        let db = self.db.lock().unwrap();
        let Ok(mut stmt) = db.prepare(&format!("SELECT t.id FROM turns t WHERE 1 = 1 {filters} ORDER BY t.at DESC LIMIT {limit}")) else { return Vec::new() };
        stmt.query_map(rusqlite::params_from_iter(values), |r| r.get(0)).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    /// FTS5's best matches for any of the query's words.
    fn words(&self, s: &Search) -> Vec<i64> {
        let words: Vec<String> = s.query.split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 2).map(|w| format!("\"{w}\"")).collect();
        if words.is_empty() {
            return Vec::new();
        }
        let (filters, mut values) = Self::filters(s);
        values.insert(0, rusqlite::types::Value::Text(words.join(" OR ")));
        let db = self.db.lock().unwrap();
        let sql = format!("SELECT t.id FROM turns_fts f JOIN turns t ON t.id = f.rowid WHERE turns_fts MATCH ? {filters} ORDER BY bm25(turns_fts) LIMIT {DEPTH}");
        let Ok(mut stmt) = db.prepare(&sql) else { return Vec::new() };
        stmt.query_map(rusqlite::params_from_iter(values), |r| r.get(0)).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    /// The turns closest in meaning to `query`, among those that pass the filters.
    fn meaning(&self, query: &[f32], s: &Search) -> Vec<i64> {
        let allowed: Option<std::collections::HashSet<i64>> = {
            let (filters, values) = Self::filters(s);
            if filters.is_empty() {
                None
            } else {
                let db = self.db.lock().unwrap();
                let Ok(mut stmt) = db.prepare(&format!("SELECT t.id FROM turns t WHERE 1 = 1 {filters}")) else { return Vec::new() };
                Some(stmt.query_map(rusqlite::params_from_iter(values), |r| r.get(0)).map(|rows| rows.flatten().collect()).unwrap_or_default())
            }
        };
        let mut cache = self.vectors.lock().unwrap();
        if cache.is_none() {
            let db = self.db.lock().unwrap();
            let all: Vec<(i64, Vec<f32>)> = db
                .prepare("SELECT id, embedding FROM turns WHERE embedding IS NOT NULL")
                .and_then(|mut stmt| stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, from_blob(&r.get::<_, Vec<u8>>(1)?)))).map(|rows| rows.flatten().collect()))
                .unwrap_or_default();
            *cache = Some(all);
        }
        let mut scored: Vec<(i64, f32)> = cache
            .as_ref()
            .unwrap()
            .iter()
            .filter(|(id, _)| allowed.as_ref().is_none_or(|a| a.contains(id)))
            .map(|(id, v)| (*id, v.iter().zip(query).map(|(a, b)| a * b).sum::<f32>()))
            .filter(|(_, cos)| *cos >= MIN_COSINE)
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored.into_iter().take(DEPTH).map(|(id, _)| id).collect()
    }

    /// How many turns are indexed, and how many have a vector.
    pub fn counts(&self) -> (usize, usize) {
        let db = self.db.lock().unwrap();
        let count = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0) as usize;
        (count("SELECT COUNT(*) FROM turns"), count("SELECT COUNT(*) FROM turns WHERE embedding IS NOT NULL"))
    }
}

/// `lines` joined in turns: a speaker's consecutive lines, up to [`TURN_CHARS`].
pub fn turns(lines: &[Line]) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    for line in lines.iter().filter(|l| !l.text.trim().is_empty()) {
        match out.last_mut() {
            Some(last) if last.who == line.who && last.text.len() + line.text.len() < TURN_CHARS => {
                last.text.push(' ');
                last.text.push_str(line.text.trim());
            }
            _ => out.push(Line { text: line.text.trim().to_string(), ..line.clone() }),
        }
    }
    out
}

/// The first [`DIMS`] of a vector, made unit again.
fn shorten(mut v: Vec<f32>) -> Vec<f32> {
    v.truncate(DIMS);
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
    v
}

fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn from_blob(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

// ---- The agent's side: what is indexed, and the two tools. ----

/// Characters of a turn a search shows: enough to judge it, then `history_read`.
const HIT_CHARS: usize = 400;
/// Characters `history_read` gives at once.
const READ_CHARS: usize = 6000;
/// Lines before the turn that `history_read` starts at, for what led to it.
const BEFORE: usize = 4;

impl super::Assistant {
    /// Index what changed in the conversations and the meetings' transcripts.
    pub fn sync_history(&self) -> usize {
        let (you, person) = super::app_tools::names(self);
        let name = self.settings().name;
        let mut docs: Vec<Doc> = self
            .conversations()
            .list
            .into_iter()
            .map(|c| Doc { key: format!("chat:{}", c.id), title: if c.title.is_empty() { c.preview.clone() } else { c.title.clone() }, stamp: format!("{}:{}:{name}:{you}", c.lines, c.updated) })
            .collect();
        docs.extend(crate::meetings::list().into_iter().filter(|m| m.transcribed_at.is_some()).map(|m| Doc {
            key: format!("meeting:{}", m.id),
            title: m.title.clone().unwrap_or_else(|| super::app_tools::when(m.started_at)),
            stamp: format!("{:?}:{}:{you}:{person}", m.transcribed_at, serde_json::to_string(&m.speakers).unwrap_or_default()),
        }));
        self.history.sync(&docs, |key| match key.split_once(':')? {
            ("chat", id) => Some(
                self.conversation(id)?
                    .lines
                    .iter()
                    .enumerate()
                    .map(|(n, l)| Line { n, at: l.at as i64, who: if l.from == "you" { you.to_string() } else { name.clone() }, text: l.text.clone() })
                    .collect(),
            ),
            ("meeting", id) => {
                let meeting = crate::meetings::get(id).ok()?;
                let transcript = crate::meetings::transcript(id).ok()??;
                let labels = crate::meetings::speaker_labels(&transcript.speakers, you, person);
                Some(
                    transcript
                        .lines
                        .iter()
                        .enumerate()
                        .map(|(n, l)| Line { n, at: meeting.started_at + l.start_ms as i64, who: labels.get(&l.who()).cloned().unwrap_or_else(|| "?".into()), text: l.text.clone() })
                        .collect(),
                )
            }
            _ => None,
        })
    }
}

pub(super) fn tools() -> Vec<serde_json::Value> {
    use serde_json::json;
    vec![
        json!({
            "name": "history_search",
            "description": "Search what was said in every past conversation with the user and every meeting transcript: by words and meaning, \
by who said it and by date. Use it for what the user or someone said, asked or decided, and when: then read around a turn with history_read \
and answer from the words. Without a query it lists the newest turns that match the other fields.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "What was said, in a few words (any language the transcripts are in)"},
                    "who": {"type": "string", "description": "Who said it: a name as the transcripts call them, or the user's label"},
                    "after": {"type": "string", "description": "From this day, YYYY-MM-DD"},
                    "before": {"type": "string", "description": "Until this day, YYYY-MM-DD, included"},
                    "source": {"type": "string", "enum": ["chat", "meeting"]},
                    "limit": {"type": "integer", "description": "How many turns, 8 if left out"},
                },
            },
        }),
        json!({
            "name": "history_read",
            "description": "Read a conversation or a meeting transcript around a turn that history_search found, by its ref (meeting:<id>#<n> or chat:<id>#<n>).",
            "input_schema": {"type": "object", "properties": {"ref": {"type": "string"}}, "required": ["ref"]},
        }),
    ]
}

/// Run `history_search` or `history_read`; `None` for another tool.
pub(super) fn call(a: &super::Assistant, name: &str, input: &serde_json::Value) -> Option<Result<String, String>> {
    match name {
        "history_search" => {
            a.sync_history();
            let day = |key: &str, end: bool| -> Result<Option<i64>, String> {
                let Some(text) = input[key].as_str().filter(|t| !t.trim().is_empty()) else { return Ok(None) };
                let date = chrono::NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d").map_err(|_| format!("{key}: a day as YYYY-MM-DD"))?;
                let time = if end { date.and_hms_opt(23, 59, 59) } else { date.and_hms_opt(0, 0, 0) }.unwrap();
                Ok(time.and_local_timezone(chrono::Local).earliest().map(|t| t.timestamp_millis()))
            };
            let (after, before) = match (day("after", false), day("before", true)) {
                (Ok(a), Ok(b)) => (a, b),
                (Err(e), _) | (_, Err(e)) => return Some(Err(e)),
            };
            let hits = a.history.search(&Search {
                query: input["query"].as_str().unwrap_or_default(),
                who: input["who"].as_str().unwrap_or_default(),
                after,
                before,
                source: input["source"].as_str(),
                limit: input["limit"].as_u64().unwrap_or(8) as usize,
            });
            if hits.is_empty() {
                return Some(Ok("nothing found".into()));
            }
            let lines: Vec<String> = hits
                .iter()
                .map(|h| {
                    let text: String = h.text.chars().take(HIT_CHARS).collect();
                    let more = if text.len() < h.text.len() { "…" } else { "" };
                    format!("{}#{} · {} · {} · {}: {text}{more}", h.doc, h.n, h.title, super::app_tools::when(h.at), h.who)
                })
                .collect();
            Some(Ok(lines.join("\n")))
        }
        "history_read" => {
            let reference = input["ref"].as_str().unwrap_or_default().trim();
            let (doc, n): (&str, usize) = reference.rsplit_once('#').map_or((reference, 0), |(d, n)| (d, n.parse().unwrap_or(0)));
            let from = n.saturating_sub(BEFORE);
            let (you, person) = super::app_tools::names(a);
            Some(match doc.split_once(':') {
                Some(("meeting", id)) => super::app_tools::read_meeting(id, from, READ_CHARS, you, person),
                Some(("chat", id)) => a.conversation(id).ok_or_else(|| "no conversation with that id".to_string()).map(|c| super::app_tools::read_chat(&c, from, READ_CHARS)),
                _ => Err("a ref is meeting:<id>#<n> or chat:<id>#<n>, as history_search gives it".into()),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(n: usize, at: i64, who: &str, text: &str) -> Line {
        Line { n, at, who: who.into(), text: text.into() }
    }

    fn doc(key: &str, stamp: &str) -> Doc {
        Doc { key: key.into(), title: format!("{key} title"), stamp: stamp.into() }
    }

    /// Unit vectors by the words they contain, so "price" is near "cost".
    struct Words;
    impl Embedder for Words {
        fn embed(&self, texts: &[String], query: bool) -> Result<Vec<Vec<f32>>, String> {
            Ok(texts
                .iter()
                .map(|t| {
                    let t = t.to_lowercase();
                    // The rest of the meaning: apart for queries and turns, so it matches nothing.
                    let (q, d) = if query { (0.1, 0.0) } else { (0.0, 0.1) };
                    let v = vec![if t.contains("price") || t.contains("cost") { 1.0 } else { 0.0 }, if t.contains("holiday") { 1.0 } else { 0.0 }, q, d];
                    shorten(v)
                })
                .collect())
        }
    }

    #[test]
    fn a_speakers_lines_make_one_turn() {
        let t = turns(&[line(0, 1, "Marco", "we ship"), line(1, 2, "Marco", "on Friday"), line(2, 3, "You", "ok"), line(3, 4, "Marco", "good")]);
        assert_eq!(t.len(), 3);
        assert_eq!((t[0].n, t[0].text.as_str()), (0, "we ship on Friday"));
        assert_eq!((t[2].n, t[2].who.as_str()), (3, "Marco"));
    }

    #[test]
    fn finds_by_words_meaning_person_and_date() {
        let h = History::open(None);
        h.set_embedder(Some(Arc::new(Words)));
        let lines = |key: &str| {
            Some(match key {
                "meeting:1" => vec![line(0, 1_000, "Marco", "The price goes to 39 a month"), line(1, 2_000, "You", "Fine for me")],
                _ => vec![line(0, 5_000, "You", "Book my holiday in Sardinia"), line(1, 6_000, "Yoru", "Done")],
            })
        };
        assert_eq!(h.sync(&[doc("meeting:1", "a"), doc("chat:9", "a")], lines), 2);
        assert_eq!(h.embed_missing(100), 4);
        // Words.
        let hits = h.search(&Search { query: "Sardinia", limit: 5, ..Default::default() });
        assert_eq!((hits[0].doc.as_str(), hits[0].who.as_str()), ("chat:9", "You"));
        // Meaning: "cost" is never said.
        let hits = h.search(&Search { query: "what will it cost", limit: 5, ..Default::default() });
        assert_eq!((hits[0].doc.as_str(), hits[0].title.as_str()), ("meeting:1", "meeting:1 title"));
        // A person, a date, a source, with no query: the newest first.
        assert_eq!(h.search(&Search { who: "marco", limit: 5, ..Default::default() }).len(), 1);
        assert_eq!(h.search(&Search { after: Some(4_000), limit: 5, ..Default::default() })[0].text, "Done");
        assert_eq!(h.search(&Search { source: Some("meeting"), limit: 5, ..Default::default() }).len(), 2);
    }

    #[test]
    fn indexes_again_only_what_changed() {
        let h = History::open(None);
        let lines = |_: &str| Some(vec![line(0, 1, "You", "hello there")]);
        assert_eq!(h.sync(&[doc("chat:1", "a"), doc("chat:2", "a")], lines), 2);
        assert_eq!(h.sync(&[doc("chat:1", "a"), doc("chat:2", "a")], lines), 0);
        assert_eq!(h.sync(&[doc("chat:1", "b"), doc("chat:2", "a")], lines), 1);
        // One gone.
        assert_eq!(h.sync(&[doc("chat:1", "b")], lines), 1);
        assert_eq!(h.counts(), (1, 0));
        assert_eq!(h.search(&Search { query: "hello", limit: 5, ..Default::default() }).len(), 1);
    }
}
