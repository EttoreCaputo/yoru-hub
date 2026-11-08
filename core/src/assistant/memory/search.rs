//! Finding the facts a message needs (docs/memory.md, Search): words
//! (FTS5), meaning (EmbeddingGemma's vectors, compared one by one: a
//! personal memory is a few thousand facts at most) and the facts of the
//! people it names, fused with Reciprocal Rank Fusion, then reranked by
//! recency and importance.

use std::collections::HashMap;

use rusqlite::{Connection, params};

/// RRF's constant: how much the first places weigh over the rest.
const RRF_K: f64 = 60.0;
/// Each list's length before fusing.
const DEPTH: usize = 50;
/// Recency's half-life, in days (Bip's, to be tuned).
const HALF_LIFE_DAYS: f64 = 45.0;
/// Below this cosine a vector match is noise.
const MIN_COSINE: f32 = 0.35;

/// What the search looks at.
pub struct Query<'a> {
    pub text: &'a str,
    /// The text's vector, when there is an embedder.
    pub vector: Option<Vec<f32>>,
    /// People (and things) the message names, or who is speaking.
    pub named: &'a [i64],
    /// Closed facts too (`recall`, questions about the past).
    pub closed: bool,
    /// Leave out the facts the core already puts in the prompt.
    pub skip_core: bool,
    pub now_ms: i64,
}

/// The facts found, best first, with their score.
pub fn run(conn: &Connection, q: &Query) -> Vec<(i64, f64)> {
    let lists = [words(conn, q), meaning(conn, q), people(conn, q)];
    let mut fused: HashMap<i64, f64> = HashMap::new();
    for list in &lists {
        for (rank, id) in list.iter().enumerate() {
            *fused.entry(*id).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
        }
    }
    let best = fused.values().cloned().fold(0.0, f64::max);
    if best <= 0.0 {
        return Vec::new();
    }
    let mut scored: Vec<(i64, f64)> = fused
        .into_iter()
        .filter_map(|(id, rrf)| {
            let (importance, from): (f64, i64) = conn.query_row("SELECT importance, valid_from FROM facts WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).ok()?;
            let days = (q.now_ms - from).max(0) as f64 / 86_400_000.0;
            let recency = 0.5f64.powf(days / HALF_LIFE_DAYS);
            Some((id, 0.7 * rrf / best + 0.15 * importance + 0.15 * recency))
        })
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored
}

/// Which facts may be found.
fn open_clause(q: &Query) -> String {
    let mut clause = String::from("1");
    if !q.closed {
        clause.push_str(" AND f.valid_to IS NULL");
    }
    if q.skip_core {
        clause.push_str(&format!(" AND NOT ({})", super::CORE_SQL));
    }
    clause
}

/// FTS5 on the facts' words, each word of the message as a prefix.
fn words(conn: &Connection, q: &Query) -> Vec<i64> {
    let terms: Vec<String> = q
        .text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3)
        .map(|w| format!("\"{}\"*", w.to_lowercase()))
        .collect();
    if terms.is_empty() {
        return Vec::new();
    }
    let sql = format!(
        "SELECT f.id FROM facts_fts JOIN facts f ON f.id = facts_fts.rowid WHERE facts_fts MATCH ?1 AND {} ORDER BY bm25(facts_fts) LIMIT {DEPTH}",
        open_clause(q)
    );
    let Ok(mut stmt) = conn.prepare(&sql) else { return Vec::new() };
    stmt.query_map([terms.join(" OR ")], |r| r.get(0)).map(|rows| rows.flatten().collect()).unwrap_or_default()
}

/// The facts nearest in meaning.
fn meaning(conn: &Connection, q: &Query) -> Vec<i64> {
    let Some(v) = &q.vector else { return Vec::new() };
    let sql = format!("SELECT f.id, f.embedding FROM facts f WHERE f.embedding IS NOT NULL AND {}", open_clause(q));
    let Ok(mut stmt) = conn.prepare(&sql) else { return Vec::new() };
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)));
    let mut near: Vec<(i64, f32)> = rows
        .map(|rows| rows.flatten().map(|(id, blob)| (id, cosine(v, &super::from_blob(&blob)))).filter(|(_, c)| *c >= MIN_COSINE).collect())
        .unwrap_or_default();
    near.sort_by(|a, b| b.1.total_cmp(&a.1));
    near.into_iter().take(DEPTH).map(|(id, _)| id).collect()
}

/// The facts about the people named, most important first.
fn people(conn: &Connection, q: &Query) -> Vec<i64> {
    let mut out = Vec::new();
    let sql = format!("SELECT f.id FROM facts f WHERE f.about = ?1 AND {} ORDER BY f.importance DESC, f.valid_from DESC LIMIT 12", open_clause(q));
    let Ok(mut stmt) = conn.prepare(&sql) else { return out };
    for id in q.named {
        if let Ok(rows) = stmt.query_map(params![id], |r| r.get::<_, i64>(0)) {
            out.extend(rows.flatten());
        }
    }
    out
}

/// Both are unit vectors.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}
