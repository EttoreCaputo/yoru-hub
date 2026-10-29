// Transcription on this computer: NVIDIA Parakeet (the Ultra post-training
// of TDT 0.6B v3, 25 European languages) on the Neural Engine, and who
// speaks when with NVIDIA's Nemotron 3 Diarization (up to 8 speakers),
// through FluidAudio. The models are in folders the caller picks (Yoru
// Hub's models/).

import FluidAudio
import Foundation

private let version: AsrModelVersion = .ultra

/// `yoru-audio model <folder> [--download]`: whether the model is in
/// <folder>, and with `--download` fetch it first (about 630 MB).
func model(folder: URL, download: Bool) async throws {
    if download && !AsrModels.modelsExist(at: folder, version: version) {
        try await AsrModels.download(to: folder, version: version) { progress in
            emit(["event": "progress", "phase": "download", "fraction": progress.fractionCompleted])
        }
    }
    emit(["event": "model", "present": AsrModels.modelsExist(at: folder, version: version)])
}

/// The most accurate preset, and the one that counts the speakers right most
/// often; its 10 s latency doesn't matter for a finished recording.
private let speakersPreset: Nemotron3Config = .fast128

/// `yoru-audio transcribe <file> --model <folder> [--language it] [--speakers <folder>]`:
/// the file's words as segments, `done` with `segments` at the end. With
/// `--speakers`, each segment says who speaks (`speaker`, 0 for the first
/// heard), the diarization model kept in (or fetched to, about 190 MB) that folder.
func transcribe(file: URL, folder: URL, language: String?, speakers: URL?) async throws {
    guard AsrModels.modelsExist(at: folder, version: version) else { throw AudioError(description: "the transcription model isn't downloaded") }
    let models = try await AsrModels.load(from: folder, version: version)
    let manager = AsrManager(config: ASRConfig(tdtConfig: TdtConfig(blankId: version.blankId), encoderHiddenSize: version.encoderHiddenSize))
    try await manager.loadModels(models)
    var state = TdtDecoderState.make(decoderLayers: await manager.decoderLayerCount)
    // Only files longer than one window (15 s) report progress.
    let progress = await manager.transcriptionProgressStream
    let reporter = Task {
        for try await fraction in progress {
            emit(["event": "progress", "phase": "transcribe", "fraction": fraction])
        }
    }
    defer { reporter.cancel() }
    // The hint only keeps the decoder in the language's script; Parakeet
    // finds the language on its own.
    let result = try await manager.transcribe(file, decoderState: &state, language: language.flatMap(Language.init(rawValue:)))
    var heard = words(result.tokenTimings ?? [])
    if let speakers, !heard.isEmpty {
        emit(["event": "progress", "phase": "speakers", "fraction": 0])
        let models = try await Nemotron3Models.loadFromHuggingFace(config: speakersPreset, cacheDirectory: speakers) { progress in
            emit(["event": "progress", "phase": "download", "fraction": progress.fractionCompleted])
        }
        let diarizer = Nemotron3Diarizer(config: speakersPreset, models: models)
        let audio = try AudioConverter().resampleAudioFile(file)
        let (probabilities, frames) = try diarizer.processComplete(audio)
        attribute(&heard, probabilities: probabilities, frames: frames, slots: speakersPreset.numSpeakers)
        emit(["event": "progress", "phase": "speakers", "fraction": 1])
    }
    emit(["event": "done", "segments": segments(heard).map(\.json)])
}

/// Give each word the speaker most active over it (10 ms frames, `slots`
/// speakers each), or the one before it when nobody is; then number the
/// speakers in the order they are first heard.
func attribute(_ words: inout [Word], probabilities: [Float], frames: Int, slots: Int) {
    var last: Int?
    for i in words.indices {
        let first = max(0, min(frames - 1, Int(words[i].start * 100)))
        let end = max(first + 1, min(frames, Int((words[i].end * 100).rounded(.up))))
        var activity = [Float](repeating: 0, count: slots)
        for frame in first ..< end {
            for slot in 0 ..< slots { activity[slot] += probabilities[frame * slots + slot] }
        }
        let best = activity.indices.max { activity[$0] < activity[$1] }
        if let best, activity[best] > 0.5 { last = best }
        words[i].speaker = last
    }
    var order: [Int: Int] = [:]
    for i in words.indices {
        guard let slot = words[i].speaker else { continue }
        if order[slot] == nil { order[slot] = order.count }
        words[i].speaker = order[slot]
    }
}

struct Word {
    var text: String
    var start: TimeInterval
    var end: TimeInterval
    var speaker: Int?
}

/// Parakeet's tokens into words: a token that starts with a space starts one.
func words(_ tokens: [TokenTiming]) -> [Word] {
    var words: [Word] = []
    for token in tokens {
        let text = token.token.trimmingCharacters(in: .whitespacesAndNewlines)
        if token.token.first?.isWhitespace == true || words.isEmpty {
            if !text.isEmpty { words.append(Word(text: text, start: token.startTime, end: token.endTime, speaker: nil)) }
        } else {
            words[words.count - 1].text += text
            words[words.count - 1].end = token.endTime
        }
    }
    return words
}

struct Segment {
    var text: String
    var start: TimeInterval
    var end: TimeInterval
    var speaker: Int?

    init(_ word: Word) {
        text = word.text
        start = word.start
        end = word.end
        speaker = word.speaker
    }

    var json: [String: Any] {
        var json: [String: Any] = ["text": text, "start_ms": Int((start * 1000).rounded()), "end_ms": Int((end * 1000).rounded())]
        if let speaker { json["speaker"] = speaker }
        return json
    }
}

/// Words into segments the way people read a transcript: a new one when
/// someone else speaks, after a pause, after a sentence once the segment
/// has some length (fluent speech has no pauses between sentences), or when
/// one runs too long.
func segments(_ words: [Word], pause: TimeInterval = 1.2, enough: TimeInterval = 8, longest: TimeInterval = 25) -> [Segment] {
    var out: [Segment] = []
    var current: Segment?
    for word in words {
        if var segment = current {
            let ended = segment.text.last.map { ".?!…".contains($0) } ?? false
            let length = segment.end - segment.start
            if word.speaker != segment.speaker || word.start - segment.end > pause || (ended && (length > enough || word.start - segment.end > 0.3)) || length > longest {
                out.append(segment)
                current = Segment(word)
            } else {
                segment.text += " " + word.text
                segment.end = word.end
                current = segment
            }
        } else {
            current = Segment(word)
        }
    }
    if let current { out.append(current) }
    return out
}
