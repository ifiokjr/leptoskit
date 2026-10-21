import Foundation
import LeptosKitBridge

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
            guard (500...2000).contains(rate), (500...2000).contains(pitch), volume <= 1000 else {
                throw PluginError.invalidArgument
            }
            let text = try input.text(limit: 4096)
            let language = try input.text(limit: 64)
            let voice = try input.text(limit: 512)
            guard text.utf16.count <= 4000 else { throw PluginError.tooLarge }
            guard !text.isEmpty, input.finished,
                  language.isEmpty || voice.isEmpty else { throw PluginError.invalidArgument }
            let pattern = #"^[A-Za-z]{2,3}(?:-[A-Za-z]{4})?(?:-(?:[A-Za-z]{2}|[0-9]{3}))?$"#
            guard language.isEmpty || language.range(of: pattern, options: .regularExpression) != nil else {
                throw PluginError.invalidArgument
            }
            return SpeechRequest(text: text, rate: Float(rate) / 1000, pitch: Float(pitch) / 1000,
                                 volume: Float(volume) / 1000, language: language.isEmpty ? nil : language,
                                 voice: voice.isEmpty ? nil : voice)
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
    var data = Data(capacity: 256)

    mutating func integer(_ value: UInt32) {
        var little = value.littleEndian
        withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
    }

    mutating func text(_ text: String) throws {
        let value = Data(text.utf8)
        guard !value.isEmpty else { throw PluginError.native }
        guard value.count <= 512 else { throw PluginError.tooLarge }
        integer(UInt32(value.count))
        data.append(value)
    }
}
