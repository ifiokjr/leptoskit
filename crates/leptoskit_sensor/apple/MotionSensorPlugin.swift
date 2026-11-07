import Foundation
import LeptosKitBridge

/// A plugin for one sensor kind: availability (`0`), one fresh sample (`1`)
/// and a stream (`2`). The `accelerometer`, `gyroscope`, `magnetometer` and
/// `barometer` plugins are this class with their kind filled in.
@MainActor
open class MotionSensorPlugin: NativePlugin {
    public typealias Kind = SensorKind

    public let name: String
    public let methods: [UInt32] = [0, 1, 2]
    private let channel: SensorChannel

    public init(name: String, kind: Kind, feeds: MotionFeeds = .shared, lingerFloor: Duration = .seconds(2)) {
        self.name = name
        channel = SensorChannel(kind: kind, feeds: feeds, lingerFloor: lingerFloor)
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        // Requests are at most a dozen bytes, so a copy costs nothing.
        try await channel.handle(method: method, payload: payload.withUnsafeBytes { Data($0) })
    }

    public func close() {
        channel.close()
    }
}

/// The combined `sensors` plugin: every request starts with a little-endian
/// u32 [`SensorKind`], followed by the same request a single-kind plugin takes.
@MainActor
open class MultiSensorPlugin: NativePlugin {
    public let name: String
    public let methods: [UInt32] = [0, 1, 2]
    private let channels: [SensorKind: SensorChannel]

    public init(name: String, feeds: MotionFeeds = .shared, lingerFloor: Duration = .seconds(2)) {
        self.name = name
        channels = Dictionary(uniqueKeysWithValues: SensorKind.allCases.map {
            ($0, SensorChannel(kind: $0, feeds: feeds, lingerFloor: lingerFloor))
        })
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        let bytes = payload.withUnsafeBytes { Data($0) }

        guard bytes.count >= 4, let kind = SensorKind(rawValue: Wire.uint32(bytes, at: 0)),
              let channel = channels[kind] else {
            throw PluginError.invalidArgument
        }

        return try await channel.handle(method: method, payload: Data(bytes.dropFirst(4)))
    }

    public func close() {
        for channel in channels.values { channel.close() }
    }
}
