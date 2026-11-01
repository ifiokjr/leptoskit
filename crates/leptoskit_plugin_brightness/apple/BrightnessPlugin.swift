import Foundation
import LeptosKitBridge
#if os(iOS)
import UIKit
#endif

@MainActor
public final class BrightnessPlugin: NativePlugin {
    public let name = "brightness"
    public let methods: [UInt32] = [0, 1, 2]
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

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard method <= 2 else { throw PluginError.unavailable }

        guard payload.count == (method == 1 ? 4 : 0) else { throw PluginError.invalidArgument }

        #if os(iOS)

        if method == 2 {
            restore()

            return Data()
        }

        guard let current = screen ?? UIApplication.shared.connectedScenes
            .compactMap({ $0 as? UIWindowScene })
            .first(where: { $0.activationState == .foregroundActive })?.screen else {
            throw PluginError.unavailable
        }

        if method == 0 {
            let level = current.brightness

            guard level.isFinite, (0...1).contains(level) else { throw PluginError.native }
            var encoded = UInt32((level * 1000).rounded()).littleEndian

            return withUnsafeBytes(of: &encoded) { Data($0) }
        }

        let level = payload.withUnsafeBytes { UInt32(littleEndian: $0.loadUnaligned(as: UInt32.self)) }

        guard level <= 1000 else { throw PluginError.invalidArgument }

        if original == nil { original = current.brightness }
        screen = current
        current.brightness = CGFloat(level) / 1000

        return Data()
        #else
        throw PluginError.unavailable

        #endif
    }

    public func close() {
        guard !closed else { return }
        closed = true

        #if os(iOS)

        if restoreOnClose { restore() }
        screen = nil
        original = nil
        #endif
    }

    #if os(iOS)
    private func restore() {
        if let original, let screen { screen.brightness = original }
        original = nil
        screen = nil
    }
    #endif
}
