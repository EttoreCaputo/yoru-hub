//! The memory as records that travel between Macs (docs/icloud.md).
//!
//! Every synced row has a global `uid`, the time it last `changed` and the
//! Mac that changed it (`changed_by`). Triggers keep them, so every writer
//! of `memory.db` (the tools, the sleep, the voices, the Memory page) is
//! covered without knowing about sync: a change queues the row in
//! `sync_outbox`, a deletion leaves a tombstone. While records from another
//! Mac are applied, `sync_quiet` holds a row and the triggers stand still, so
//! nothing is sent back. Local integer ids stay what the prompts use (`f12`);
//! references between rows travel as uids.
//!
//! Embeddings don't travel: each Mac makes its own (`embed_missing`).

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};

use super::Memory;
use crate::sync::{Field, Record, Store, newer};

const ZONE: &str = "Memory";
/// A tombstone is kept this long, so a Mac that was off doesn't bring the row back.
const TOMBSTONE_DAYS: i64 = 30;
const NOW: &str = "CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)";
const DEVICE: &str = "(SELECT value FROM meta WHERE key = 'device')";
const RANDOM_UID: &str = "lower(hex(randomblob(16)))";

struct Table {
    name: &'static str,
    kind: &'static str,
    /// The columns that travel. A change to any other (the embedding) stays here.
    cols: &'static [&'static str],
    /// Columns holding another table's id, and that table.
    refs: &'static [(&'static str, &'static str)],
    /// Its text changing makes the embedding stale.
    embedded: bool,
}

/// In the order records are applied: what is referred to comes first.
const TABLES: [Table; 5] = [
    Table {
        name: "entities",
        kind: "entity",
        cols: &["name", "kind", "aliases", "summary", "gender", "placeholder", "is_user", "created", "voiceprint", "voice"],
        refs: &[],
        embedded: false,
    },
    Table { name: "episodes", kind: "episode", cols: &["source", "text", "started", "updated", "ended", "consolidated"], refs: &[], embedded: false },
    Table { name: "verdicts", kind: "verdict", cols: &["at", "source", "by", "title", "useful"], refs: &[], embedded: false },
    Table {
        name: "facts",
        kind: "fact",
        cols: &["about", "section", "text", "valid_from", "valid_to", "episode", "importance", "private", "created"],
        refs: &[("about", "entities"), ("episode", "episodes")],
        embedded: true,
    },
    Table {
        name: "relations",
        kind: "relation",
        cols: &["a", "kind", "b", "valid_from", "valid_to", "episode"],
        refs: &[("a", "entities"), ("b", "entities"), ("episode", "episodes")],
        embedded: false,
    },
];

fn table(kind: &str) -> Option<(usize, &'static Table)> {
    TABLES.iter().enumerate().find(|(_, t)| t.kind == kind)
}

/// The uid a new row gets. The user's node is `user` on every Mac, so the
/// two Users made at first start are one node.
fn new_uid(t: &Table, row: &str) -> String {
    if t.name == "entities" { format!("CASE WHEN {row}is_user = 1 THEN 'user' ELSE {RANDOM_UID} END") } else { RANDOM_UID.to_string() }
}

fn has_column(db: &Connection, table: &str, column: &str) -> bool {
    db.prepare(&format!("SELECT {column} FROM {table} LIMIT 0")).is_ok()
}

