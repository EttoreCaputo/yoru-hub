// Small wrappers over the Core Audio HAL's property calls.

import CoreAudio
import Foundation

struct AudioError: Error, CustomStringConvertible {
    let description: String
}

/// `status` as an error naming `what`, unless it's `noErr`.
func check(_ status: OSStatus, _ what: String) throws {
    guard status == noErr else { throw AudioError(description: "\(what) failed (\(fourCC(status)))") }
}

/// Core Audio's errors are four-character codes more often than not.
private func fourCC(_ status: OSStatus) -> String {
    let n = UInt32(bitPattern: status)
    let bytes = [24, 16, 8, 0].map { UInt8((n >> $0) & 0xFF) }
    if bytes.allSatisfy({ $0 >= 0x20 && $0 < 0x7F }) { return "'" + String(decoding: bytes, as: UTF8.self) + "'" }
    return String(status)
}

private func address(_ selector: AudioObjectPropertySelector, _ scope: AudioObjectPropertyScope) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress(mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain)
}

func readValue<T>(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector, scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal, initial: T) throws -> T {
    var addr = address(selector, scope)
    var value = initial
    var size = UInt32(MemoryLayout<T>.size)
    try withUnsafeMutablePointer(to: &value) {
        try check(AudioObjectGetPropertyData(object, &addr, 0, nil, &size, $0), "reading a property")
    }
    return value
}

func readArray<T>(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector, scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal, of _: T.Type) throws -> [T] {
    var addr = address(selector, scope)
    var size: UInt32 = 0
    try check(AudioObjectGetPropertyDataSize(object, &addr, 0, nil, &size), "sizing a property")
    let count = Int(size) / MemoryLayout<T>.stride
    guard count > 0 else { return [] }
    let buffer = UnsafeMutableRawPointer.allocate(byteCount: Int(size), alignment: MemoryLayout<T>.alignment)
    defer { buffer.deallocate() }
    try check(AudioObjectGetPropertyData(object, &addr, 0, nil, &size, buffer), "reading a property")
    let typed = buffer.bindMemory(to: T.self, capacity: count)
    return Array(UnsafeBufferPointer(start: typed, count: Int(size) / MemoryLayout<T>.stride))
}

func readString(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector) -> String? {
    var addr = address(selector, kAudioObjectPropertyScopeGlobal)
    var value: Unmanaged<CFString>?
    var size = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
    guard AudioObjectGetPropertyData(object, &addr, 0, nil, &size, &value) == noErr, let value else { return nil }
    return value.takeRetainedValue() as String
}

let systemObject = AudioObjectID(kAudioObjectSystemObject)

func defaultDevice(input: Bool) throws -> AudioDeviceID {
    let selector = input ? kAudioHardwarePropertyDefaultInputDevice : kAudioHardwarePropertyDefaultOutputDevice
    let id = try readValue(systemObject, selector, initial: AudioDeviceID(kAudioObjectUnknown))
    guard id != kAudioObjectUnknown else { throw AudioError(description: input ? "no microphone" : "no output device") }
    return id
}

/// The device whose UID is `uid`.
func device(uid: String) throws -> AudioDeviceID {
    for id in try readArray(systemObject, kAudioHardwarePropertyDevices, of: AudioDeviceID.self) where readString(id, kAudioDevicePropertyDeviceUID) == uid {
        return id
    }
    throw AudioError(description: "no audio device \(uid)")
}

func hasInput(_ device: AudioDeviceID) -> Bool {
    ((try? readArray(device, kAudioDevicePropertyStreams, scope: kAudioObjectPropertyScopeInput, of: AudioStreamID.self)) ?? []).count > 0
}

/// The format of the device's first input stream: what its IOProc hands over.
func inputFormat(_ device: AudioDeviceID) throws -> AudioStreamBasicDescription {
    guard let stream = try readArray(device, kAudioDevicePropertyStreams, scope: kAudioObjectPropertyScopeInput, of: AudioStreamID.self).first else {
        throw AudioError(description: "the device has no input")
    }
    return try readValue(stream, kAudioStreamPropertyVirtualFormat, initial: AudioStreamBasicDescription())
}
