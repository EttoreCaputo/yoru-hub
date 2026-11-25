//! Every store whose records travel between Macs (docs/icloud.md), as the
//! engine in the app sees them: [`stores`] hands out one [`Store`] per kind.
//! The JSON stores go through a [`Ledger`]; the adapters here give the ones
//! that aren't a list of items (the settings, the skill files, the
//! connectors) the shape a ledger needs.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use super::Assistant;
use super::chats::SavedChat;
use super::watchers::Watchers;
use crate::sync::{Items, Ledger, Record, Store};

/// The zone of everything but the memory, the conversations, the meetings
/// and the Spoken clips, which have zones of their own.
pub const SETTINGS: &str = "Settings";
/// Every zone, made on iCloud at the first start.
pub const ZONES: &[&str] = &["Memory", SETTINGS, "Chats", "Meetings", "Spoken"];
/// Meetings with their audio go a few at a time.
const BIG_BATCH: usize = 4;

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
        Arc::new(Ledger::new("Chats", "chat", Arc::new(ChatItems(a.clone())), dir)),
    ];
    if let Some(watchers) = watchers {
        stores.push(Arc::new(Ledger::new(SETTINGS, "watcher", watchers, dir)));
    }
    if let Some(dir) = dir {
        let meetings = DirItems { dir: dir.join("meetings"), layout: Layout::Folders, optional_audio: true };
        stores.push(Arc::new(Ledger::new("Meetings", "meeting", Arc::new(meetings), Some(dir)).batch(BIG_BATCH)));
        let spoken = DirItems { dir: dir.join("spoken"), layout: Layout::Stems, optional_audio: false };
        stores.push(Arc::new(Ledger::new("Spoken", "clip", Arc::new(spoken), Some(dir)).batch(BIG_BATCH * 8)));
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

/// The conversations, each by its id: the ones put aside and the one in
/// use. The one in use here is not overwritten from another Mac, unless
/// what came is the same conversation gone further (it was opened there
/// too, and continued); a conversation deleted elsewhere while in use here
/// stays.
struct ChatItems(Arc<Assistant>);

impl Items for ChatItems {
    fn items(&self) -> Vec<(String, Value)> {
        let a = &self.0;
        let current = a.saved_current();
        let mut items: Vec<(String, Value)> =
            a.archive.all().into_iter().filter(|c| c.id != current.id).map(|c| (c.id.clone(), serde_json::to_value(&c).unwrap_or_default())).collect();
        if !current.lines.is_empty() {
            items.push((current.id.clone(), serde_json::to_value(&current).unwrap_or_default()));
        }
        items
    }

    fn put(&self, changes: Vec<(String, Option<Value>)>) {
        let a = &self.0;
        for (id, value) in changes {
            let current = a.archive.current();
            match value.and_then(|v| serde_json::from_value::<SavedChat>(v).ok()) {
                Some(chat) if chat.id == current => a.continue_from(chat),
                Some(chat) => a.archive.put(chat),
                None if id != current => {
                    a.archive.take(&id);
                }
                None => {}
            }
        }
    }
}

impl Assistant {
    /// The conversation in use, gone further on another Mac: take its lines
    /// when they begin with these (and no turn is under way).
    fn continue_from(&self, chat: SavedChat) {
        let Ok(_turn) = self.turn.try_lock() else { return };
        let here = self.chat();
        if chat.lines.len() > here.len() && chat.lines.starts_with(&here) {
            self.titles.replace(chat.title);
            *self.log.lock().unwrap() = chat.lines;
            self.log_changed();
        }
    }
}

/// Meetings send their audio too (docs/icloud.md): off, only their text.
pub static MEETING_AUDIO: AtomicBool = AtomicBool::new(true);

const AUDIO: &[&str] = &["m4a", "wav", "caf", "mp3", "aac", "opus", "flac"];

fn is_audio(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ext)| AUDIO.contains(&ext.to_ascii_lowercase().as_str()))
}

/// How a folder of files is cut into items.
#[derive(Clone, Copy, PartialEq)]
enum Layout {
    /// One folder per item (`meetings/<id>/`).
    Folders,
    /// One stem per item (`spoken/<id>.json` and `<id>.wav`).
    Stems,
}

