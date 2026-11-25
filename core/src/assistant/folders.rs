//! Folders on Home: the user's own groups of conversations and meetings
//! ("Work", "Trip to Lisbon"). An item is in one folder or in none.
//!
//! Kept in `folders.json` in the data folder, apart from the conversations
//! and meetings themselves, so putting an item in a folder never rewrites
//! the item. The file is shaped for iCloud sync (`docs/icloud.md`): one
//! record per folder and one per item's place, each with its own `updated`
//! stamp, so two Macs merge them one by one (last writer wins per record).
//! Nothing is removed at once: a deleted folder stays as a tombstone
//! (`deleted`), and an item taken out of its folder keeps its place with no
//! folder, so a Mac that was off doesn't bring either back. Tombstones go
//! after [`TOMBSTONE_DAYS`].

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::inbox::now_ms;

/// More would crowd the sidebar.
pub const MAX: usize = 50;
const NAME_MAX: usize = 60;
/// How long a deletion is remembered, for a Mac that was off.
pub const TOMBSTONE_DAYS: u64 = 30;

/// What can go in a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Chat,
    Meeting,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    pub id: String,
    pub name: String,
    /// Where it shows among the others, lowest first.
    pub order: i64,
    /// When it was last changed (ms), to merge copies from other Macs.
    pub updated: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
}

/// Which folder an item is in (`None`: taken out of one).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub kind: Kind,
    pub id: String,
    pub folder: Option<String>,
    pub updated: u64,
}

/// The whole file: every record, tombstones included.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    folders: Vec<Folder>,
    #[serde(default)]
    places: Vec<Place>,
}

/// What the app shows: the live folders in order, and the items in them.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct FolderView {
    pub folders: Vec<Folder>,
    pub places: Vec<Place>,
}

pub struct Folders {
    file: Option<PathBuf>,
    store: Mutex<Store>,
}

impl Folders {
    /// The folders kept in `file` (`None`: in memory only).
    pub fn open(file: Option<PathBuf>) -> Self {
        let mut store: Store = file.as_ref().and_then(|f| fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        prune(&mut store, now_ms());
        Self { file, store: Mutex::new(store) }
    }

    pub fn view(&self) -> FolderView {
        view_of(&self.store.lock().unwrap())
    }

    /// A new folder, after the others.
    pub fn create(&self, name: &str) -> Result<FolderView, String> {
        self.change(|s, now| {
            let name = clean(name)?;
            let live: Vec<&Folder> = s.folders.iter().filter(|f| !f.deleted).collect();
            if live.len() >= MAX {
                return Err(format!("at most {MAX} folders"));
            }
            taken(&live, &name, None)?;
            let order = live.iter().map(|f| f.order).max().map_or(0, |o| o + 1);
            s.folders.push(Folder { id: new_id(now), name, order, updated: now, deleted: false });
            Ok(())
        })
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<FolderView, String> {
        self.change(|s, now| {
            let name = clean(name)?;
            taken(&s.folders.iter().filter(|f| !f.deleted).collect::<Vec<_>>(), &name, Some(id))?;
            let folder = live_mut(s, id)?;
            folder.name = name;
            folder.updated = now;
            Ok(())
        })
    }

    /// Delete a folder: what was in it goes back to no folder, never deleted.
    pub fn delete(&self, id: &str) -> Result<FolderView, String> {
        self.change(|s, now| {
            let folder = live_mut(s, id)?;
            folder.deleted = true;
            folder.updated = now;
            for p in s.places.iter_mut().filter(|p| p.folder.as_deref() == Some(id)) {
                p.folder = None;
                p.updated = now;
            }
            Ok(())
        })
    }

    /// Put an item in folder `folder`, or in none.
    pub fn put(&self, kind: Kind, id: &str, folder: Option<&str>) -> Result<FolderView, String> {
        self.change(|s, now| {
            if let Some(f) = folder {
                live_mut(s, f)?;
            }
            set_place(s, kind, id, folder, now);
            Ok(())
        })
    }

    /// An item was deleted: it leaves its folder.
    pub fn forget(&self, kind: Kind, id: &str) {
        let has = self.store.lock().unwrap().places.iter().any(|p| p.kind == kind && p.id == id && p.folder.is_some());
        if has {
            let _ = self.put(kind, id, None);
        }
    }

    fn change(&self, f: impl FnOnce(&mut Store, u64) -> Result<(), String>) -> Result<FolderView, String> {
        let mut store = self.store.lock().unwrap();
        let mut next = store.clone();
        f(&mut next, now_ms())?;
        self.save(&next)?;
        *store = next;
        Ok(view_of(&store))
    }

    fn save(&self, store: &Store) -> Result<(), String> {
        let Some(file) = &self.file else { return Ok(()) };
        let bytes = serde_json::to_vec_pretty(store).map_err(|e| e.to_string())?;
        // Written aside and moved in, so a crash never leaves half a file.
        let tmp = file.with_extension("json.tmp");
        fs::write(&tmp, bytes).and_then(|_| fs::rename(&tmp, file)).map_err(|e| e.to_string())
    }
}

fn view_of(s: &Store) -> FolderView {
    let mut folders: Vec<Folder> = s.folders.iter().filter(|f| !f.deleted).cloned().collect();
    folders.sort_by(|a, b| a.order.cmp(&b.order).then(a.updated.cmp(&b.updated)));
    // A place naming a folder that's gone (deleted on another Mac) is no folder.
    let places = s.places.iter().filter(|p| p.folder.as_ref().is_some_and(|f| folders.iter().any(|x| &x.id == f))).cloned().collect();
    FolderView { folders, places }
}

fn set_place(s: &mut Store, kind: Kind, id: &str, folder: Option<&str>, now: u64) {
    let folder = folder.map(str::to_string);
    match s.places.iter_mut().find(|p| p.kind == kind && p.id == id) {
        Some(p) if p.folder == folder => {}
        Some(p) => {
            p.folder = folder;
            p.updated = now;
        }
        None => s.places.push(Place { kind, id: id.to_string(), folder, updated: now }),
    }
}

fn live_mut<'a>(s: &'a mut Store, id: &str) -> Result<&'a mut Folder, String> {
    s.folders.iter_mut().find(|f| f.id == id && !f.deleted).ok_or_else(|| "no such folder".to_string())
}

