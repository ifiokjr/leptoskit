import Foundation
import LeptosKitBridge
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

@MainActor
public final class HapticsPlugin: NativePlugin {
    public let name = "haptics"
    public let methods: [UInt32] = [0, 1, 2, 3]

    #if os(iOS)
    private let selection = UISelectionFeedbackGenerator()
    private let notification = UINotificationFeedbackGenerator()
    private let impacts: [UIImpactFeedbackGenerator] = [
        .init(style: .light), .init(style: .medium), .init(style: .heavy),
        .init(style: .rigid), .init(style: .soft)
    ]
    #endif

    public init() {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard method <= 2 else { throw PluginError.unavailable }
        let value: UInt32
        if method == 0 {
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            value = 0
        } else {
            guard payload.count == 4 else { throw PluginError.invalidArgument }
            value = payload.withUnsafeBytes { UInt32(littleEndian: $0.loadUnaligned(as: UInt32.self)) }
            guard value <= (method == 1 ? 4 : 2) else { throw PluginError.invalidArgument }
        }
        #if os(iOS)
        perform(method, value)
        #elseif os(macOS)
        let pattern: NSHapticFeedbackManager.FeedbackPattern = switch method {
        case 0: .alignment
        case 1: .levelChange
        default: .generic
        }
        NSHapticFeedbackManager.defaultPerformer.perform(pattern, performanceTime: .now)
        #else
        throw PluginError.unavailable
        #endif
        return Data()
    }

    #if os(iOS)
    private func perform(_ method: UInt32, _ value: UInt32) {
        switch method {
        case 0:
            selection.selectionChanged()
        case 1:
            impacts[Int(value)].impactOccurred()
        default:
            let kinds: [UINotificationFeedbackGenerator.FeedbackType] = [.success, .warning, .error]
            notification.notificationOccurred(kinds[Int(value)])
        }
    }
    #endif
}