/// Add the sync columns, tables and triggers; give the rows made before
/// them a uid and queue them all, so the first sync sends everything.
/// `device` is this Mac's id ([`crate::sync::device`]), shared by every
/// store; `None` (no data folder) keeps a random one.
pub(super) fn install(db: &Connection, device: Option<&str>) -> rusqlite::Result<()> {
    db.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS sync_outbox(kind TEXT NOT NULL, uid TEXT NOT NULL, changed INTEGER NOT NULL, PRIMARY KEY(kind, uid));
         CREATE TABLE IF NOT EXISTS sync_tombstones(kind TEXT NOT NULL, uid TEXT NOT NULL, changed INTEGER NOT NULL, changed_by TEXT NOT NULL, PRIMARY KEY(kind, uid));
         CREATE TABLE IF NOT EXISTS sync_pending(kind TEXT NOT NULL, uid TEXT NOT NULL, record TEXT NOT NULL, PRIMARY KEY(kind, uid));
         CREATE TABLE IF NOT EXISTS sync_quiet(x);
         DELETE FROM sync_quiet;
         INSERT OR IGNORE INTO meta(key, value) VALUES ('device', {RANDOM_UID});
         DELETE FROM sync_tombstones WHERE changed < {NOW} - {TOMBSTONE_DAYS} * 86400000;"
    ))?;
    if let Some(device) = device {
        db.execute("UPDATE meta SET value = ?1 WHERE key = 'device'", [device])?;
    }
    for t in &TABLES {
        if !has_column(db, t.name, "uid") {
            db.execute_batch(&format!(
                "ALTER TABLE {n} ADD COLUMN uid TEXT;
                 ALTER TABLE {n} ADD COLUMN changed INTEGER;
                 ALTER TABLE {n} ADD COLUMN changed_by TEXT;",
                n = t.name
            ))?;
        }
        db.execute_batch(&format!(
            "UPDATE {n} SET uid = {uid}, changed = {NOW}, changed_by = {DEVICE} WHERE uid IS NULL;
             INSERT OR IGNORE INTO sync_outbox(kind, uid, changed) SELECT '{k}', uid, changed FROM {n} WHERE NOT EXISTS (SELECT 1 FROM meta WHERE key = 'sync_{k}');
             INSERT OR IGNORE INTO meta(key, value) VALUES ('sync_{k}', '1');
             CREATE UNIQUE INDEX IF NOT EXISTS {n}_uid ON {n}(uid);
             DROP TRIGGER IF EXISTS sync_{n}_ai;
             DROP TRIGGER IF EXISTS sync_{n}_au;
             DROP TRIGGER IF EXISTS sync_{n}_ad;
             CREATE TRIGGER sync_{n}_ai AFTER INSERT ON {n} WHEN NOT EXISTS (SELECT 1 FROM sync_quiet) BEGIN
                 UPDATE {n} SET uid = coalesce(new.uid, {new_uid}), changed = {NOW}, changed_by = {DEVICE} WHERE id = new.id;
                 INSERT OR REPLACE INTO sync_outbox(kind, uid, changed) SELECT '{k}', uid, changed FROM {n} WHERE id = new.id;
             END;
             CREATE TRIGGER sync_{n}_au AFTER UPDATE OF {cols} ON {n} WHEN NOT EXISTS (SELECT 1 FROM sync_quiet) BEGIN
                 UPDATE {n} SET changed = max({NOW}, coalesce(old.changed, 0) + 1), changed_by = {DEVICE} WHERE id = new.id;
                 INSERT OR REPLACE INTO sync_outbox(kind, uid, changed) SELECT '{k}', uid, changed FROM {n} WHERE id = new.id;
             END;
             CREATE TRIGGER sync_{n}_ad AFTER DELETE ON {n} WHEN old.uid IS NOT NULL AND NOT EXISTS (SELECT 1 FROM sync_quiet) BEGIN
                 INSERT OR REPLACE INTO sync_tombstones(kind, uid, changed, changed_by) VALUES ('{k}', old.uid, max({NOW}, coalesce(old.changed, 0) + 1), {DEVICE});
                 INSERT OR REPLACE INTO sync_outbox(kind, uid, changed) SELECT '{k}', uid, changed FROM sync_tombstones WHERE kind = '{k}' AND uid = old.uid;
             END;",
            n = t.name,
            k = t.kind,
            uid = new_uid(t, ""),
            new_uid = new_uid(t, "new."),
            cols = t.cols.join(", "),
        ))?;
    }
    Ok(())
}

/// The id of the row with this uid in `table`.
fn id_of(db: &Connection, table: &str, uid: &str) -> Option<i64> {
    db.query_row(&format!("SELECT id FROM {table} WHERE uid = ?1"), [uid], |r| r.get(0)).optional().ok().flatten()
}

fn uid_of(db: &Connection, table: &str, id: i64) -> Option<String> {
    db.query_row(&format!("SELECT uid FROM {table} WHERE id = ?1"), [id], |r| r.get(0)).optional().ok().flatten()
}

