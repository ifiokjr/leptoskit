import Foundation
import LeptosKitBridge

/// Bounds the Rust crate enforces on one request.
private enum Limits {
    /// Rate and pitch are thousandths of the engine's normal value.
    static let thousandthsMin: UInt32 = 500
    static let thousandthsMax: UInt32 = 2000
    static let volumeMax: UInt32 = 1000
    /// Longest accepted strings, in UTF-8 bytes.
    static let textBytes = 4096
    static let languageBytes = 64
    static let voiceBytes = 512
    /// Characters a platform engine speaks in one utterance.
    static let textCharacters = 4000
    /// A voice list starts small and grows on demand.
    static let encoderBytes = 256
}

/// The language tags the Rust crate accepts.
private let languagePattern = #"^[A-Za-z]{2,3}(?:-[A-Za-z]{4})?(?:-(?:[A-Za-z]{2}|[0-9]{3}))?$"#

struct SpeechRequest {
    let text: String
    let rate: Float
    let pitch: Float
    let volume: Float
    let language: String?
    let voice: String?

    init(payload: NativeBuffer) throws {
        self = try payload.withUnsafeBytes { bytes in
            var input = SpeechReader(bytes: bytes)

            guard try input.integer() == 1 else { throw PluginError.invalidArgument }
            let rate = try input.integer()
            let pitch = try input.integer()
            let volume = try input.integer()
            try checkOptions(rate, pitch, volume)

            let text = try input.text(limit: Limits.textBytes)
            let language = try input.text(limit: Limits.languageBytes)
            let voice = try input.text(limit: Limits.voiceBytes)
            try checkScope(text, language, voice, finished: input.finished)

            return SpeechRequest(
                text: text,
                rate: Float(rate) / 1000,
                pitch: Float(pitch) / 1000,
                volume: Float(volume) / 1000,
                language: language.isEmpty ? nil : language,
                voice: voice.isEmpty ? nil : voice
            )
        }
    }

    private init(text: String, rate: Float, pitch: Float, volume: Float, language: String?, voice: String?) {
        self.text = text
        self.rate = rate
        self.pitch = pitch
        self.volume = volume
        self.language = language
        self.voice = voice
    }
}

/// Rejects options the Rust crate would refuse to play.
private func checkOptions(_ rate: UInt32, _ pitch: UInt32, _ volume: UInt32) throws {
    guard (Limits.thousandthsMin...Limits.thousandthsMax).contains(rate),
          (Limits.thousandthsMin...Limits.thousandthsMax).contains(pitch),
          volume <= Limits.volumeMax else {
        throw PluginError.invalidArgument
    }
}

/// Rejects text the engine cannot speak, a request naming two voices, and trailing bytes.
private func checkScope(_ text: String, _ language: String, _ voice: String, finished: Bool) throws {
    guard text.utf16.count <= Limits.textCharacters else { throw PluginError.tooLarge }

    guard !text.isEmpty, finished, language.isEmpty || voice.isEmpty else { throw PluginError.invalidArgument }

    guard language.isEmpty || language.range(of: languagePattern, options: .regularExpression) != nil else {
        throw PluginError.invalidArgument
    }
}

private struct SpeechReader {
    let bytes: UnsafeRawBufferPointer
    var offset = 0
    var finished: Bool { offset == bytes.count }

    mutating func integer() throws -> UInt32 {
        guard bytes.count - offset >= 4 else { throw PluginError.invalidArgument }
        defer { offset += 4 }

        return bytes.loadUnaligned(fromByteOffset: offset, as: UInt32.self).littleEndian
    }

    mutating func text(limit: Int) throws -> String {
        let count = Int(try integer())

        guard count <= limit else { throw PluginError.tooLarge }

        guard count <= bytes.count - offset,
              let text = String(bytes: bytes[offset..<(offset + count)], encoding: .utf8) else {
            throw PluginError.invalidArgument
        }
        offset += count

        return text
    }
}

struct SpeechEncoder {
    var data = Data(capacity: Limits.encoderBytes)

    mutating func integer(_ value: UInt32) {
        var little = value.littleEndian
        withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
    }

    mutating func text(_ text: String) throws {
        let value = Data(text.utf8)

        guard !value.isEmpty else { throw PluginError.native }

        guard value.count <= Limits.voiceBytes else { throw PluginError.tooLarge }
        integer(UInt32(value.count))
        data.append(value)
    }
}
