//! Every store whose records travel between Macs (docs/icloud.md), as the
//! engine in the app sees them: [`stores`] hands out one [`Store`] per kind.
//! The JSON stores go through a [`Ledger`]; the adapters here give the ones
//! that aren't a list of items (the settings, the skill files, the
//! connectors) the shape a ledger needs.

use std::fs;
use std::path::PathBuf;
#[cfg(test)]
use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use super::Assistant;
use super::watchers::Watchers;
use crate::sync::{Items, Ledger, Record, Store};

/// The zone of everything but the memory (and, later, the conversations,
/// meetings and Spoken clips, which have zones of their own).
pub const SETTINGS: &str = "Settings";

/// The stores that travel, for the assistant in `a` and the watchers.
pub fn stores(a: &Arc<Assistant>, watchers: Option<Arc<Watchers>>) -> Vec<Arc<dyn Store>> {
    let dir = a.dir().cloned();
    let dir = dir.as_deref();
    let mut stores: Vec<Arc<dyn Store>> = vec![
        Arc::new(MemoryStore(a.clone())),
        Arc::new(Ledger::new(SETTINGS, "setting", Arc::new(SettingsItems(a.clone())), dir)),
        Arc::new(Ledger::new(SETTINGS, "task", a.tasks.clone(), dir)),
        Arc::new(Ledger::new(SETTINGS, "timer", a.timers.clone(), dir)),
        Arc::new(Ledger::new(SETTINGS, "notification", a.inbox.clone(), dir)),
        Arc::new(Ledger::new(SETTINGS, "folder", Arc::new(FolderItems(a.clone())), dir)),
    ];
    if let Some(watchers) = watchers {
        stores.push(Arc::new(Ledger::new(SETTINGS, "watcher", watchers, dir)));
    }
    if let Some(dir) = dir {
        stores.push(Arc::new(Ledger::new(SETTINGS, "skill", Arc::new(SkillFiles(dir.join("skills"))), Some(dir))));
        #[cfg(feature = "connectors")]
        stores.push(Arc::new(Ledger::new(SETTINGS, "connector", Arc::new(ConnectorItems(dir.to_path_buf())), Some(dir))));
    }
    stores
}

/// The memory, which keeps its own outbox ([`super::memory::sync`]).
struct MemoryStore(Arc<Assistant>);

impl Store for MemoryStore {
    fn device(&self) -> String {
        self.0.memory.device()
    }
    fn changes(&self, max: usize) -> Vec<Record> {
        self.0.memory.changes(max)
    }
    fn sent(&self, records: &[Record]) {
        self.0.memory.sent(records)
    }
    fn apply(&self, records: &[Record]) -> usize {
        self.0.memory.apply(records)
    }
}

/// `assistant.json`, one record per setting: two Macs changing two
/// different settings both keep theirs.
struct SettingsItems(Arc<Assistant>);

impl Items for SettingsItems {
    fn items(&self) -> Vec<(String, Value)> {
        match serde_json::to_value(self.0.settings()) {
            Ok(Value::Object(map)) => map.into_iter().collect(),
            _ => Vec::new(),
        }
    }

    fn put(&self, changes: Vec<(String, Option<Value>)>) {
        let Ok(Value::Object(mut map)) = serde_json::to_value(self.0.settings()) else { return };
        for (key, value) in changes {
            // A setting taken out goes back to its default.
            match value {
                Some(v) => map.insert(key, v),
                None => map.remove(&key),
            };
        }
        if let Ok(settings) = serde_json::from_value(Value::Object(map)) {
            let _ = self.0.set_settings(settings);
        }
    }
}

/// The folders on Home and where each conversation and meeting sits.
struct FolderItems(Arc<Assistant>);

impl Items for FolderItems {
    fn items(&self) -> Vec<(String, Value)> {
        self.0.folders.sync_items()
    }
    fn put(&self, changes: Vec<(String, Option<Value>)>) {
        self.0.folders.sync_put(changes)
    }
}

/// The user's skills, `skills/<name>.md`, each file as text.
struct SkillFiles(PathBuf);

/// A file name from another Mac that stays in its folder.
fn safe_name(name: &str, ext: &str) -> bool {
    name.ends_with(ext) && !name.starts_with('.') && !name.contains(['/', '\\', '\0']) && name.len() <= 200
}

impl Items for SkillFiles {
    fn items(&self) -> Vec<(String, Value)> {
        let Ok(entries) = fs::read_dir(&self.0) else { return Vec::new() };
        let mut items: Vec<(String, Value)> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok().filter(|n| safe_name(n, ".md"))?;
                Some((name, Value::String(fs::read_to_string(e.path()).ok()?)))
            })
            .collect();
        items.sort_by(|a, b| a.0.cmp(&b.0));
        items
    }

    fn put(&self, changes: Vec<(String, Option<Value>)>) {
        for (name, value) in changes {
            if !safe_name(&name, ".md") {
                continue;
            }
            let file = self.0.join(&name);
            match value {
                Some(Value::String(text)) => {
                    let _ = fs::create_dir_all(&self.0);
                    let _ = fs::write(file, text);
                }
                Some(_) => {}
                None => {
                    let _ = fs::remove_file(file);
                }
            }
        }
    }
}

