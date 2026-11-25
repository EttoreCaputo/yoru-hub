// YoruSync: the iCloud engine (docs/icloud.md, How it's built). CKSyncEngine
// keeps the change tokens, batches, retries and account changes; this file
// only turns the core's records into CKRecords and back. It is linked into
// the app (not the yoru-audio helper) because only the app carries the
// provisioning profile that grants CloudKit.
//
// The C interface takes and gives records as JSON, the wire shape of
// `app/src-tauri/src/icloud.rs`: {zone, kind, uid, changed, by, deleted, data},
// `data` being the record's fields as a JSON string.

import CloudKit
import Foundation

public typealias YoruSyncCallback = @convention(c) (UnsafePointer<CChar>, UnsafePointer<CChar>) -> Void

private let containerId = "iCloud.it.bytebusters.yoru.hub"
/// Over this, the fields go as a file (a CKAsset): a record holds 1 MB at most.
private let inlineLimit = 700_000
/// A record that failed is not sent again for this long, unless it changes.
private let failedPause: TimeInterval = 300

struct Wire: Codable {
    var zone: String
    var kind: String
    var uid: String
    var changed: Int64
    var by: String
    var deleted: Bool
    var data: String
}

final class Engine: CKSyncEngineDelegate, @unchecked Sendable {
    let dir: URL
    let zones: [String]
    let callback: YoruSyncCallback
    let lock = NSLock()
    /// Records to save, by CloudKit id.
    var pending: [CKRecord.ID: Wire] = [:]
    /// What failed and when, so it isn't sent again at once.
    var failed: [CKRecord.ID: (changed: Int64, at: Date)] = [:]
    /// The server's system fields of each record (its change tag), so a save
    /// replaces the record instead of clashing with it.
    var systemFields: [String: Data] = [:]
    var engine: CKSyncEngine!

    init(dir: URL, zones: [String], callback: @escaping YoruSyncCallback) {
        self.dir = dir
        self.zones = zones
        self.callback = callback
        try? FileManager.default.createDirectory(at: dir.appendingPathComponent("assets"), withIntermediateDirectories: true)
        if let data = try? Data(contentsOf: fieldsFile),
           let saved = try? PropertyListDecoder().decode([String: Data].self, from: data) {
            systemFields = saved
        }
        let state = (try? Data(contentsOf: stateFile)).flatMap { try? JSONDecoder().decode(CKSyncEngine.State.Serialization.self, from: $0) }
        let database = CKContainer(identifier: containerId).privateCloudDatabase
        var config = CKSyncEngine.Configuration(database: database, stateSerialization: state, delegate: self)
        config.automaticallySync = true
        engine = CKSyncEngine(config)
        if state == nil {
            addZones()
        }
    }

    var stateFile: URL { dir.appendingPathComponent("cloudkit-state.json") }
    var fieldsFile: URL { dir.appendingPathComponent("cloudkit-records.plist") }

    func addZones() {
        engine.state.add(pendingDatabaseChanges: zones.map { .saveZone(CKRecordZone(zoneName: $0)) })
    }

    func tell(_ event: String, _ text: String) {
        event.withCString { e in text.withCString { t in callback(e, t) } }
    }

    func tell(_ event: String, records: [Wire]) {
        guard !records.isEmpty, let json = try? JSONEncoder().encode(records) else { return }
        tell(event, String(decoding: json, as: UTF8.self))
    }

    func key(_ id: CKRecord.ID) -> String { "\(id.zoneID.zoneName)/\(id.recordName)" }

    func saveFields() {
        lock.lock()
        let fields = systemFields
        lock.unlock()
        if let data = try? PropertyListEncoder().encode(fields) {
            try? data.write(to: fieldsFile, options: .atomic)
        }
    }

    func remember(_ record: CKRecord) {
        let coder = NSKeyedArchiver(requiringSecureCoding: true)
        record.encodeSystemFields(with: coder)
        coder.finishEncoding()
        lock.lock()
        systemFields[key(record.recordID)] = coder.encodedData
        lock.unlock()
    }

    // MARK: From the core

    func save(_ records: [Wire]) {
        var ids: [CKSyncEngine.PendingRecordZoneChange] = []
        lock.lock()
        for r in records {
            let id = CKRecord.ID(recordName: "\(r.kind):\(r.uid)", zoneID: CKRecordZone.ID(zoneName: r.zone))
            if pending[id]?.changed == r.changed { continue }
            if let f = failed[id], f.changed == r.changed, Date().timeIntervalSince(f.at) < failedPause { continue }
            pending[id] = r
            ids.append(.saveRecord(id))
        }
        lock.unlock()
        if !ids.isEmpty {
            engine.state.add(pendingRecordZoneChanges: ids)
        }
    }

