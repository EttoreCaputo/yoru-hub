// One recorded track: a device's input, written to a file as it comes.

import AVFoundation
import CoreAudio

final class Track {
    /// `mic`, `mic-2`…, or `others` for the tap.
    let name: String
    /// What it records, for people: a microphone's name, an app's, "System audio".
    let source: String
    let kind: String
    let device: AudioDeviceID
    let format: AVAudioFormat
    let url: URL
    private var file: AVAudioFile?
    private var proc: AudioDeviceIOProcID?
    private let queue: DispatchQueue
    private let lock = NSLock()
    private var frames: Int64 = 0
    private var peak: Float = 0

    init(name: String, source: String, kind: String, device: AudioDeviceID, stream: AudioStreamBasicDescription, folder: URL) throws {
        var asbd = stream
        guard let format = AVAudioFormat(streamDescription: &asbd), format.commonFormat == .pcmFormatFloat32 else {
            throw AudioError(description: "\(source): unsupported sample format")
        }
        self.name = name
        self.source = source
        self.kind = kind
        self.device = device
        self.format = format
        url = folder.appendingPathComponent("\(name).caf")
        queue = DispatchQueue(label: "yoru-audio.\(name)", qos: .userInitiated)
        // Speech in AAC; ALAC where AAC can't go (above 48 kHz or more than
        // two channels). CAF keeps what was written if the recording is cut short.
        let channels = Int(format.channelCount)
        let aac = format.sampleRate <= 48000 && channels <= 2
        var settings: [String: Any] = [
            AVFormatIDKey: aac ? kAudioFormatMPEG4AAC : kAudioFormatAppleLossless,
            AVSampleRateKey: format.sampleRate,
            AVNumberOfChannelsKey: channels,
        ]
        if aac { settings[AVEncoderBitRateKey] = channels > 1 ? 96000 : 64000 }
        file = try AVAudioFile(forWriting: url, settings: settings, commonFormat: .pcmFormatFloat32, interleaved: format.isInterleaved)
    }

    func start() throws {
        // With a queue the HAL calls the block on it, off the real-time
        // thread, so writing the file there is fine.
        try check(AudioDeviceCreateIOProcIDWithBlock(&proc, device, queue) { [weak self] _, input, _, _, _ in
            self?.write(input)
        }, "\(source): opening the input")
        try check(AudioDeviceStart(device, proc), "\(source): starting")
    }

    func stop() {
        if let proc {
            AudioDeviceStop(device, proc)
            AudioDeviceDestroyIOProcID(device, proc)
        }
        proc = nil
        // Wait for a block still running, then close the file.
        queue.sync { file = nil }
    }

    private func write(_ input: UnsafePointer<AudioBufferList>) {
        guard input.pointee.mNumberBuffers > 0 else { return }
        // A device with several input streams hands them all over; the
        // format is the first one's.
        if format.isInterleaved && input.pointee.mNumberBuffers > 1 {
            var first = AudioBufferList(mNumberBuffers: 1, mBuffers: input.pointee.mBuffers)
            withUnsafePointer(to: &first) { write(list: $0) }
        } else {
            write(list: input)
        }
    }

    private func write(list: UnsafePointer<AudioBufferList>) {
        guard let file, let buffer = AVAudioPCMBuffer(pcmFormat: format, bufferListNoCopy: list, deallocator: nil), buffer.frameLength > 0 else { return }
        do {
            try file.write(from: buffer)
        } catch {
            emit(["event": "error", "message": "\(source): \(error.localizedDescription)"])
            return
        }
        let level = Track.peak(buffer)
        lock.lock()
        frames += Int64(buffer.frameLength)
        peak = max(peak, level)
        lock.unlock()
    }

    /// The loudest sample since the last call, 0 to 1.
    func takePeak() -> Float {
        lock.lock()
        defer { peak = 0; lock.unlock() }
        return min(peak, 1)
    }

    var durationMs: Int64 {
        lock.lock()
        defer { lock.unlock() }
        return frames * 1000 / Int64(format.sampleRate)
    }

    private static func peak(_ buffer: AVAudioPCMBuffer) -> Float {
        guard let data = buffer.floatChannelData else { return 0 }
        let count = Int(buffer.frameLength) * (buffer.format.isInterleaved ? Int(buffer.format.channelCount) : 1)
        var top: Float = 0
        for i in 0 ..< count { top = max(top, abs(data[0][i])) }
        return top
    }

    var info: [String: Any] {
        ["name": name, "source": source, "kind": kind, "rate": Int(format.sampleRate), "channels": Int(format.channelCount)]
    }
}
