// What the computer plays, through a Core Audio process tap (macOS 14.2+):
// all of it, or one app's. A tap is read through a private aggregate device
// that holds it, clocked by the output device.

import AVFoundation
import CoreAudio

final class Tap {
    let tap: AudioObjectID
    let aggregate: AudioDeviceID
    let format: AudioStreamBasicDescription

    /// Every process when `processes` is nil, else only those.
    init(processes: [AudioObjectID]?) throws {
        let description = processes.map { CATapDescription(stereoMixdownOfProcesses: $0) }
            ?? CATapDescription(stereoGlobalTapButExcludeProcesses: [])
        description.uuid = UUID()
        description.name = "Yoru"
        description.isPrivate = true
        // The user keeps hearing it.
        description.muteBehavior = .unmuted
        var tap = AudioObjectID(kAudioObjectUnknown)
        try check(AudioHardwareCreateProcessTap(description, &tap), "creating the audio tap")
        self.tap = tap

        let output = try defaultDevice(input: false)
        guard let outputUID = readString(output, kAudioDevicePropertyDeviceUID) else {
            AudioHardwareDestroyProcessTap(tap)
            throw AudioError(description: "the output device has no UID")
        }
        let aggregateDescription: [String: Any] = [
            kAudioAggregateDeviceNameKey: "Yoru recording",
            kAudioAggregateDeviceUIDKey: UUID().uuidString,
            kAudioAggregateDeviceMainSubDeviceKey: outputUID,
            kAudioAggregateDeviceIsPrivateKey: true,
            kAudioAggregateDeviceIsStackedKey: false,
            kAudioAggregateDeviceTapAutoStartKey: true,
            kAudioAggregateDeviceSubDeviceListKey: [[kAudioSubDeviceUIDKey: outputUID]],
            kAudioAggregateDeviceTapListKey: [[kAudioSubTapDriftCompensationKey: true, kAudioSubTapUIDKey: description.uuid.uuidString]],
        ]
        var aggregate = AudioDeviceID(kAudioObjectUnknown)
        do {
            try check(AudioHardwareCreateAggregateDevice(aggregateDescription as CFDictionary, &aggregate), "creating the recording device")
            format = try readValue(tap, kAudioTapPropertyFormat, initial: AudioStreamBasicDescription())
        } catch {
            if aggregate != kAudioObjectUnknown { AudioHardwareDestroyAggregateDevice(aggregate) }
            AudioHardwareDestroyProcessTap(tap)
            throw error
        }
        self.aggregate = aggregate
    }

    /// Call once its track has stopped.
    func destroy() {
        AudioHardwareDestroyAggregateDevice(aggregate)
        AudioHardwareDestroyProcessTap(tap)
    }
}
