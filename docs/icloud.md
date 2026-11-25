# iCloud sync between Macs

Yoru Hub on two or more Macs signed in to the same Apple ID shares one Yoru: the same memory, conversations, meetings, Spoken clips, tasks, timers, inbox, watchers, routines and skills. Live, both Macs on at once, through CloudKit in the user's private database. Started 2026-11-25.

## Why not a folder in iCloud Drive

`memory.db` and `history.db` are SQLite in WAL mode. iCloud Drive copies `.db`, `-wal` and `-shm` one by one, whenever it likes: sooner or later the copy on the other Mac is a broken database, and two Macs writing at once overwrite each other. So the files stay where they are (`~/Library/Application Support/yoru-hub`) and what travels is records, merged one by one.

## What travels and what doesn't

| Data | Synced as | Notes |
|---|---|---|
| Memory (`memory.db`: entities, facts, relations, episodes, verdicts) | one record per row, zone `Memory` | rows get a global id (below); voice prints travel as bytes, embeddings don't (each Mac makes its own) |
| Conversations (`conversations/*.json`, titles) | one record per conversation, zone `Chats` | the JSON as an asset; last writer wins, the conversation in progress is only written by the Mac it's on |
| Meetings (`meetings/<id>/`) | one record per meeting, zone `Meetings` | `meeting.json`, `transcript.json`, `summary.json` as fields; `mic.m4a` and `others.m4a` as assets (setting: text only, or with audio) |
| Spoken (`spoken/`) | one record per clip, zone `Spoken` | the audio as an asset |
| Folders (`folders.json`) | one record per folder and one per item's place, zone `Settings` | already shaped for it: each record has its `updated` stamp; deleted folders and items taken out are tombstones kept 30 days; a place whose folder is gone counts as no folder |
| Tasks, timers, inbox, watchers, routines, skills, `assistant.json`, `connectors.json` (the list, not the tokens) | one record per item, zone `Settings` | each JSON file split into its items, with an `updated` stamp |
| `history.db` | not synced | it's an index of the conversations and meetings: each Mac rebuilds it (`History::sync`) |
| `keys.json`, OAuth tokens, Pocket key | not synced | secrets stay in this Mac's keychain; each Mac logs in on its own (iCloud Keychain later, maybe) |
| `models/`, `piper/`, `voice/`, `icons/`, `board-tools-*.json`, logs | not synced | downloaded or rebuilt on each Mac |
| `wifi-boards.json`, `hub.json`, `pocket/` | not synced | they belong to the hardware near this Mac |

## The hard parts