fn tombstone(db: &Connection, kind: &str, uid: &str) -> Option<(i64, String)> {
    db.query_row("SELECT changed, changed_by FROM sync_tombstones WHERE kind = ?1 AND uid = ?2", [kind, uid], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()
        .ok()
        .flatten()
}

/// The row with this uid, as it travels.
fn read(db: &Connection, t: &Table, uid: &str) -> Option<Record> {
    let sql = format!("SELECT {}, changed, changed_by FROM {} WHERE uid = ?1", t.cols.join(", "), t.name);
    let (values, changed, by): (Vec<Value>, i64, String) = db
        .query_row(&sql, [uid], |r| {
            let values = (0..t.cols.len()).map(|i| r.get(i)).collect::<rusqlite::Result<_>>()?;
            Ok((values, r.get(t.cols.len())?, r.get(t.cols.len() + 1)?))
        })
        .optional()
        .ok()
        .flatten()?;
    let fields = t
        .cols
        .iter()
        .zip(values)
        .map(|(col, v)| {
            let field = match (t.refs.iter().find(|(c, _)| c == col), v) {
                (Some((_, target)), Value::Integer(id)) => uid_of(db, target, id).map(Field::Text).unwrap_or(Field::Null),
                (_, v) => Field::from(v),
            };
            (col.to_string(), field)
        })
        .collect();
    Some(Record { zone: ZONE.into(), kind: t.kind.into(), uid: uid.into(), changed, by, deleted: false, fields })
}

/// What applying one record came to.
enum Applied {
    Changed,
    Skipped,
    /// It refers to a row not here yet.
    Waits,
}

fn upsert(db: &Connection, t: &Table, r: &Record) -> rusqlite::Result<Applied> {
    if tombstone(db, t.kind, &r.uid).is_some_and(|(c, by)| !newer((r.changed, &r.by), (c, &by))) {
        return Ok(Applied::Skipped);
    }
    let local: Option<(i64, i64, String)> = db
        .query_row(&format!("SELECT id, coalesce(changed, 0), coalesce(changed_by, '') FROM {} WHERE uid = ?1", t.name), [&r.uid], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .optional()?;
    if local.as_ref().is_some_and(|(_, c, by)| !newer((r.changed, &r.by), (*c, by))) {
        return Ok(Applied::Skipped);
    }
    let mut values: Vec<Value> = Vec::with_capacity(t.cols.len() + 3);
    for col in t.cols {
        let field = r.fields.get(*col).unwrap_or(&Field::Null);
        let value = match (t.refs.iter().find(|(c, _)| c == col), field) {
            (Some((_, target)), Field::Text(uid)) => match id_of(db, target, uid) {
                Some(id) => Value::Integer(id),
                None => return Ok(Applied::Waits),
            },
            (_, f) => Value::from(f),
        };
        values.push(value);
    }
    values.push(Value::Integer(r.changed));
    values.push(Value::Text(r.by.clone()));
    let n = t.cols.len();
    match local {
        Some((id, ..)) => {
            let mut set: Vec<String> = t.cols.iter().enumerate().map(|(i, c)| format!("{c} = ?{}", i + 1)).collect();
            if t.embedded {
                // The right side sees the old text: a new one drops the stale vector.
                let text = t.cols.iter().position(|c| *c == "text").unwrap_or(0) + 1;
                set.push(format!("embedding = CASE WHEN text IS ?{text} THEN embedding ELSE NULL END"));
            }
            set.push(format!("changed = ?{}, changed_by = ?{}", n + 1, n + 2));
            values.push(Value::Integer(id));
            db.execute(&format!("UPDATE {} SET {} WHERE id = ?{}", t.name, set.join(", "), n + 3), params_from_iter(values))?;
        }
        None => {
            values.push(Value::Text(r.uid.clone()));
            let marks: Vec<String> = (1..=n + 3).map(|i| format!("?{i}")).collect();
            db.execute(
                &format!("INSERT INTO {}({}, changed, changed_by, uid) VALUES ({})", t.name, t.cols.join(", "), marks.join(", ")),
                params_from_iter(values),
            )?;
        }
    }
    db.execute("DELETE FROM sync_tombstones WHERE kind = ?1 AND uid = ?2", [t.kind, &r.uid])?;
    db.execute("DELETE FROM sync_outbox WHERE kind = ?1 AND uid = ?2", [t.kind, &r.uid])?;
    Ok(Applied::Changed)
}

fn delete(db: &Connection, t: &Table, r: &Record) -> rusqlite::Result<Applied> {
    let local: Option<(i64, String)> = db
        .query_row(&format!("SELECT coalesce(changed, 0), coalesce(changed_by, '') FROM {} WHERE uid = ?1", t.name), [&r.uid], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .optional()?;
    // Changed here after it was deleted there: it stays, and goes back out.
    if local.as_ref().is_some_and(|(c, by)| newer((*c, by), (r.changed, &r.by))) {
        return Ok(Applied::Skipped);
    }
    if tombstone(db, t.kind, &r.uid).is_some_and(|(c, by)| !newer((r.changed, &r.by), (c, &by))) {
        return Ok(Applied::Skipped);
    }
    db.execute(&format!("DELETE FROM {} WHERE uid = ?1", t.name), [&r.uid])?;
    db.execute(
        "INSERT OR REPLACE INTO sync_tombstones(kind, uid, changed, changed_by) VALUES (?1, ?2, ?3, ?4)",
        params![t.kind, r.uid, r.changed, r.by],
    )?;
    db.execute("DELETE FROM sync_outbox WHERE kind = ?1 AND uid = ?2", [t.kind, &r.uid])?;
    Ok(if local.is_some() { Applied::Changed } else { Applied::Skipped })
}

fn apply_all(db: &Connection, records: &[Record]) -> rusqlite::Result<usize> {
    let mut all: Vec<Record> = {
        let mut stmt = db.prepare("SELECT record FROM sync_pending")?;
        let waiting: Vec<String> = stmt.query_map([], |r| r.get(0))?.flatten().collect();
        waiting.iter().filter_map(|s| serde_json::from_str(s).ok()).collect()
    };
    db.execute("DELETE FROM sync_pending", [])?;
    all.extend(records.iter().filter(|r| r.zone == ZONE).cloned());
    let mut known: Vec<(usize, &Table, Record)> = all.into_iter().filter_map(|r| table(&r.kind).map(|(i, t)| (i, t, r))).collect();
    // What is referred to comes first; deletions after, the referring ones first.
    known.sort_by_key(|(i, _, r)| if r.deleted { (1, TABLES.len() - i) } else { (0, *i) });
    let mut changed = 0;
    for (_, t, r) in &known {
        let applied = if r.deleted { delete(db, t, r)? } else { upsert(db, t, r)? };
        match applied {
            Applied::Changed => changed += 1,
            Applied::Skipped => {}
            Applied::Waits => {
                let json = serde_json::to_string(r).unwrap_or_default();
                db.execute("INSERT OR REPLACE INTO sync_pending(kind, uid, record) VALUES (?1, ?2, ?3)", params![r.kind, r.uid, json])?;
            }
        }
    }
    Ok(changed)
}

impl Store for Memory {
    fn device(&self) -> String {
        self.with_db(|db| db.query_row("SELECT value FROM meta WHERE key = 'device'", [], |r| r.get(0)).unwrap_or_default())
    }

    fn changes(&self, max: usize) -> Vec<Record> {
        self.with_db(|db| {
            let queued: Vec<(String, String)> = db
                .prepare("SELECT kind, uid FROM sync_outbox ORDER BY changed LIMIT ?1")
                .and_then(|mut stmt| stmt.query_map([max as i64], |r| Ok((r.get(0)?, r.get(1)?)))?.collect())
                .unwrap_or_default();
            let mut out = Vec::new();
            for (kind, uid) in queued {
                let Some((_, t)) = table(&kind) else { continue };
                if let Some(record) = read(db, t, &uid) {
                    out.push(record);
                } else if let Some((changed, by)) = tombstone(db, &kind, &uid) {
                    out.push(Record { zone: ZONE.into(), kind, uid, changed, by, deleted: true, fields: Default::default() });
                } else {
                    let _ = db.execute("DELETE FROM sync_outbox WHERE kind = ?1 AND uid = ?2", [&kind, &uid]);
                }
            }
            out
        })
    }

    fn sent(&self, records: &[Record]) {
        self.with_db(|db| {
            for r in records {
                let _ = db.execute("DELETE FROM sync_outbox WHERE kind = ?1 AND uid = ?2 AND changed = ?3", params![r.kind, r.uid, r.changed]);
            }
        });
    }

    fn apply(&self, records: &[Record]) -> usize {
        let mut db = self.db.lock().unwrap();
        let result = db.transaction().and_then(|tx| {
            tx.execute("INSERT INTO sync_quiet VALUES (1)", [])?;
            let n = apply_all(&tx, records)?;
            tx.execute("DELETE FROM sync_quiet", [])?;
            tx.commit()?;
            Ok(n)
        });
        result.unwrap_or_else(|e| {
            eprintln!("memory: sync apply: {e}");
            0
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `from` changed goes to `to`.
    fn ship(from: &Memory, to: &Memory) -> usize {
        let records = from.changes(10_000);
        from.sent(&records);
        to.apply(&records)
    }

    fn texts(m: &Memory) -> Vec<String> {
        let mut t: Vec<String> = m.graph().facts.into_iter().filter(|f| f.to.is_none()).map(|f| f.text).collect();
        t.sort();
        t
    }

    #[test]
    fn two_macs_end_up_with_one_memory() {
        let (a, b) = (Memory::open(None), Memory::open(None));
        assert_ne!(a.device(), b.device());
        a.relate_tool("Sofia", "partner", None, Some("f")).unwrap();
        a.remember("Sofia loves tulips", Some("Sofia"), "notes", false, false, None).unwrap();
        a.remember("Lives in Rome", None, "about", false, false, None).unwrap();
        ship(&a, &b);
        ship(&b, &a);
        assert_eq!(texts(&a), texts(&b));
        assert_eq!(b.entities().iter().filter(|e| e.name == "Sofia").count(), 1);
        assert_eq!(b.entities().iter().filter(|e| e.is_user).count(), 1, "the two Users are one node");
        assert_eq!(b.resolve("your wife", None), b.find("Sofia"), "the relation came along");
        let rome = b.matching("Rome")[0].clone();
        assert_eq!(rome.about, Some(b.user()));

        // Applied records aren't sent back.
        assert!(b.changes(100).is_empty() && a.changes(100).is_empty());

        // A revision on B reaches A; a deletion on A reaches B.
        b.revise(&format!("f{}", rome.id), "Lives in Milan").unwrap();
        ship(&b, &a);
        assert!(texts(&a).contains(&"Lives in Milan".to_string()) && !texts(&a).contains(&"Lives in Rome".to_string()));
        let tulips = a.matching("tulips")[0].id;
        a.forget(&[tulips]).unwrap();
        ship(&a, &b);
        assert!(b.matching("tulips").is_empty());
        assert_eq!(texts(&a), texts(&b));
    }

    #[test]
    fn a_fact_before_its_person_waits_for_it() {
        let (a, b) = (Memory::open(None), Memory::open(None));
        a.relate_tool("Luca", "friend", None, None).unwrap();
        a.remember("Luca plays the drums", Some("Luca"), "notes", false, false, None).unwrap();
        let records = a.changes(1000);
        let (facts, rest): (Vec<Record>, Vec<Record>) = records.into_iter().partition(|r| r.kind == "fact");
        b.apply(&facts);
        assert!(b.matching("drums").is_empty());
        b.apply(&rest);
        let drums = b.matching("drums");
        assert_eq!(drums.len(), 1);
        assert_eq!(drums[0].about, b.find("Luca"));
    }

    /// An edit in place (the Memory page): the later one wins on both Macs.
    /// A revision closes the fact and makes a new one, so two revisions at
    /// once are two facts, both kept, as two Macs told two things would.
    #[test]
    fn the_last_writer_wins_on_both() {
        let (a, b) = (Memory::open(None), Memory::open(None));
        a.remember("Likes tea", None, "about", false, false, None).unwrap();
        ship(&a, &b);
        let mut on_a = a.matching("tea")[0].clone();
        let mut on_b = b.matching("tea")[0].clone();
        on_a.text = "Likes green tea".into();
        a.put_fact(&on_a).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        on_b.text = "Likes black tea".into();
        b.put_fact(&on_b).unwrap();
        ship(&a, &b);
        ship(&b, &a);
        assert_eq!(texts(&a), vec!["Likes black tea".to_string()]);
        assert_eq!(texts(&a), texts(&b));

        let tea = a.matching("black tea")[0].id;
        a.revise(&format!("f{tea}"), "Likes oolong").unwrap();
        let tea = b.matching("black tea")[0].id;
        b.revise(&format!("f{tea}"), "Likes matcha").unwrap();
        ship(&a, &b);
        ship(&b, &a);
        assert_eq!(texts(&a), vec!["Likes matcha".to_string(), "Likes oolong".to_string()]);
        assert_eq!(texts(&a), texts(&b));
    }

    #[test]
    fn rows_made_before_sync_get_a_uid_and_go_out() {
        let dir = std::env::temp_dir().join(format!("yoru-sync-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        {
            let m = Memory::open(Some(&dir));
            m.remember("Has a cat", None, "about", false, false, None).unwrap();
            // As if made by a version without sync.
            m.with_db(|db| {
                db.execute_batch(
                    "DROP TRIGGER sync_facts_ai; DELETE FROM sync_outbox; DELETE FROM meta WHERE key = 'sync_fact';
                     INSERT INTO facts(about, section, text, valid_from, created) VALUES (1, 'about', 'Has a dog', 0, 0);",
                )
                .unwrap()
            });
        }
        let m = Memory::open(Some(&dir));
        let out = m.changes(100);
        let dog = out.iter().find(|r| r.fields.get("text") == Some(&Field::Text("Has a dog".into()))).expect("the old row goes out");
        assert_eq!(dog.uid.len(), 32);
        assert_eq!(dog.fields.get("about"), Some(&Field::Text("user".into())));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