    // MARK: CKSyncEngineDelegate

    func handleEvent(_ event: CKSyncEngine.Event, syncEngine: CKSyncEngine) async {
        switch event {
        case .stateUpdate(let e):
            if let data = try? JSONEncoder().encode(e.stateSerialization) {
                try? data.write(to: stateFile, options: .atomic)
            }
        case .accountChange(let e):
            switch e.changeType {
            case .signIn:
                tell("account", "in")
            case .signOut:
                forget()
                tell("account", "out")
            case .switchAccounts:
                forget()
                tell("account", "switched")
            @unknown default:
                break
            }
        case .fetchedDatabaseChanges(let e):
            for deletion in e.deletions where zones.contains(deletion.zoneID.zoneName) {
                tell("error", "zone \(deletion.zoneID.zoneName) deleted on iCloud")
            }
        case .fetchedRecordZoneChanges(let e):
            var came: [Wire] = []
            for m in e.modifications {
                remember(m.record)
                if let w = wire(m.record) { came.append(w) }
            }
            saveFields()
            tell("fetched", records: came)
        case .sentRecordZoneChanges(let e):
            sent(e)
        case .didFetchChanges:
            tell("status", "fetched")
        case .didSendChanges:
            tell("status", "sent")
        default:
            break
        }
    }

    func nextRecordZoneChangeBatch(_ context: CKSyncEngine.SendChangesContext, syncEngine: CKSyncEngine) async -> CKSyncEngine.RecordZoneChangeBatch? {
        let changes = syncEngine.state.pendingRecordZoneChanges.filter { context.options.scope.contains($0) }
        return await CKSyncEngine.RecordZoneChangeBatch(pendingChanges: changes) { id in
            if let record = self.record(for: id) { return record }
            syncEngine.state.remove(pendingRecordZoneChanges: [.saveRecord(id)])
            return nil
        }
    }

    // MARK: Records

    func record(for id: CKRecord.ID) -> CKRecord? {
        lock.lock()
        let w = pending[id]
        let fields = systemFields[key(id)]
        lock.unlock()
        guard let w else { return nil }
        var record: CKRecord?
        if let fields, let coder = try? NSKeyedUnarchiver(forReadingFrom: fields) {
            coder.requiresSecureCoding = true
            record = CKRecord(coder: coder)
            coder.finishDecoding()
        }
        let r = record ?? CKRecord(recordType: w.kind, recordID: id)
        r["kind"] = w.kind
        r["uid"] = w.uid
        r["changed"] = w.changed
        r["by"] = w.by
        r["deleted"] = w.deleted ? 1 : 0
        if w.data.utf8.count > inlineLimit {
            let file = dir.appendingPathComponent("assets").appendingPathComponent(UUID().uuidString + ".json")
            try? Data(w.data.utf8).write(to: file)
            r["data"] = nil
            r["blob"] = CKAsset(fileURL: file)
        } else {
            r["data"] = w.data
            r["blob"] = nil
        }
        return r
    }

    func wire(_ r: CKRecord) -> Wire? {
        guard let kind = r["kind"] as? String, let uid = r["uid"] as? String,
              let changed = r["changed"] as? Int64, let by = r["by"] as? String else { return nil }
        var data = r["data"] as? String
        if data == nil, let asset = r["blob"] as? CKAsset, let url = asset.fileURL {
            data = (try? Data(contentsOf: url)).map { String(decoding: $0, as: UTF8.self) }
        }
        return Wire(zone: r.recordID.zoneID.zoneName, kind: kind, uid: uid, changed: changed, by: by,
                    deleted: (r["deleted"] as? Int64 ?? 0) != 0, data: data ?? "{}")
    }

