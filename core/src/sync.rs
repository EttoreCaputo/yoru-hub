//! What travels between Macs (docs/icloud.md): records, one per row or item,
//! each with a global id, the time it last changed and the Mac that changed
//! it. A store hands out what changed since it last sent ([`Store::changes`])
//! and takes what came from the others ([`Store::apply`]); the last writer
//! wins per record. Nothing here knows CloudKit: the engine that carries the
//! records sits in the app, so the core and the CLI work without it.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// One value of a record. Bytes travel as base64 so a record is plain JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", content = "v", rename_all = "lowercase")]
pub enum Field {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Bytes(String),
}

impl Field {
    pub fn bytes(b: &[u8]) -> Self {
        Self::Bytes(base64::engine::general_purpose::STANDARD.encode(b))
    }

    pub fn as_text(&self) -> Option<&str> {
        if let Self::Text(s) = self { Some(s) } else { None }
    }
}

impl From<rusqlite::types::Value> for Field {
    fn from(v: rusqlite::types::Value) -> Self {
        use rusqlite::types::Value as V;
        match v {
            V::Null => Self::Null,
            V::Integer(i) => Self::Int(i),
            V::Real(r) => Self::Real(r),
            V::Text(s) => Self::Text(s),
            V::Blob(b) => Self::bytes(&b),
        }
    }
}

impl From<&Field> for rusqlite::types::Value {
    fn from(f: &Field) -> Self {
        use rusqlite::types::Value as V;
        match f {
            Field::Null => V::Null,
            Field::Int(i) => V::Integer(*i),
            Field::Real(r) => V::Real(*r),
            Field::Text(s) => V::Text(s.clone()),
            Field::Bytes(b) => base64::engine::general_purpose::STANDARD.decode(b).map(V::Blob).unwrap_or(V::Null),
        }
    }
}

/// A row or an item as it travels. A deleted one has no fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// The CloudKit zone it belongs to (`Memory`, `Settings`…).
    pub zone: String,
    /// What it is within the zone (`fact`, `entity`…).
    pub kind: String,
    /// Its global id, the same on every Mac.
    pub uid: String,
    /// When it last changed, ms since the epoch.
    pub changed: i64,
    /// The Mac that changed it ([`Store::device`]).
    pub by: String,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub fields: BTreeMap<String, Field>,
}

/// Which of two versions of a record wins: the later one, and on the same
/// millisecond the Mac whose id sorts last, so every Mac picks the same.
pub fn newer(a: (i64, &str), b: (i64, &str)) -> bool {
    a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)) == Ordering::Greater
}

/// Something whose records travel.
pub trait Store: Send + Sync {
    /// This Mac's id in the store, stamped on what it changes.
    fn device(&self) -> String;
    /// Up to `max` records changed here and not yet sent, oldest first.
    fn changes(&self, max: usize) -> Vec<Record>;
    /// These went out: forget them, unless they changed again meanwhile.
    fn sent(&self, records: &[Record]);
    /// Take records from the other Macs; how many changed something here.
    /// One that refers to a record not here yet waits for it.
    fn apply(&self, records: &[Record]) -> usize;
}

/// The id of this Mac, the same for every store: made once, kept in
/// `sync/device` in the data folder. Without a folder (tests), a new one.
pub fn device(dir: Option<&Path>) -> String {
    let fresh = || format!("{:016x}", rand_u64());
    let Some(dir) = dir else { return fresh() };
    let file = dir.join("sync").join("device");
    if let Ok(id) = fs::read_to_string(&file).map(|s| s.trim().to_string())
        && !id.is_empty()
    {
        return id;
    }
    let id = fresh();
    let _ = fs::create_dir_all(dir.join("sync"));
    let _ = fs::write(&file, &id);
    id
}

