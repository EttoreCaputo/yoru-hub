// yoru-audio: Yoru Hub's helper for recording meetings (docs/meetings.md).
//
//   yoru-audio sources
//       The apps that use audio and the microphones, as one JSON object.
//   yoru-audio record <folder> [--system | --app <bundle id>] [--mic <uid|default>]...
//       Records each source as a track in <folder> until "stop" on stdin,
//       stdin closed, or SIGTERM; then finalizes the tracks.
//   yoru-audio finalize <folder>
//       Turns the tracks a recording left behind (`*.caf`) into `.m4a`.
//   yoru-audio import <file> <folder>
//       An audio or video file's sound as <folder>/others.m4a.
//   yoru-audio model <folder> [--download]
//       Whether the transcription model is in <folder>; fetches it first
//       with --download, reporting `progress`.
//   yoru-audio transcribe <file> --model <folder> [--language <code>] [--speakers <folder>]
//       The file's words as segments (`done` with `segments`), each with
//       its speaker when --speakers says where the diarization model is,
//       with `progress` on the way (Transcribe.swift).
//
// While recording, events go to stdout as JSON lines: `started` with the
// tracks, `level` four times a second, `error`, then `done` with the
// finished tracks. Yoru Hub starts it, so macOS asks for the microphone and
// system audio permissions on Yoru's behalf.

import AVFoundation
import Foundation

private let out = NSLock()
// Dispatch sources live as long as something holds them.
private var signals: [DispatchSourceSignal] = []
private var keep: DispatchSourceTimer?

/// One JSON line on stdout.
func emit(_ object: [String: Any]) {
    guard let data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]) else { return }
    out.lock()
    FileHandle.standardOutput.write(data + Data("\n".utf8))
    out.unlock()
}

func fail(_ message: String) -> Never {
    emit(["event": "error", "message": message])
    exit(1)
}

struct Options {
    var folder: URL
    var system = false
    var app: String?
    var mics: [String] = []
}

private func value(_ rest: inout ArraySlice<String>, for option: String) -> String {
    guard let value = rest.popFirst() else { fail("\(option): no value") }
    return value
}

func parse(_ args: ArraySlice<String>) -> Options {
    guard let first = args.first else { fail("record: no folder") }
    var options = Options(folder: URL(fileURLWithPath: first, isDirectory: true))
    var rest = args.dropFirst()
    while let arg = rest.popFirst() {
        switch arg {
        case "--system": options.system = true
        case "--app": options.app = value(&rest, for: arg)
        case "--mic": options.mics.append(value(&rest, for: arg))
        default: fail("unknown option \(arg)")
        }
    }
    if options.system && options.app != nil { fail("--system and --app don't go together") }
    if !options.system && options.app == nil && options.mics.isEmpty { fail("nothing to record") }
    return options
}

/// The microphone permission, asked now so the prompt comes before the
/// recording rather than a silent track.
func askMicrophone() {
    let done = DispatchSemaphore(value: 0)
    var granted = false
    AVCaptureDevice.requestAccess(for: .audio) { granted = $0; done.signal() }
    done.wait()
    if !granted { fail("Yoru isn't allowed to use the microphone (System Settings, Privacy & Security, Microphone)") }
}

final class Recording {
    let tap: Tap?
    let tracks: [Track]
    private var stopped = false
    private var ticks = 0
    private var warned: Set<String> = []

    init(_ options: Options) throws {
        try FileManager.default.createDirectory(at: options.folder, withIntermediateDirectories: true)
        var tracks: [Track] = []
        var tap: Tap?
        if options.system || options.app != nil {
            var source = "System audio"
            var processes: [AudioObjectID]?
            if let id = options.app {
                guard let app = try audioApps().first(where: { $0.id == id }) else { throw AudioError(description: "\(id) isn't using audio") }
                source = app.name
                processes = app.processes
            }
            let made = try Tap(processes: processes)
            tap = made
            tracks.append(try Track(name: "others", source: source, kind: "others", device: made.aggregate, stream: made.format, folder: options.folder))
        }
        if !options.mics.isEmpty { askMicrophone() }
        for (i, uid) in options.mics.enumerated() {
            let device = uid == "default" ? try defaultDevice(input: true) : try device(uid: uid)
            let name = readString(device, kAudioObjectPropertyName) ?? uid
            tracks.append(try Track(name: i == 0 ? "mic" : "mic-\(i + 1)", source: name, kind: "mic", device: device, stream: try inputFormat(device), folder: options.folder))
        }
        self.tap = tap
        self.tracks = tracks
    }

