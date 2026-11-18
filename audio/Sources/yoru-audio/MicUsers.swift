// Which apps use the microphone: how Yoru Hub tells that a call started
// (docs/meetings.md, Detection). Core Audio knows it per process since
// macOS 14 (`kAudioProcessPropertyIsRunningInput`), with no permission.

import AppKit
import CoreAudio

/// The apps with a process taking sound from a microphone now, by bundle id.
func microphoneUsers() -> [[String: String]] {
    let apps = (try? audioApps()) ?? []
    return apps
        .filter { app in app.processes.contains { ((try? readValue($0, kAudioProcessPropertyIsRunningInput, initial: UInt32(0))) ?? 0) != 0 } }
        .map { ["id": $0.id, "name": $0.name] }
}

private var watch: Timer?

/// `yoru-audio mic-users`: a `mic_users` line with the apps now, then one
/// whenever they change, until stdin closes (Yoru Hub went away).
func watchMicrophoneUsers() {
    Thread.detachNewThread {
        while readLine() != nil {}
        exit(0)
    }
    var last: [[String: String]]?
    let look = {
        let now = microphoneUsers()
        if now != last {
            last = now
            emit(["event": "mic_users", "apps": now])
        }
    }
    look()
    // A run loop, not dispatchMain: the running apps' list is kept current
    // by notifications that need one.
    watch = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { _ in look() }
    RunLoop.main.run()
}