**Ids.** The memory uses `INTEGER PRIMARY KEY` and the brain sees facts as `f12`. Two Macs would both make an `f12`. Each row gets a `uid TEXT UNIQUE` (128 random bits; the user's node is `user` on every Mac, so the two Users made at first start are one node); references between rows (`facts.about`, `relations.a/b`, `episode`) travel as uids and are mapped to local integers on arrival. Local ids stay what the prompts and the app use, so nothing above the store changes.

**Conflicts.** Every record carries `updated` (ms) and the Mac that wrote it. Last writer wins per record: facts are almost always appended and closed (`valid_to`), not edited, so a real clash is rare. A deletion is a tombstone record, kept 30 days, so a Mac that was off doesn't bring the row back.

**Who does things.** Timers, routines, watchers and memory consolidation have side effects: two Macs firing the same alarm or extracting the same facts twice is wrong. One Mac is the *doer*: the one with the board connected; with no board, the one used last. Its claim is a lease record in zone `Settings` (`doer`: the Mac's id and an expiry, renewed every minute, taken over when it expires). The others show everything and fire nothing. Consolidation additionally takes the episode's own lease, so an episode is consolidated once.

**When it syncs.** CloudKit pushes changes to the other Macs (silent notifications, `aps-environment`); as a fallback the app fetches every 5 minutes, on wake and when its window comes forward. Local changes are sent within a few seconds, in batches.

## How it's built

- **CKSyncEngine** (macOS 14+) does the bookkeeping: change tokens, batching, retries, account changes. It needs the entitlements on the process that calls it, so it runs **inside the app** (not in `yoru-audio`, a bare helper that can't carry a provisioning profile): `audio/Sources/YoruSync`, a Swift static library in the same package as the helper, built and linked into the app by `app/src-tauri/build.rs` (the Swift runtime is the system's, `/usr/lib/swift`). Its C interface takes and gives records as JSON (`yoru_sync_start`, `_save`, `_fetch`, `_account`, `_entitled`). A record is a CKRecord named `<kind>:<uid>` in its zone, with `kind`, `uid`, `changed`, `by`, `deleted` and `data` (the fields as JSON; over 700 KB, a `blob` asset instead). A deletion is saved as a record with `deleted` set, not deleted on iCloud, so last writer wins for deletions too. The engine keeps its state in `sync/cloudkit-state.json` and each record's server fields (its change tag) in `sync/cloudkit-records.plist`. When a save clashes with a newer copy on the server, that copy is handed to the core as if fetched: the core keeps the later of the two and sends its own again if it was the later.
- **The app side** (`app/src-tauri/src/icloud.rs`): every 5 seconds it asks each store for its changes and hands them to the engine; what comes in is applied to every store and the pages are told (`inbox`, `assistant`, `synced` events). It fetches every 5 minutes and when the window comes forward, on top of CloudKit's own pushes. It runs only in a build signed with the container (`yoru_sync_entitled`: touching CloudKit without it kills the process) and only when this Mac's `icloud` setting is on (`set_icloud`, `icloud_status`); off, it stays quiet until the app quits. What it did goes in `yoru.log` (`[icloud]`).
- **Core side** (`core/src/sync.rs`): `Record` (zone, kind, uid, `changed`, `by`, `deleted`, fields; bytes as base64) and the `Store` trait each store implements (`changes`, `sent`, `apply`), so the core knows nothing about CloudKit and the CLI keeps working without it. A local outbox table in each store records what changed and hasn't been sent.
- **The memory** (`core/src/assistant/memory/sync.rs`, since 2026-11-25): triggers on `entities`, `episodes`, `verdicts`, `facts` and `relations` keep `uid`, `changed` and `changed_by` (the Mac's id, `device` in `meta`), queue every change in `sync_outbox` and leave a tombstone (`sync_tombstones`, 30 days) for every deletion, so every writer of `memory.db` is covered without knowing about sync. While records from another Mac are applied a row in `sync_quiet` holds the triggers still, so nothing echoes back. A record that refers to a row not here yet waits in `sync_pending` and is tried again with the next ones. The first start with sync gives the old rows a uid and queues them all. A revision closes the fact and makes a new one, so two Macs revising the same fact at once keep both new facts; an edit in place (the Memory page) is last writer wins.
- **The JSON stores** (`Ledger` in `core/src/sync.rs`): a store only says what its items are (`Items`: each item by its uid, and how to put one in); the ledger remembers a hash of each in `sync/<kind>.json` and, whenever it is asked for changes, stamps what is new, changed or gone, so no writer of `tasks.json` has to know about sync. What comes in is put through the store and hashed again, so it isn't sent back. `core/src/assistant/synced.rs` lists every store that travels (`stores`): the memory, the settings (one record per key of `assistant.json`, routines included, so two Macs changing two different settings keep both), tasks (without the next run, which each Mac works out), timers (by a `uid` of their own: their numeric ids are per Mac), the inbox, watchers, folders and places, the user's skill files and the connectors (the list; tokens stay in each Mac's keychain). This Mac's id is `sync/device` in the data folder, shared by every store.
- **Settings → Accounts and services → iCloud**: on/off (off by default), meetings with or without audio, the Macs seen, which one is the doer, last sync and errors.

## Signing

CloudKit needs entitlements Apple only grants with a provisioning profile:

- `com.apple.developer.icloud-container-identifiers` = `iCloud.it.bytebusters.yoru.hub`
- `com.apple.developer.icloud-services` = `CloudKit`
- `com.apple.developer.aps-environment` (push)
- `com.apple.application-identifier`, `com.apple.developer.team-identifier`

Since 2026-11-25 the profile exists. Xcode made it with automatic signing on the user's account (team `79AC9KV62T`, the one of the Apple Development certificate `tools/build_app.sh` signs with): the App ID `it.bytebusters.yoru.hub` with iCloud (CloudKit) and Push, the container `iCloud.it.bytebusters.yoru.hub` and a "Mac Team Provisioning Profile" for this Mac, valid until 2027-11-25. It lives in `app/src-tauri/yoru.provisionprofile` (not committed). To make it again, or after adding a Mac: a throwaway Xcode project (`xcodegen`) with that bundle id, team and entitlements, built with `xcodebuild -allowProvisioningUpdates`; the new profile is in `~/Library/Developer/Xcode/UserData/Provisioning Profiles`. A second Mac is added by building the same throwaway project on it, signed in to the same account in Xcode.

When the profile is there, `tools/build_app.sh` adds `tauri.icloud.conf.json`: it embeds the profile as `Contents/embedded.provisionprofile` and signs with `Entitlements.icloud.plist` (the usual ones plus the identifiers above). Tauri signs the sidecars with the same entitlements, and a bare helper can't carry the profile that grants iCloud, so the script signs them again with the plain `Entitlements.plist` and seals the app again. Without the profile (the CI's ad-hoc build) nothing changes and iCloud is simply off.

It is a development profile: the app runs only on the Macs listed in it and talks to CloudKit's *Development* environment. A build for other people needs a Developer ID Application certificate, a Developer ID profile and the schema deployed to *Production*.

## Steps

1. **Global ids** in `memory.db` (uid columns, backfill, refs by uid), tombstones, the outbox; stamps and tombstones for the JSON stores through ledgers: done 2026-11-25, tested with two data folders exchanging records (`memory::sync` and `synced` tests). The conversations move to step 4 with the meetings.
2. **The engine**: `YoruSync` with CKSyncEngine, the profile embedded, the zones `Memory` and `Settings`: done 2026-11-25. On this Mac the first start sent every record (50) to the Development environment; the way back is still to be seen with a second Mac.
3. **Doer lease**: timers, routines, watchers and consolidation fire only on the doer.
4. **Conversations, meetings, Spoken**, with assets and the audio setting.
5. **The Settings page** and push.
6. Later: Developer ID build and Production schema; secrets through iCloud Keychain.
