//! What travels between Macs (docs/icloud.md): records, one per row or item,
//! each with a global id, the time it last changed and the Mac that changed
//! it. A store hands out what changed since it last sent ([`Store::changes`])
//! and takes what came from the others ([`Store::apply`]); the last writer
//! wins per record. Nothing here knows CloudKit: the engine that carries the
//! records sits in the app, so the core and the CLI work without it.

use std::cmp::Ordering;
use std::collections::BTreeMap;

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
pub trait Store {
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