    func sent(_ e: CKSyncEngine.Event.SentRecordZoneChanges) {
        var done: [Wire] = []
        var theirs: [Wire] = []
        var again: [CKSyncEngine.PendingRecordZoneChange] = []
        var zonesMissing = false
        for saved in e.savedRecords {
            remember(saved)
            if let blob = saved["blob"] as? CKAsset, let url = blob.fileURL, url.path.hasPrefix(dir.path) {
                try? FileManager.default.removeItem(at: url)
            }
            guard let w = wire(saved) else { continue }
            lock.lock()
            if pending[saved.recordID]?.changed == w.changed { pending[saved.recordID] = nil }
            failed[saved.recordID] = nil
            lock.unlock()
            done.append(w)
        }
        for f in e.failedRecordSaves {
            let id = f.record.recordID
            switch f.error.code {
            case .serverRecordChanged:
                // Someone saved it first: the core weighs theirs against
                // ours, and sends ours again if it is the later one.
                if let server = f.error.serverRecord {
                    remember(server)
                    if let w = wire(server) { theirs.append(w) }
                }
                lock.lock()
                pending[id] = nil
                lock.unlock()
            case .zoneNotFound:
                zonesMissing = true
                again.append(.saveRecord(id))
            case .unknownItem:
                lock.lock()
                systemFields[key(id)] = nil
                lock.unlock()
                again.append(.saveRecord(id))
            default:
                lock.lock()
                if let w = pending.removeValue(forKey: id) { failed[id] = (w.changed, Date()) }
                lock.unlock()
                tell("error", "\(id.recordName): \(f.error.localizedDescription)")
            }
        }
        if zonesMissing { addZones() }
        if !again.isEmpty { engine.state.add(pendingRecordZoneChanges: again) }
        saveFields()
        tell("sent", records: done)
        tell("fetched", records: theirs)
    }

    /// Another account, or none: what this Mac knew of iCloud no longer holds.
    func forget() {
        lock.lock()
        systemFields = [:]
        pending = [:]
        failed = [:]
        lock.unlock()
        try? FileManager.default.removeItem(at: fieldsFile)
        try? FileManager.default.removeItem(at: stateFile)
    }
}

private var shared: Engine?

/// Start the engine, its files in `dir`, with the zones named in `zones`
/// (comma separated). `callback` gets (event, text): `fetched` and `sent`
/// with records as JSON, `account`, `status`, `error`.
@_cdecl("yoru_sync_start")
public func yoru_sync_start(_ dir: UnsafePointer<CChar>, _ zones: UnsafePointer<CChar>, _ callback: YoruSyncCallback) {
    guard shared == nil else { return }
    let zones = String(cString: zones).split(separator: ",").map(String.init)
    shared = Engine(dir: URL(fileURLWithPath: String(cString: dir)), zones: zones, callback: callback)
}

/// Records changed here, as JSON: sent soon.
@_cdecl("yoru_sync_save")
public func yoru_sync_save(_ json: UnsafePointer<CChar>) {
    guard let engine = shared,
          let records = try? JSONDecoder().decode([Wire].self, from: Data(String(cString: json).utf8)) else { return }
    engine.save(records)
}

/// Ask iCloud for what changed, now.
@_cdecl("yoru_sync_fetch")
public func yoru_sync_fetch() {
    guard let engine = shared?.engine else { return }
    Task {
        do { try await engine.fetchChanges() } catch { shared?.tell("error", "fetch: \(error.localizedDescription)") }
    }
}

/// Send what is waiting, now.
@_cdecl("yoru_sync_send")
public func yoru_sync_send() {
    guard let engine = shared?.engine else { return }
    Task {
        do { try await engine.sendChanges() } catch { shared?.tell("error", "send: \(error.localizedDescription)") }
    }
}

/// Whether an iCloud account is signed in and reachable: calls back with
/// `account` and `available`, `none`, `restricted` or `unknown`.
@_cdecl("yoru_sync_account")
public func yoru_sync_account() {
    CKContainer(identifier: containerId).accountStatus { status, error in
        let text: String
        switch status {
        case .available: text = "available"
        case .noAccount: text = "none"
        case .restricted: text = "restricted"
        case .temporarilyUnavailable: text = "unavailable"
        default: text = error.map { "unknown: \($0.localizedDescription)" } ?? "unknown"
        }
        shared?.tell("account", text)
    }
}

/// Whether this process was signed with the iCloud container: without it,
/// touching CloudKit kills the process, so an ad-hoc or debug build never
/// starts the engine.
@_cdecl("yoru_sync_entitled")
public func yoru_sync_entitled() -> Bool {
    guard let task = SecTaskCreateFromSelf(nil) else { return false }
    let value = SecTaskCopyValueForEntitlement(task, "com.apple.developer.icloud-container-identifiers" as CFString, nil)
    return (value as? [String])?.contains(containerId) ?? false
}
