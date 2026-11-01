import Foundation
import LeptosKitBridge
#if os(iOS)
import UIKit
#endif

@MainActor
public final class BrightnessPlugin: NativePlugin {
    public let name = "brightness"
    public let methods: [UInt32] = [0, 1, 2]

    /// Brightness crosses the wire as thousandths of a unit fraction.
    private static let thousandthsPerUnit: CGFloat = 1000

    /// Only `set` carries a level, and it is exactly one u32.
    private static let levelPayloadBytes = 4

    private let restoreOnClose: Bool
    private var closed = false

    #if os(iOS)
    private var original: CGFloat?
    private var screen: UIScreen?
    #endif

    /// iOS brightness is screen-wide. Restoration defaults to on; an app must
    /// explicitly opt out when intentionally retaining its override after close.
    public init(restoreOnClose: Bool = true) {
        self.restoreOnClose = restoreOnClose
    }

    #if os(iOS)
    /// Answers the current level, applies a new one, or restores the baseline.
    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        try Self.checkArguments(method: method, payload: payload)

        switch method {
        case 0:
            let target = try activeScreen()

            return try Self.encode(level: target.brightness)
        case 1:
            let target = try activeScreen()

            try setLevel(payload, on: target)
        default:
            restore()
        }

        return Data()
    }
    #else
    /// Brightness needs UIScreen, so every call reports the absent capability.
    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        try Self.checkArguments(method: method, payload: payload)

        throw PluginError.unavailable
    }
    #endif

    #if os(iOS)
    /// Stops serving calls and puts the screen back when the app asked for it.
    public func close() {
        guard !closed else { return }
        closed = true

        if restoreOnClose { restore() }

        screen = nil
        original = nil
    }
    #else
    /// Nothing was overridden here, so closing only stops serving calls.
    public func close() {
        closed = true
    }
    #endif

    /// Only `set` carries a level, and no method takes more than one u32.
    private static func checkArguments(method: UInt32, payload: NativeBuffer) throws {
        guard method <= 2 else { throw PluginError.unavailable }

        guard payload.count == (method == 1 ? levelPayloadBytes : 0) else {
            throw PluginError.invalidArgument
        }
    }

    #if os(iOS)
    /// The screen the override applies to: the one already overridden, else the
    /// foreground-active scene's screen.
    private func activeScreen() throws -> UIScreen {
        if let screen { return screen }

        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        let active = scenes.first { $0.activationState == .foregroundActive }?.screen

        guard let active else { throw PluginError.unavailable }

        return active
    }

    /// Encodes the level as thousandths, refusing a value iOS cannot mean.
    private static func encode(level: CGFloat) throws -> Data {
        guard level.isFinite, (0...1).contains(level) else { throw PluginError.native }

        var encoded = UInt32((level * thousandthsPerUnit).rounded()).littleEndian

        return withUnsafeBytes(of: &encoded) { Data($0) }
    }

    /// Applies a thousandths level, remembering the window's baseline once.
    private func setLevel(_ payload: NativeBuffer, on target: UIScreen) throws {
        let level = payload.withUnsafeBytes { UInt32(littleEndian: $0.loadUnaligned(as: UInt32.self)) }

        guard level <= UInt32(Self.thousandthsPerUnit) else { throw PluginError.invalidArgument }

        if original == nil { original = target.brightness }

        screen = target
        target.brightness = CGFloat(level) / Self.thousandthsPerUnit
    }

    /// Puts the saved level back and forgets it. Safe to repeat.
    private func restore() {
        if let original, let screen { screen.brightness = original }

        original = nil
        screen = nil
    }
    #endif
}
