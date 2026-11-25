# iCloud sync between Macs (planned)

Yoru Hub on two or more Macs signed in to the same Apple ID shares one Yoru: the same memory, conversations, meetings, Spoken clips, tasks, timers, inbox, watchers, routines and skills. Live, both Macs on at once, through CloudKit in the user's private database. Started 2026-11-25.

## Why not a folder in iCloud Drive

`memory.db` and `history.db` are SQLite in WAL mode. iCloud Drive copies `.db`, `-wal` and `-shm` one by one, whenever it likes: sooner or later the copy on the other Mac is a broken database, and two Macs writing at once overwrite each other. So the files stay where they are (`~/Library/Application Support/yoru-hub`) and what travels is records, merged one by one.

## What travels and what doesn't

| Data | Synced as | Notes |
|---|---|---|
| Memory (`memory.db`: entities, facts, relations, episodes, verdicts) | one record per row, zone `Memory` | rows get a global id (below); embeddings and voice prints travel as bytes |
| Conversations (`conversations/*.json`, titles) | one record per conversation, zone `Chats` | the JSON as an asset; last writer wins, the conversation in progress is only written by the Mac it's on |
| Meetings (`meetings/<id>/`) | one record per meeting, zone `Meetings` | `meeting.json`, `transcript.json`, `summary.json` as fields; `mic.m4a` and `others.m4a` as assets (setting: text only, or with audio) |
| Spoken (`spoken/`) | one record per clip, zone `Spoken` | the audio as an asset |
| Tasks, timers, inbox, watchers, routines, skills, `assistant.json`, `connectors.json` (the list, not the tokens) | one record per item, zone `Settings` | each JSON file split into its items, with an `updated` stamp |
| `history.db` | not synced | it's an index of the conversations and meetings: each Mac rebuilds it (`History::sync`) |
| `keys.json`, OAuth tokens, Pocket key | not synced | secrets stay in this Mac's keychain; each Mac logs in on its own (iCloud Keychain later, maybe) |
| `models/`, `piper/`, `voice/`, `icons/`, `board-tools-*.json`, logs | not synced | downloaded or rebuilt on each Mac |
| `wifi-boards.json`, `hub.json`, `pocket/` | not synced | they belong to the hardware near this Mac |

## The hard parts

**Ids.** The memory uses `INTEGER PRIMARY KEY` and the brain sees facts as `f12`. Two Macs would both make an `f12`. Each row gets a `uid TEXT UNIQUE` (a UUIDv7, so it sorts by time); references between rows (`facts.about`, `relations.a/b`, `episode`) travel as uids and are mapped to local integers on arrival. Local ids stay what the prompts and the app use, so nothing above the store changes.

**Conflicts.** Every record carries `updated` (ms) and the Mac that wrote it. Last writer wins per record: facts are almost always appended and closed (`valid_to`), not edited, so a real clash is rare. A deletion is a tombstone record, kept 30 days, so a Mac that was off doesn't bring the row back.

**Who does things.** Timers, routines, watchers and memory consolidation have side effects: two Macs firing the same alarm or extracting the same facts twice is wrong. One Mac is the *doer*: the one with the board connected; with no board, the one used last. Its claim is a lease record in zone `Settings` (`doer`: the Mac's id and an expiry, renewed every minute, taken over when it expires). The others show everything and fire nothing. Consolidation additionally takes the episode's own lease, so an episode is consolidated once.

**When it syncs.** CloudKit pushes changes to the other Macs (silent notifications, `aps-environment`); as a fallback the app fetches every 5 minutes, on wake and when its window comes forward. Local changes are sent within a few seconds, in batches.

## How it's built

- **CKSyncEngine** (macOS 14+) does the bookkeeping: change tokens, batching, retries, account changes. It needs the entitlements on the process that calls it, so it runs **inside the app** (not in `yoru-audio`, a bare helper that can't carry a provisioning profile): a small Swift static library in `audio/` (`YoruSync`, built by `app/src-tauri/build.rs` like the helper) exposing a C interface, or `objc2-cloud-kit` if it covers CKSyncEngine well enough. To be checked at step 2.
- **Core side** (`core/src/sync/`): a trait each store implements (`changes since`, `apply record`, `delete`), so the core knows nothing about CloudKit and the CLI keeps working without it. A local outbox table in each store records what changed and hasn't been sent.
- **Settings → Accounts and services → iCloud**: on/off (off by default), meetings with or without audio, the Macs seen, which one is the doer, last sync and errors.

## Signing

CloudKit needs entitlements Apple only grants with a provisioning profile:

- `com.apple.developer.icloud-container-identifiers` = `iCloud.it.bytebusters.yoru.hub`
- `com.apple.developer.icloud-services` = `CloudKit`
- `com.apple.developer.aps-environment` (push)
- `com.apple.application-identifier`, `com.apple.developer.team-identifier`

The profile is embedded as `Contents/embedded.provisionprofile` (`bundle.macOS.files` in `tauri.conf.json`). With the Apple Development certificate `tools/build_app.sh` uses today, it's a **macOS App Development** profile: it runs only on the Macs listed in it and talks to CloudKit's *Development* environment. A build for others needs a Developer ID Application certificate, a Developer ID profile and the schema deployed to *Production*. The CI's ad-hoc build can't carry the profile: there iCloud is simply off.

What the user does once, at developer.apple.com (team `29H6RWQA4V`):

1. Identifiers → iCloud Containers → new: `iCloud.it.bytebusters.yoru.hub`.
2. Identifiers → App IDs → `it.bytebusters.yoru.hub` (create it if missing, macOS): tick iCloud (CloudKit, with the container above) and Push Notifications.
3. Devices: add each Mac (its Provisioning UDID: System Information → Hardware).
4. Profiles → new → macOS App Development, that App ID, the Apple Development certificate, the Macs; download it to `app/src-tauri/yoru.provisionprofile` (not committed).

## Steps

1. **Global ids** in `memory.db` (uid columns, backfill, refs by uid) and per-item `updated` stamps in the JSON stores; tombstones; the outbox. No CloudKit yet: tested with two data folders exchanging records in memory.
2. **The engine**: `YoruSync` with CKSyncEngine, the profile embedded, the zones; Settings and the memory first.
3. **Doer lease**: timers, routines, watchers and consolidation fire only on the doer.
4. **Conversations, meetings, Spoken**, with assets and the audio setting.
5. **The Settings page** and push.
6. Later: Developer ID build and Production schema; secrets through iCloud Keychain.
