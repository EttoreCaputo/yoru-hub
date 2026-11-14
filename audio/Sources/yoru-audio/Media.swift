// Other apps' music and videos, paused while Yoru uses the Mac's audio in
// the notch and started again once he lets it go. Through MediaRemote,
// the private framework behind the media keys and Control Center: its
// commands still reach the app that plays (reading what plays is closed
// to other apps since macOS 15.4, so CoreAudio says whether anything does).

import Foundation

enum Media {
    private typealias Send = @convention(c) (UInt32, CFDictionary?) -> Bool
    private static let send: Send? = {
        guard let remote = dlopen("/System/Library/PrivateFrameworks/MediaRemote.framework/MediaRemote", RTLD_NOW),
              let symbol = dlsym(remote, "MRMediaRemoteSendCommand") else { return nil }
        return unsafeBitCast(symbol, to: Send.self)
    }()
    /// MediaRemote's commands.
    private static let play: UInt32 = 0
    private static let pause: UInt32 = 1

    /// Pause what another app plays. True when something played and the
    /// pause was sent: only then is it started again ([`resume`]), so
    /// music the user had stopped stays stopped.
    static func pauseOthers() -> Bool {
        let own = Bundle.main.bundleIdentifier
        let playing = ((try? audioApps()) ?? []).filter { $0.playing && $0.id != own }
        guard !playing.isEmpty, let send else { return false }
        return send(pause, nil)
    }

    static func resume() {
        _ = send?(play, nil)
    }
}
