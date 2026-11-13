// `yoru-audio talk`: the Mac as Yoru's ears and voice, for the Yoru in the
// notch (docs/notch.md). One AVAudioEngine with Apple's voice processing on,
// so the microphone hears the user without Yoru's own voice from the
// speakers, and the user can talk over him as on the board.
//
// Commands on stdin, one JSON object a line:
//   {"cmd": "start"}               engine on (the microphone indicator lights)
//   {"cmd": "stop"}                engine off, anything queued dropped
//   {"cmd": "mic", "on": bool}     send what the microphone hears, or not
//   {"cmd": "play", "pcm": b64}    queue 16 kHz mono 16-bit PCM to play
//   {"cmd": "mark", "id": n}       say `played` once what is queued so far is heard
//   {"cmd": "flush"}               drop what is queued and playing
// Events on stdout: `ready` once the engine runs, `audio` with 32 ms of
// 16 kHz mono 16-bit PCM (base64) while the microphone is on, `played`
// for a mark, `error`. Closing stdin ends it.

import AVFoundation
import Foundation

final class Talk {
    /// What the board speaks and hears: 16 kHz mono.
    static let rate = 16000.0
    /// 32 ms, as the board's frames.
    static let frame = 512

    private let engine = AVAudioEngine()
    private let player = AVAudioPlayerNode()
    private let speech = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: rate, channels: 1, interleaved: false)!
    private let wire = AVAudioFormat(commonFormat: .pcmFormatInt16, sampleRate: rate, channels: 1, interleaved: true)!
    private var converter: AVAudioConverter?
    private let lock = NSLock()
    private var listening = false
    private var pending: [Int16] = []
    private var running = false

    func handle(_ command: [String: Any]) {
        switch command["cmd"] as? String {
        case "start": start()
        case "stop": stop()
        case "mic":
            lock.lock()
            listening = command["on"] as? Bool ?? false
            pending.removeAll()
            lock.unlock()
        case "play":
            guard let text = command["pcm"] as? String, let data = Data(base64Encoded: text) else { return }
            play(data)
        case "mark":
            let id = command["id"] as? Int ?? 0
            guard running, let marker = AVAudioPCMBuffer(pcmFormat: speech, frameCapacity: 1) else {
                emit(["event": "played", "id": id])
                return
            }
            marker.frameLength = 1
            player.scheduleBuffer(marker, completionCallbackType: .dataPlayedBack) { _ in emit(["event": "played", "id": id]) }
        case "flush":
            guard running else { return }
            player.stop()
            player.play()
        default:
            emit(["event": "error", "message": "talk: unknown command"])
        }
    }

    private func start() {
        guard !running else {
            emit(["event": "ready"])
            return
        }
        askMicrophone()
        let input = engine.inputNode
        do {
            try input.setVoiceProcessingEnabled(true)
        } catch {
            emit(["event": "error", "message": "echo cancelling: \(error.localizedDescription)"])
        }
        // Voice processing ducks every other sound by default: music would
        // drop each time Yoru listens.
        input.voiceProcessingOtherAudioDuckingConfiguration = .init(enableAdvancedDucking: false, duckingLevel: .min)
        if player.engine == nil {
            engine.attach(player)
            engine.connect(player, to: engine.mainMixerNode, format: speech)
        }
        let format = input.outputFormat(forBus: 0)
        converter = AVAudioConverter(from: format, to: wire)
        input.installTap(onBus: 0, bufferSize: 1024, format: format) { [weak self] buffer, _ in self?.heard(buffer) }
        do {
            engine.prepare()
            try engine.start()
        } catch {
            input.removeTap(onBus: 0)
            emit(["event": "error", "message": "the microphone: \(error.localizedDescription)"])
            return
        }
        player.play()
        running = true
        emit(["event": "ready"])
    }

    private func stop() {
        guard running else { return }
        player.stop()
        engine.inputNode.removeTap(onBus: 0)
        engine.stop()
        running = false
        lock.lock()
        pending.removeAll()
        lock.unlock()
    }

    private func heard(_ buffer: AVAudioPCMBuffer) {
        lock.lock()
        let on = listening
        lock.unlock()
        guard on, let converter else { return }
        let ratio = Talk.rate / buffer.format.sampleRate
        let capacity = AVAudioFrameCount(Double(buffer.frameLength) * ratio) + 32
        guard let out = AVAudioPCMBuffer(pcmFormat: wire, frameCapacity: capacity) else { return }
        var given = false
        var error: NSError?
        converter.convert(to: out, error: &error) { _, status in
            if given {
                status.pointee = .noDataNow
                return nil
            }
            given = true
            status.pointee = .haveData
            return buffer
        }
        guard error == nil, let samples = out.int16ChannelData, out.frameLength > 0 else { return }
        lock.lock()
        pending.append(contentsOf: UnsafeBufferPointer(start: samples[0], count: Int(out.frameLength)))
        var frames: [Data] = []
        while pending.count >= Talk.frame {
            frames.append(pending.prefix(Talk.frame).withUnsafeBufferPointer { Data(buffer: $0) })
            pending.removeFirst(Talk.frame)
        }
        lock.unlock()
        for data in frames { emit(["event": "audio", "pcm": data.base64EncodedString()]) }
    }

    private func play(_ data: Data) {
        guard running else { return }
        let count = data.count / 2
        guard count > 0, let buffer = AVAudioPCMBuffer(pcmFormat: speech, frameCapacity: AVAudioFrameCount(count)), let out = buffer.floatChannelData else { return }
        data.withUnsafeBytes { raw in
            let samples = raw.bindMemory(to: Int16.self)
            for i in 0 ..< count { out[0][i] = Float(Int16(littleEndian: samples[i])) / 32768 }
        }
        buffer.frameLength = AVAudioFrameCount(count)
        player.scheduleBuffer(buffer)
    }
}

/// Run `talk` until stdin closes.
func talk() -> Never {
    let session = Talk()
    // Not the main queue: asking for the microphone waits for an answer
    // that macOS may deliver there.
    let queue = DispatchQueue(label: "yoru-audio.talk")
    Thread.detachNewThread {
        while let line = readLine() {
            guard let data = line.data(using: .utf8),
                  let command = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else { continue }
            queue.async { session.handle(command) }
        }
        exit(0)
    }
    dispatchMain()
}