    func start() throws {
        for track in tracks { try track.start() }
        emit(["event": "started", "tracks": tracks.map(\.info)])
    }

    func levels() {
        emit(["event": "level", "levels": Dictionary(uniqueKeysWithValues: tracks.map { ($0.name, Double($0.takePeak())) })])
        ticks += 1
        // A running device hands over buffers all the time, silent or not.
        // None after three seconds: macOS isn't letting Yoru have the sound,
        // and says nothing about it. Mostly the system audio permission.
        guard ticks >= 12 else { return }
        for track in tracks where track.durationMs == 0 && !warned.contains(track.name) {
            warned.insert(track.name)
            let code = track.kind == "others" ? "no_system_audio" : "no_microphone_sound"
            emit(["event": "error", "code": code, "track": track.name, "message": "\(track.source): no sound is coming in"])
        }
    }

    /// Stop, finalize, report and exit. Once.
    func finish() -> Never {
        if !stopped {
            stopped = true
            for track in tracks { track.stop() }
            tap?.destroy()
        }
        // A track that got nothing stays listed, without a file.
        let finished = tracks.map { track -> [String: Any] in
            guard let file = finalize(track.url) else { return track.info.merging(["duration_ms": 0]) { $1 } }
            return track.info.merging(["file": file, "duration_ms": track.durationMs]) { $1 }
        }
        emit(["event": "done", "tracks": finished])
        exit(0)
    }
}

/// `<name>.caf` into `<name>.m4a` (the same audio, no re-encoding); the CAF
/// goes once the M4A is there. Returns the file's name.
func finalize(_ caf: URL) -> String? {
    let m4a = caf.deletingPathExtension().appendingPathExtension("m4a")
    let fm = FileManager.default
    guard fm.fileExists(atPath: caf.path) else { return fm.fileExists(atPath: m4a.path) ? m4a.lastPathComponent : nil }
    try? fm.removeItem(at: m4a)
    // Nothing recorded: no file rather than a broken one.
    if ((try? AVAudioFile(forReading: caf))?.length ?? 0) == 0 {
        try? fm.removeItem(at: caf)
        return nil
    }
    let asset = AVURLAsset(url: caf)
    guard let export = AVAssetExportSession(asset: asset, presetName: AVAssetExportPresetPassthrough) else {
        emit(["event": "error", "message": "\(caf.lastPathComponent): can't convert it"])
        return caf.lastPathComponent
    }
    export.outputURL = m4a
    export.outputFileType = .m4a
    let done = DispatchSemaphore(value: 0)
    export.exportAsynchronously { done.signal() }
    done.wait()
    guard export.status == .completed else {
        // An empty or broken track stays as it is: still playable, or nothing.
        emit(["event": "error", "message": "\(caf.lastPathComponent): \(export.error?.localizedDescription ?? "conversion failed")"])
        return caf.lastPathComponent
    }
    try? fm.removeItem(at: caf)
    return m4a.lastPathComponent
}

