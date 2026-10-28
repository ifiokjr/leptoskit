import Foundation
import LeptosKitBridge
import QuartzCore
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

/// The arena's native module: launch scenario, system-log report, a frame
/// clock, and the ArenaEcho methods. Method ids match `arena_bridge`'s Rust
/// contract.
@MainActor
public final class ArenaPlugin: NativePlugin {
    public let name = "arena"
    public let methods: [UInt32] = [0, 1, 2, 10, 11, 12, 13, 14]
    private let frames = FrameClock()

    public init() {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        switch method {
        case 0: return Data(Self.scenario().utf8)
        case 1:
            for line in try payload.utf8().split(separator: "\n") { NSLog("%@", String(line)) }
            return Data()
        case 2: return await frames.next()
        case 10:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return Data()
        case 11:
            guard payload.count == 4 else { throw PluginError.invalidArgument }
            let value = payload.withUnsafeBytes { Int32(littleEndian: $0.loadUnaligned(as: Int32.self)) }
            return withUnsafeBytes(of: (value &+ 1).littleEndian) { Data($0) }
        case 12: return Data(try payload.utf8().utf8)
        case 13: return payload.withUnsafeBytes { Data($0) }
        case 14:
            let object = try payload.withUnsafeBytes { try JSONSerialization.jsonObject(with: Data($0)) }
            guard let fields = object as? [String: Any] else { throw PluginError.invalidArgument }
            return try JSONSerialization.data(withJSONObject: fields)
        default: throw PluginError.unavailable
        }
    }

    public func close() { frames.close() }

    /// `--scenario <id>` from the process arguments.
    private static func scenario() -> String {
        let arguments = ProcessInfo.processInfo.arguments
        guard let flag = arguments.firstIndex(of: "--scenario"), flag + 1 < arguments.count else { return "" }
        return arguments[flag + 1]
    }
}

/// Resumes every waiting call on the next display-link callback, and pauses
/// the link while nobody waits.
@MainActor
private final class FrameClock: NSObject {
    private var waiting: [CheckedContinuation<Data, Never>] = []
    private var link: CADisplayLink?

    func next() async -> Data {
        await withCheckedContinuation { continuation in
            waiting.append(continuation)
            start()
        }
    }

    func close() {
        link?.invalidate()
        link = nil
        let pending = waiting
        waiting.removeAll()
        for continuation in pending { continuation.resume(returning: Data()) }
    }

    private func start() {
        if let link {
            link.isPaused = false
            return
        }
        #if os(iOS)
        let link = CADisplayLink(target: self, selector: #selector(tick(_:)))
        #else
        guard let screen = NSScreen.main else { return }
        let link = screen.displayLink(target: self, selector: #selector(tick(_:)))
        #endif
        link.add(to: .main, forMode: .common)
        self.link = link
    }

    @objc private func tick(_ link: CADisplayLink) {
        let timestamp = UInt64(link.timestamp * 1_000_000_000)
        let period = UInt64(max(link.targetTimestamp - link.timestamp, 0) * 1_000_000_000)
        var reply = Data()
        withUnsafeBytes(of: timestamp.littleEndian) { reply.append(contentsOf: $0) }
        withUnsafeBytes(of: period.littleEndian) { reply.append(contentsOf: $0) }
        let pending = waiting
        waiting.removeAll()
        link.isPaused = true
        for continuation in pending { continuation.resume(returning: reply) }
    }
}
