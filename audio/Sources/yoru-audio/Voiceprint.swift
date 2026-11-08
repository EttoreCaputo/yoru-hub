// Voice prints, for telling who speaks to Yoru (docs/memory.md, Knowing who
// speaks): FluidAudio's WeSpeaker embedding, 256 numbers, L2-normalised,
// of one person's speech.

import FluidAudio
import Foundation

/// `yoru-audio voiceprint --model <folder>`: loads the diarizer's models
/// (downloading them into <folder> the first time), says
/// `{"event":"ready"}`, then answers each line on stdin until it closes.
/// A line is JSON: `{"file": <audio file>, "ranges": [[start_ms, end_ms], ...]}`
/// (`ranges` optional: the whole file); the answer is
/// `{"event":"print","embedding":[...]}` or `{"event":"error","message":...}`.
func voiceprint(folder: URL) async throws {
    let models = try await DiarizerModels.download(to: folder.appendingPathComponent("diarizer", isDirectory: true))
    let diarizer = DiarizerManager()
    diarizer.initialize(models: models)
    emit(["event": "ready"])
    let converter = AudioConverter()
    while let line = readLine() {
        guard let data = line.data(using: .utf8),
              let request = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let path = request["file"] as? String
        else {
            if !line.trimmingCharacters(in: .whitespaces).isEmpty { emit(["event": "error", "message": "not a request"]) }
            continue
        }
        do {
            var samples = try converter.resampleAudioFile(URL(fileURLWithPath: path))
            if let ranges = request["ranges"] as? [[Double]], !ranges.isEmpty {
                samples = ranges.flatMap { range -> ArraySlice<Float> in
                    guard range.count == 2 else { return [] }
                    let start = max(0, min(samples.count, Int(range[0] * 16)))
                    let end = max(start, min(samples.count, Int(range[1] * 16)))
                    return samples[start..<end]
                }
            }
            // Under a second says little of a voice.
            guard samples.count >= 16_000 else {
                emit(["event": "error", "message": "too short"])
                continue
            }
            let embedding = try diarizer.extractSpeakerEmbedding(from: samples)
            emit(["event": "print", "embedding": embedding.map { Double($0) }])
        } catch {
            emit(["event": "error", "message": "\(error.localizedDescription)"])
        }
    }
}