/// `import`: the sound of `source`, re-encoded as AAC, in `folder`.
func importFile(_ source: URL, into folder: URL) throws {
    let asset = AVURLAsset(url: source)
    let done = DispatchSemaphore(value: 0)
    var tracks: [AVAssetTrack] = []
    asset.loadTracks(withMediaType: .audio) { loaded, _ in tracks = loaded ?? []; done.signal() }
    done.wait()
    guard !tracks.isEmpty else { throw AudioError(description: "\(source.lastPathComponent) has no sound") }
    try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
    let out = folder.appendingPathComponent("others.m4a")
    try? FileManager.default.removeItem(at: out)
    guard let export = AVAssetExportSession(asset: asset, presetName: AVAssetExportPresetAppleM4A) else {
        throw AudioError(description: "\(source.lastPathComponent) can't be read")
    }
    export.outputURL = out
    export.outputFileType = .m4a
    export.exportAsynchronously { done.signal() }
    done.wait()
    guard export.status == .completed else { throw AudioError(description: export.error?.localizedDescription ?? "the import failed") }
    let file = try AVAudioFile(forReading: out)
    let format = file.fileFormat
    emit(["event": "done", "tracks": [[
        "name": "others", "source": source.lastPathComponent, "kind": "others", "file": "others.m4a",
        "duration_ms": Int64(Double(file.length) * 1000 / format.sampleRate), "rate": Int(format.sampleRate), "channels": Int(format.channelCount),
    ]]])
}

/// Run an async command, then exit.
func run(_ body: @escaping () async throws -> Void) -> Never {
    Task {
        do { try await body() } catch { fail("\(error.localizedDescription)") }
        exit(0)
    }
    dispatchMain()
}

let args = CommandLine.arguments.dropFirst()
switch args.first {
case "sources":
    do { try printSources() } catch { fail("\(error)") }
case "finalize":
    guard let folder = args.dropFirst().first else { fail("finalize: no folder") }
    let cafs = (try? FileManager.default.contentsOfDirectory(at: URL(fileURLWithPath: folder), includingPropertiesForKeys: nil)) ?? []
    let files = cafs.filter { $0.pathExtension == "caf" }.compactMap(finalize)
    emit(["event": "done", "files": files])
case "import":
    let rest = Array(args.dropFirst())
    guard rest.count == 2 else { fail("import: a file and a folder") }
    do { try importFile(URL(fileURLWithPath: rest[0]), into: URL(fileURLWithPath: rest[1], isDirectory: true)) } catch { fail("\(error)") }
case "model":
    guard let folder = args.dropFirst().first else { fail("model: no folder") }
    let download = args.contains("--download")
    run { try await model(folder: URL(fileURLWithPath: folder, isDirectory: true), download: download) }
case "transcribe":
    var rest = args.dropFirst()
    guard let file = rest.popFirst() else { fail("transcribe: no file") }
    var folder: String?
    var language: String?
    var speakers: URL?
    while let arg = rest.popFirst() {
        switch arg {
        case "--model": folder = value(&rest, for: arg)
        case "--language": language = value(&rest, for: arg)
        case "--speakers": speakers = URL(fileURLWithPath: value(&rest, for: arg), isDirectory: true)
        default: fail("unknown option \(arg)")
        }
    }
    guard let folder else { fail("transcribe: no --model") }
    run { try await transcribe(file: URL(fileURLWithPath: file), folder: URL(fileURLWithPath: folder, isDirectory: true), language: language, speakers: speakers) }
case "record":
    let recording: Recording
    do {
        recording = try Recording(parse(args.dropFirst()))
        try recording.start()
    } catch {
        fail("\(error)")
    }
    let main = DispatchQueue.main
    // "stop" on stdin, or stdin closed: Yoru Hub went away.
    Thread.detachNewThread {
        while let line = readLine() {
            if line.trimmingCharacters(in: .whitespaces) == "stop" { break }
        }
        main.async { recording.finish() }
    }
    for sig in [SIGTERM, SIGINT] {
        signal(sig, SIG_IGN)
        let source = DispatchSource.makeSignalSource(signal: sig, queue: main)
        source.setEventHandler { recording.finish() }
        source.resume()
        signals.append(source)
    }
    let timer = DispatchSource.makeTimerSource(queue: main)
    timer.schedule(deadline: .now() + 0.25, repeating: 0.25)
    timer.setEventHandler { recording.levels() }
    timer.resume()
    keep = timer
    dispatchMain()
default:
    fail("usage: yoru-audio sources | record <folder> [--system | --app <bundle id>] [--mic <uid|default>]... | finalize <folder> | import <file> <folder> | model <folder> [--download] | transcribe <file> --model <folder> [--language <code>] [--speakers <folder>]")
}
