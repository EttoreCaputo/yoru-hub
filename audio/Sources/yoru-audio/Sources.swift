// What can be recorded: the apps that use audio and the microphones.

import AppKit
import CoreAudio
import Darwin

/// An app with audio processes; a browser's sound comes from its helpers,
/// which count as the app's own.
struct AudioApp {
    let id: String
    let name: String
    var processes: [AudioObjectID]
    var playing: Bool
    let icon: NSImage?
}

/// `image` as a PNG `size` pixels wide, base64: sharp on a Retina screen at the picker's 44 points.
private func png(_ image: NSImage, size: CGFloat = 128) -> String? {
    guard let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(size), pixelsHigh: Int(size), bitsPerSample: 8, samplesPerPixel: 4,
                                     hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)
    else { return nil }
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    image.draw(in: NSRect(x: 0, y: 0, width: size, height: size))
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])?.base64EncodedString()
}

/// The path of the executable of `pid`.
private func executablePath(_ pid: pid_t) -> String? {
    var buffer = [CChar](repeating: 0, count: Int(MAXPATHLEN) * 4)
    guard proc_pidpath(pid, &buffer, UInt32(buffer.count)) > 0 else { return nil }
    return String(cString: buffer)
}

/// Some names carry direction marks (WhatsApp's starts with U+200E).
private func displayName(_ name: String) -> String {
    name.trimmingCharacters(in: CharacterSet.whitespacesAndNewlines.union(CharacterSet(charactersIn: "\u{200E}\u{200F}")))
}

/// Every running app that has used audio since it started, by bundle id.
func audioApps() throws -> [AudioApp] {
    let apps = NSWorkspace.shared.runningApplications.filter { $0.activationPolicy == .regular && $0.bundleIdentifier != nil }
    var found: [String: AudioApp] = [:]
    for process in try readArray(systemObject, kAudioHardwarePropertyProcessObjectList, of: AudioObjectID.self) {
        guard let pid = try? readValue(process, kAudioProcessPropertyPID, initial: pid_t(-1)), pid > 0, pid != getpid() else { continue }
        // The app itself, or the app whose bundle holds this helper.
        let path = executablePath(pid)
        guard let app = apps.first(where: { $0.processIdentifier == pid })
            ?? apps.first(where: { app in app.bundleURL.map { path?.hasPrefix($0.path + "/") ?? false } ?? false }),
            let id = app.bundleIdentifier
        else { continue }
        let playing = ((try? readValue(process, kAudioProcessPropertyIsRunningOutput, initial: UInt32(0))) ?? 0) != 0
        found[id, default: AudioApp(id: id, name: displayName(app.localizedName ?? id), processes: [], playing: false, icon: app.icon)].processes.append(process)
        found[id]!.playing = found[id]!.playing || playing
    }
    return found.values.sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
}

struct Microphone {
    let uid: String
    let name: String
    let isDefault: Bool
}

func microphones() throws -> [Microphone] {
    let fallback = try? defaultDevice(input: true)
    return try readArray(systemObject, kAudioHardwarePropertyDevices, of: AudioDeviceID.self).compactMap { id in
        guard hasInput(id), let uid = readString(id, kAudioDevicePropertyDeviceUID) else { return nil }
        // Aggregates made by other apps for their own recordings, not microphones.
        if uid.hasPrefix("CADefaultDeviceAggregate") { return nil }
        return Microphone(uid: uid, name: readString(id, kAudioObjectPropertyName) ?? uid, isDefault: id == fallback)
    }
}

/// `yoru-audio sources`: the apps and microphones as one JSON object.
func printSources() throws {
    let apps = try audioApps().map { app -> [String: Any] in
        var entry: [String: Any] = ["id": app.id, "name": app.name, "playing": app.playing]
        if let icon = app.icon.flatMap({ png($0) }) { entry["icon"] = icon }
        return entry
    }
    let mics = try microphones().map { ["id": $0.uid, "name": $0.name, "default": $0.isDefault] as [String: Any] }
    emit(["apps": apps, "microphones": mics])
}