fn clean(name: &str) -> Result<String, String> {
    let name: String = name.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(NAME_MAX).collect();
    if name.is_empty() { Err("a folder needs a name".into()) } else { Ok(name) }
}

/// Two folders with the same name couldn't be told apart.
fn taken(live: &[&Folder], name: &str, except: Option<&str>) -> Result<(), String> {
    let lower = name.to_lowercase();
    if live.iter().any(|f| Some(f.id.as_str()) != except && f.name.to_lowercase() == lower) { Err(format!("there's already a folder called {name}")) } else { Ok(()) }
}

/// Unique across Macs: the time, then random bits.
fn new_id(now: u64) -> String {
    let mut bytes = [0u8; 4];
    let _ = getrandom::fill(&mut bytes);
    format!("{now:x}{:08x}", u32::from_le_bytes(bytes))
}

/// Tombstones older than [`TOMBSTONE_DAYS`] are forgotten.
fn prune(s: &mut Store, now: u64) {
    let cutoff = now.saturating_sub(TOMBSTONE_DAYS * 86_400_000);
    s.folders.retain(|f| !f.deleted || f.updated > cutoff);
    s.places.retain(|p| p.folder.is_some() || p.updated > cutoff);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yoru-folders-{}", new_id(now_ms())));
        fs::create_dir_all(&dir).unwrap();
        dir.join("folders.json")
    }

    #[test]
    fn creates_in_order_and_refuses_duplicates_and_blank_names() {
        let f = Folders::open(None);
        f.create("Work").unwrap();
        let v = f.create("  Trip   to Lisbon ").unwrap();
        assert_eq!(v.folders.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["Work", "Trip to Lisbon"]);
        assert!(f.create("work").is_err());
        assert!(f.create("   ").is_err());
    }

    #[test]
    fn rename_keeps_the_id_and_checks_the_name() {
        let f = Folders::open(None);
        let a = f.create("Work").unwrap().folders[0].id.clone();
        f.create("Home").unwrap();
        assert!(f.rename(&a, "home").is_err());
        let v = f.rename(&a, "Office").unwrap();
        assert_eq!(v.folders[0].id, a);
        assert_eq!(v.folders[0].name, "Office");
        assert!(f.rename(&a, "office").is_ok(), "its own name, another case");
    }

    #[test]
    fn deleting_a_folder_frees_its_items_and_leaves_tombstones() {
        let f = Folders::open(None);
        let a = f.create("Work").unwrap().folders[0].id.clone();
        f.put(Kind::Chat, "c1", Some(&a)).unwrap();
        f.put(Kind::Meeting, "m1", Some(&a)).unwrap();
        assert_eq!(f.view().places.len(), 2);
        let v = f.delete(&a).unwrap();
        assert!(v.folders.is_empty() && v.places.is_empty());
        let s = f.store.lock().unwrap();
        assert!(s.folders[0].deleted);
        assert!(s.places.iter().all(|p| p.folder.is_none()), "the items stay, with no folder");
    }

    #[test]
    fn moving_items_between_folders_and_out() {
        let f = Folders::open(None);
        let a = f.create("A").unwrap().folders[0].id.clone();
        let b = f.create("B").unwrap().folders[1].id.clone();
        f.put(Kind::Chat, "c1", Some(&a)).unwrap();
        let v = f.put(Kind::Chat, "c1", Some(&b)).unwrap();
        assert_eq!(v.places.len(), 1);
        assert_eq!(v.places[0].folder.as_deref(), Some(b.as_str()));
        assert!(f.put(Kind::Chat, "c1", Some("nope")).is_err());
        f.forget(Kind::Chat, "c1");
        assert!(f.view().places.is_empty());
    }

    #[test]
    fn a_place_in_a_folder_deleted_elsewhere_counts_as_none() {
        let f = Folders::open(None);
        let a = f.create("A").unwrap().folders[0].id.clone();
        f.put(Kind::Chat, "c1", Some(&a)).unwrap();
        // As if another Mac's tombstone arrived without the places.
        f.store.lock().unwrap().folders[0].deleted = true;
        assert!(f.view().places.is_empty());
    }

    #[test]
    fn kept_in_the_file_and_old_tombstones_pruned() {
        let file = temp();
        let f = Folders::open(Some(file.clone()));
        let a = f.create("A").unwrap().folders[0].id.clone();
        let b = f.create("B").unwrap().folders[1].id.clone();
        f.put(Kind::Meeting, "m1", Some(&a)).unwrap();
        f.delete(&b).unwrap();
        let again = Folders::open(Some(file.clone()));
        assert_eq!(again.view(), f.view());
        assert_eq!(again.store.lock().unwrap().folders.len(), 2, "the tombstone is kept");

        let mut s = again.store.lock().unwrap().clone();
        prune(&mut s, now_ms() + (TOMBSTONE_DAYS + 1) * 86_400_000);
        assert_eq!(s.folders.len(), 1);
        assert_eq!(s.places.len(), 1);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }
}

