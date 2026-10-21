import Foundation
import LeptosKitBridge

@MainActor
public final class LocalizationPlugin: NativePlugin {
    public let name = "localization"
    public let methods: [UInt32] = [0]

    public init() {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard method == 0 else { throw PluginError.unavailable }
        guard payload.count == 0 else { throw PluginError.invalidArgument }
        let languages = Locale.preferredLanguages
        guard languages.count <= 64 else { throw PluginError.tooLarge }
        var output = PreferenceEncoder()
        output.integer(1)
        output.integer(UInt32(languages.count))
        for tag in languages {
            let locale = Locale(identifier: tag)
            try output.text(tag)
            try output.text(locale.language.languageCode?.identifier)
            try output.text(locale.region?.identifier)
        }
        let calendar = Calendar.current
        let identifier = String(describing: calendar.identifier)
        try output.text(identifier == "gregorian" ? "gregory" : identifier)
        try output.text(TimeZone.current.identifier)
        switch Locale.current.hourCycle {
        case .zeroToEleven, .oneToTwelve: output.integer(1)
        case .zeroToTwentyThree, .oneToTwentyFour: output.integer(2)
        @unknown default: output.integer(0)
        }
        output.integer(UInt32((calendar.firstWeekday + 5) % 7 + 1))
        return output.bytes
    }
}

private struct PreferenceEncoder {
    var bytes = Data(capacity: 128)

    mutating func integer(_ value: UInt32) {
        var little = value.littleEndian
        withUnsafeBytes(of: &little) { bytes.append(contentsOf: $0) }
    }

    mutating func text(_ value: String?) throws {
        let utf8 = Data((value ?? "").utf8)
        guard utf8.count <= 256 else { throw PluginError.tooLarge }
        integer(UInt32(utf8.count))
        bytes.append(utf8)
    }
}