/// A folder of files, each item its files: as stand-ins, their size and
/// time; as they travel, their bytes.
struct DirItems {
    dir: PathBuf,
    layout: Layout,
    /// The audio may stay home ([`MEETING_AUDIO`]).
    optional_audio: bool,
}

impl DirItems {
    fn audio_on(&self) -> bool {
        !self.optional_audio || MEETING_AUDIO.load(Ordering::Relaxed)
    }

    /// Every item's files, by item.
    fn files(&self) -> BTreeMap<String, Vec<(String, PathBuf)>> {
        let mut items: BTreeMap<String, Vec<(String, PathBuf)>> = BTreeMap::new();
        let Ok(entries) = fs::read_dir(&self.dir) else { return items };
        let keep = |name: &str| !name.starts_with('.') && !name.ends_with(".tmp");
        for entry in entries.flatten() {
            let Ok(name) = entry.file_name().into_string() else { continue };
            let path = entry.path();
            match self.layout {
                Layout::Folders if path.is_dir() && !name.starts_with('.') => {
                    if recording(&path) {
                        continue;
                    }
                    let files = fs::read_dir(&path)
                        .into_iter()
                        .flatten()
                        .flatten()
                        .filter(|f| f.path().is_file())
                        .filter_map(|f| f.file_name().into_string().ok().filter(|n| keep(n)).map(|n| (n, f.path())))
                        .collect();
                    items.insert(name, files);
                }
                Layout::Stems if path.is_file() && keep(&name) => {
                    let Some((stem, _)) = name.split_once('.') else { continue };
                    items.entry(stem.to_string()).or_default().push((name, path));
                }
                _ => {}
            }
        }
        items
    }

    fn folder(&self, uid: &str) -> PathBuf {
        match self.layout {
            Layout::Folders => self.dir.join(uid),
            Layout::Stems => self.dir.clone(),
        }
    }
}

/// A meeting still being recorded stays out until it ends.
fn recording(folder: &Path) -> bool {
    fs::read(folder.join("meeting.json")).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).is_some_and(|m| m["state"] == "recording")
}

fn stamp(path: &Path) -> Value {
    let meta = fs::metadata(path).ok();
    let size = meta.as_ref().map_or(0, |m| m.len());
    let time = meta.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as u64);
    serde_json::json!([size, time])
}

impl Items for DirItems {
    // The stand-in doesn't depend on the audio setting: turning it off
    // changes what goes out next, not every meeting at once.
    fn items(&self) -> Vec<(String, Value)> {
        self.files()
            .into_iter()
            .map(|(uid, files)| {
                let files: serde_json::Map<String, Value> = files.iter().map(|(n, p)| (n.clone(), stamp(p))).collect();
                (uid, serde_json::json!({ "files": files }))
            })
            .collect()
    }

    fn expand(&self, uid: &str, item: Value) -> Value {
        use base64::Engine as _;
        let folder = self.folder(uid);
        let audio = self.audio_on();
        let files: serde_json::Map<String, Value> = item["files"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(name, _)| audio || !is_audio(name))
            .filter_map(|(name, _)| fs::read(folder.join(name)).ok().map(|b| (name.clone(), serde_json::json!({ "b64": base64::engine::general_purpose::STANDARD.encode(b) }))))
            .collect();
        serde_json::json!({ "files": files, "audio": audio })
    }

    fn put(&self, changes: Vec<(String, Option<Value>)>) {
        use base64::Engine as _;
        for (uid, value) in changes {
            if !safe_name(&uid, "") || uid.contains('.') {
                continue;
            }
            let folder = self.folder(uid.as_str());
            let mine: Vec<(String, PathBuf)> = self.files_of(&uid);
            let Some(value) = value else {
                match self.layout {
                    Layout::Folders => drop(fs::remove_dir_all(&folder)),
                    Layout::Stems => mine.iter().for_each(|(_, p)| drop(fs::remove_file(p))),
                }
                continue;
            };
            let Some(files) = value["files"].as_object() else { continue };
            let with_audio = value["audio"].as_bool().unwrap_or(true);
            let _ = fs::create_dir_all(&folder);
            for (name, file) in files {
                let fits = safe_name(name, "") && (self.layout == Layout::Folders || name.starts_with(&format!("{uid}.")));
                let Some(bytes) = file["b64"].as_str().and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok()) else { continue };
                if fits {
                    let tmp = folder.join(format!(".{name}.sync"));
                    if fs::write(&tmp, bytes).is_ok() {
                        let _ = fs::rename(&tmp, folder.join(name));
                    }
                }
            }
            // What the other Mac no longer has goes, but not the audio it didn't send.
            for (name, path) in mine {
                if !files.contains_key(&name) && (with_audio || !is_audio(&name)) {
                    let _ = fs::remove_file(path);
                }
            }
        }
    }
}