fn rand_u64() -> u64 {
    use std::hash::{BuildHasher, RandomState};
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    RandomState::new().hash_one(t) ^ t.rotate_left(17)
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// A store of JSON items, each with an id of its own (the tasks, the inbox,
/// the folders…), as the [`Ledger`] sees it.
pub trait Items: Send + Sync {
    /// Every item here now, by its uid.
    fn items(&self) -> Vec<(String, serde_json::Value)>;
    /// Items from another Mac: put each one in (`None`: take it out).
    fn put(&self, changes: Vec<(String, Option<serde_json::Value>)>);
}

/// How long a deletion is remembered, for a Mac that was off.
const TOMBSTONE_MS: i64 = 30 * 86_400_000;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Entry {
    /// The item as last seen, hashed: a new hash is a change made here.
    hash: u64,
    changed: i64,
    by: String,
    #[serde(default)]
    deleted: bool,
    /// Changed here and not sent yet.
    #[serde(default)]
    queued: bool,
}

/// Gives the items of a JSON store what sync needs without the store
/// knowing: it compares them with what it saw last (hashes, in
/// `sync/<kind>.json`), and a new, changed or gone item gets a stamp and goes
/// in the queue. So no writer of the store has to remember to stamp.
pub struct Ledger<I: Items> {
    zone: &'static str,
    kind: &'static str,
    items: std::sync::Arc<I>,
    device: String,
    file: Option<PathBuf>,
    entries: Mutex<BTreeMap<String, Entry>>,
}

/// FNV-1a: the same on every run and every build, unlike std's hasher.
fn hash(value: &serde_json::Value) -> u64 {
    serde_json::to_string(value).unwrap_or_default().bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

impl<I: Items> Ledger<I> {
    /// The ledger of `items`, kept in `dir/sync/<kind>.json` (`None`: in memory).
    pub fn new(zone: &'static str, kind: &'static str, items: std::sync::Arc<I>, dir: Option<&Path>) -> Self {
        let file = dir.map(|d| d.join("sync").join(format!("{kind}.json")));
        let entries = file.as_ref().and_then(|f| fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { zone, kind, items, device: device(dir), file, entries: Mutex::new(entries) }
    }

    fn save(&self, entries: &BTreeMap<String, Entry>) {
        if let Some(file) = &self.file {
            if let Some(dir) = file.parent() {
                let _ = fs::create_dir_all(dir);
            }
            let _ = serde_json::to_vec(entries).map(|b| fs::write(file, b));
        }
    }

    /// Stamp what changed here since the last look.
    fn scan(&self, entries: &mut BTreeMap<String, Entry>) -> bool {
        let now = now_ms();
        let mut dirty = false;
        let items = self.items.items();
        let here: std::collections::HashSet<&str> = items.iter().map(|(uid, _)| uid.as_str()).collect();
        for (uid, value) in &items {
            let h = hash(value);
            let entry = entries.entry(uid.clone()).or_default();
            if entry.hash != h || entry.deleted || entry.by.is_empty() {
                *entry = Entry { hash: h, changed: now.max(entry.changed + 1), by: self.device.clone(), deleted: false, queued: true };
                dirty = true;
            }
        }
        for (uid, entry) in entries.iter_mut() {
            if !entry.deleted && !here.contains(uid.as_str()) {
                *entry = Entry { hash: 0, changed: now.max(entry.changed + 1), by: self.device.clone(), deleted: true, queued: true };
                dirty = true;
            }
        }
        let before = entries.len();
        entries.retain(|_, e| !(e.deleted && !e.queued && now - e.changed > TOMBSTONE_MS));
        dirty || entries.len() != before
    }

    fn record(&self, uid: &str, entry: &Entry, value: Option<&serde_json::Value>) -> Record {
        let fields = match value {
            Some(v) if !entry.deleted => BTreeMap::from([("json".to_string(), Field::Text(v.to_string()))]),
            _ => BTreeMap::new(),
        };
        Record { zone: self.zone.into(), kind: self.kind.into(), uid: uid.into(), changed: entry.changed, by: entry.by.clone(), deleted: entry.deleted, fields }
    }
}

impl<I: Items> Store for Ledger<I> {
    fn device(&self) -> String {
        self.device.clone()
    }

    fn changes(&self, max: usize) -> Vec<Record> {
        let mut entries = self.entries.lock().unwrap();
        if self.scan(&mut entries) {
            self.save(&entries);
        }
        let mut queued: Vec<(&String, &Entry)> = entries.iter().filter(|(_, e)| e.queued).collect();
        queued.sort_by_key(|(_, e)| e.changed);
        queued.truncate(max);
        let items: BTreeMap<String, serde_json::Value> = self.items.items().into_iter().collect();
        queued.into_iter().map(|(uid, e)| self.record(uid, e, items.get(uid))).collect()
    }

    fn sent(&self, records: &[Record]) {
        let mut entries = self.entries.lock().unwrap();
        for r in records.iter().filter(|r| r.zone == self.zone && r.kind == self.kind) {
            if let Some(e) = entries.get_mut(&r.uid).filter(|e| e.changed == r.changed) {
                e.queued = false;
            }
        }
        self.save(&entries);
    }

    fn apply(&self, records: &[Record]) -> usize {
        let mut entries = self.entries.lock().unwrap();
        // What changed here first, so it's weighed against what comes.
        self.scan(&mut entries);
        let mut changes = Vec::new();
        let mut stamps = Vec::new();
        for r in records.iter().filter(|r| r.zone == self.zone && r.kind == self.kind) {
            if entries.get(&r.uid).is_some_and(|e| !newer((r.changed, &r.by), (e.changed, &e.by))) {
                continue;
            }
            let value = if r.deleted {
                None
            } else {
                match r.fields.get("json").and_then(Field::as_text).and_then(|j| serde_json::from_str(j).ok()) {
                    Some(v) => Some(v),
                    None => continue,
                }
            };
            changes.push((r.uid.clone(), value));
            stamps.push(r);
        }
        if changes.is_empty() {
            self.save(&entries);
            return 0;
        }
        self.items.put(changes);
        // Hash what the store holds now: it may write an item its own way.
        let now: BTreeMap<String, serde_json::Value> = self.items.items().into_iter().collect();
        for r in &stamps {
            let hash = now.get(&r.uid).map_or(0, hash);
            let deleted = r.deleted || !now.contains_key(&r.uid);
            entries.insert(r.uid.clone(), Entry { hash, changed: r.changed, by: r.by.clone(), deleted, queued: false });
        }
        self.save(&entries);
        stamps.len()
    }
}