/// `connectors.json`, one record per connector: what the user chose, never
/// a token (those stay in each Mac's keychain, so each Mac signs in on its own).
#[cfg(feature = "connectors")]
struct ConnectorItems(PathBuf);

#[cfg(feature = "connectors")]
impl Items for ConnectorItems {
    fn items(&self) -> Vec<(String, Value)> {
        let Ok(file) = crate::connectors::config::ConnectorsFile::load(&self.0) else { return Vec::new() };
        file.connectors.iter().map(|c| (c.id.clone(), serde_json::to_value(c).unwrap_or_default())).collect()
    }

    fn put(&self, changes: Vec<(String, Option<Value>)>) {
        // A broken file is left alone, as the connectors' own save does.
        let Ok(mut file) = crate::connectors::config::ConnectorsFile::load(&self.0) else { return };
        for (id, value) in changes {
            let at = file.connectors.iter().position(|c| c.id == id);
            match (value.and_then(|v| serde_json::from_value(v).ok()), at) {
                (Some(c), Some(at)) => file.connectors[at] = c,
                (Some(c), None) => file.connectors.push(c),
                (None, Some(at)) => {
                    file.connectors.remove(at);
                }
                (None, None) => {}
            }
        }
        let _ = file.save(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timers::Timers;
    use serde_json::json;

    fn mac(dir: &Path) -> (Arc<Assistant>, Vec<Arc<dyn Store>>) {
        let a = Arc::new(Assistant::open(Some(dir.to_path_buf()), Arc::new(Timers::open(Some(dir.join("timers.json"))))));
        let watchers = Arc::new(Watchers::open(Some(dir.join("watchers.json"))));
        let stores = stores(&a, Some(watchers));
        (a, stores)
    }

    /// What CloudKit does, by hand: each Mac's changes go to the other.
    fn exchange(a: &[Arc<dyn Store>], b: &[Arc<dyn Store>]) {
        for (from, to) in [(a, b), (b, a)] {
            for (f, t) in from.iter().zip(to) {
                let records = f.changes(1000);
                t.apply(&records);
                f.sent(&records);
            }
        }
    }

    #[test]
    fn two_macs_end_up_with_the_same_settings_and_lists() {
        let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (a, sa) = mac(da.path());
        let (b, sb) = mac(db.path());
        assert_ne!(sa[1].device(), sb[1].device());
        assert_eq!(sa[0].device(), sa[1].device(), "one id per Mac");

        a.tasks.call_tool(&json!({"action": "add", "name": "Issues", "what": "sum up my issues", "repeat": "daily", "at": "09:00"})).unwrap();
        a.timers.start(600, Some("pasta"), "en").unwrap();
        a.inbox.post("test", "Hello", "from A", super::super::inbox::Urgency::Normal, None, None);
        fs::create_dir_all(da.path().join("skills")).unwrap();
        fs::write(da.path().join("skills/mine.md"), "---\nname: mine\ndescription: x\n---\nbody").unwrap();
        let mut s = b.settings();
        s.name = "Kuro".into();
        b.set_settings(s).unwrap();
        exchange(&sa, &sb);

        assert_eq!(b.tasks.list()[0].task.name, "Issues");
        assert!(b.tasks.list()[0].next.is_some(), "B schedules it itself");
        assert_eq!(b.timers.list()[0].label.as_deref(), Some("pasta"));
        assert_eq!(b.inbox.list()[0].title, "Hello");
        assert!(fs::read_to_string(db.path().join("skills/mine.md")).unwrap().ends_with("body"));
        assert_eq!(a.settings().name, "Kuro");
        assert_eq!(b.settings().name, "Kuro");

        // Nothing echoes: what came in isn't sent back.
        assert!(sa.iter().chain(&sb).all(|s| s.changes(1000).is_empty()), "quiet after a round");

        // A deletion travels, and an older edit doesn't bring it back.
        a.tasks.remove("Issues").unwrap();
        exchange(&sa, &sb);
        assert!(b.tasks.list().is_empty());

        // Two Macs changing two settings at once keep both.
        let mut s = a.settings();
        s.effort = "low".into();
        a.set_settings(s).unwrap();
        let mut s = b.settings();
        s.web_search = !s.web_search;
        let search = s.web_search;
        b.set_settings(s).unwrap();
        exchange(&sa, &sb);
        assert_eq!((a.settings().effort.as_str(), a.settings().web_search), ("low", search));
        assert_eq!(a.settings(), b.settings());
    }

    #[test]
    fn a_file_name_from_another_mac_stays_in_its_folder() {
        assert!(safe_name("mine.md", ".md"));
        assert!(!safe_name("../keys.md", ".md") && !safe_name(".hidden.md", ".md") && !safe_name("x.json", ".md"));
    }
}