impl Folders {
    /// Every folder and place as it travels (`folder:<id>`, `place:<kind>:<id>`).
    pub fn sync_items(&self) -> Vec<(String, serde_json::Value)> {
        let s = self.store.lock().unwrap();
        let folders = s.folders.iter().map(|f| (format!("folder:{}", f.id), serde_json::to_value(f).unwrap_or_default()));
        let places = s.places.iter().map(|p| (place_uid(p.kind, &p.id), serde_json::to_value(p).unwrap_or_default()));
        folders.chain(places).collect()
    }

    /// Folders and places from another Mac.
    pub fn sync_put(&self, changes: Vec<(String, Option<serde_json::Value>)>) {
        let mut store = self.store.lock().unwrap();
        let mut next = store.clone();
        for (uid, value) in changes {
            if uid.starts_with("folder:") {
                let at = next.folders.iter().position(|f| format!("folder:{}", f.id) == uid);
                match (value.and_then(|v| serde_json::from_value::<Folder>(v).ok()), at) {
                    (Some(f), Some(at)) => next.folders[at] = f,
                    (Some(f), None) => next.folders.push(f),
                    (None, Some(at)) => {
                        next.folders.remove(at);
                    }
                    (None, None) => {}
                }
            } else {
                let at = next.places.iter().position(|p| place_uid(p.kind, &p.id) == uid);
                match (value.and_then(|v| serde_json::from_value::<Place>(v).ok()), at) {
                    (Some(p), Some(at)) => next.places[at] = p,
                    (Some(p), None) => next.places.push(p),
                    (None, Some(at)) => {
                        next.places.remove(at);
                    }
                    (None, None) => {}
                }
            }
        }
        if self.save(&next).is_ok() {
            *store = next;
        }
    }
}

fn place_uid(kind: Kind, id: &str) -> String {
    format!("place:{}:{id}", if kind == Kind::Chat { "chat" } else { "meeting" })
}