impl DirItems {
    fn files_of(&self, uid: &str) -> Vec<(String, PathBuf)> {
        self.files().remove(uid).unwrap_or_default()
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
    fn conversations_meetings_and_clips_travel_with_their_files() {
        let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (a, sa) = mac(da.path());
        let (b, sb) = mac(db.path());
        let line = |at: u64, text: &str| super::super::ChatLine { at, from: "you".into(), text: text.into(), channel: super::super::Channel::Text, actions: Vec::new() };

        // A conversation on A, put aside: B has it, and the one in use on B stays B's.
        *a.log.lock().unwrap() = vec![line(1, "hello")];
        let first = a.archive.current();
        a.new_conversation();
        let meeting = da.path().join("meetings/20261125-100000");
        fs::create_dir_all(&meeting).unwrap();
        fs::write(meeting.join("meeting.json"), r#"{"state":"done"}"#).unwrap();
        fs::write(meeting.join("mic.m4a"), vec![7u8; 300_000]).unwrap();
        let live = da.path().join("meetings/20261125-110000");
        fs::create_dir_all(&live).unwrap();
        fs::write(live.join("meeting.json"), r#"{"state":"recording"}"#).unwrap();
        fs::create_dir_all(da.path().join("spoken")).unwrap();
        fs::write(da.path().join("spoken/c1.json"), "{}").unwrap();
        fs::write(da.path().join("spoken/c1.wav"), [1u8, 2, 3]).unwrap();
        exchange(&sa, &sb);
        assert_eq!(b.conversation(&first).unwrap().lines[0].text, "hello");
        assert_eq!(fs::read(db.path().join("meetings/20261125-100000/mic.m4a")).unwrap().len(), 300_000);
        assert!(!db.path().join("meetings/20261125-110000").exists(), "not while it records");
        assert_eq!(fs::read(db.path().join("spoken/c1.wav")).unwrap(), [1, 2, 3]);
        assert!(sa.iter().chain(&sb).all(|s| s.changes(1000).is_empty()), "quiet after a round");

        // B opens it and goes on: A, in another, gets the new line.
        b.open_conversation(&first).unwrap();
        b.log.lock().unwrap().push(line(2, "again"));
        exchange(&sa, &sb);
        assert_eq!(a.conversation(&first).unwrap().lines.len(), 2);

        // With the audio off, a meeting's text still travels and its audio stays.
        MEETING_AUDIO.store(false, Ordering::Relaxed);
        fs::write(meeting.join("summary.json"), "{}").unwrap();
        exchange(&sa, &sb);
        MEETING_AUDIO.store(true, Ordering::Relaxed);
        assert!(db.path().join("meetings/20261125-100000/summary.json").exists());
        assert!(db.path().join("meetings/20261125-100000/mic.m4a").exists());

        // Deleted on A, gone on B.
        fs::remove_dir_all(&meeting).unwrap();
        fs::remove_file(da.path().join("spoken/c1.wav")).unwrap();
        fs::remove_file(da.path().join("spoken/c1.json")).unwrap();
        exchange(&sa, &sb);
        assert!(!db.path().join("meetings/20261125-100000").exists());
        assert!(!db.path().join("spoken/c1.wav").exists());
    }

    #[test]
    fn a_file_name_from_another_mac_stays_in_its_folder() {
        assert!(safe_name("mine.md", ".md"));
        assert!(!safe_name("../keys.md", ".md") && !safe_name(".hidden.md", ".md") && !safe_name("x.json", ".md"));
    }
}
