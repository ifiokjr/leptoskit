import CLeptosKitBridge
import Foundation
import LeptosKitBridge
import LeptosKitSensors
import Testing

@MainActor
private func payload(_ bytes: [UInt8]) throws -> NativeBuffer {
    let bridge = try #require(lk_bridge_create())
    defer { lk_bridge_destroy(bridge) }
    let name = Array("sensor-test".utf8)
    let methods: [UInt32] = [0]
    let plugin = name.withUnsafeBufferPointer { name in
        methods.withUnsafeBufferPointer { methods in
            lk_bridge_register(bridge, name.baseAddress, name.count, methods.baseAddress, methods.count)
        }
    }
    #expect(plugin == 0)
    #expect(lk_bridge_grant(bridge, 0, 0, true) == 0)
    let call = bytes.withUnsafeBufferPointer { lk_bridge_submit(bridge, 0, 0, $0.baseAddress, $0.count) }
    #expect(call > 0)
    var request = lk_request()
    #expect(lk_bridge_next_request(bridge, &request) == 1)

    return NativeBuffer(taking: request.payload)
}

/// A motion source the test drives by hand.
@MainActor
private final class FakeSource: MotionSource {
    var available = true
    var denied = false
    private(set) var starts: [(SensorKind, TimeInterval)] = []
    private(set) var stops: [SensorKind] = []
    private var deliveries: [SensorKind: @Sendable @MainActor (Result<[Double], PluginError>) -> Void] = [:]

    func isAvailable(_ kind: SensorKind) -> Bool { available }

    func authorize(_ kind: SensorKind) throws {
        if denied { throw PluginError.permissionDenied }
    }

    func start(_ kind: SensorKind, interval: TimeInterval,
               deliver: @escaping @Sendable @MainActor (Result<[Double], PluginError>) -> Void) {
        starts.append((kind, interval))
        deliveries[kind] = deliver
    }

    func stop(_ kind: SensorKind) {
        stops.append(kind)
        deliveries[kind] = nil
    }

    func running(_ kind: SensorKind) -> Bool { deliveries[kind] != nil }

    func emit(_ kind: SensorKind, _ values: [Double]) {
        deliveries[kind]?(.success(values))
    }

    func fail(_ kind: SensorKind, _ error: PluginError) {
        deliveries[kind]?(.failure(error))
    }
}

/// Lets queued main-actor work, such as a newly started request, run.
@MainActor
private func settle() async {
    for _ in 0..<20 { await Task.yield() }
}

private func request(_ milliseconds: UInt32) -> Data {
    withUnsafeBytes(of: milliseconds.littleEndian) { Data($0) }
}

private func watch(_ milliseconds: UInt32, after cursor: UInt64) -> Data {
    var bytes = request(milliseconds)
    withUnsafeBytes(of: cursor.littleEndian) { bytes.append(contentsOf: $0) }
    return bytes
}

/// A decoded watch reply.
private struct Batch {
    let cursor: UInt64
    let dropped: UInt32
    let samples: [[Double]]

    init(_ bytes: Data, components: Int = 4) {
        func load<T: FixedWidthInteger>(_ type: T.Type, _ offset: Int) -> T {
            bytes.withUnsafeBytes { T(littleEndian: $0.loadUnaligned(fromByteOffset: offset, as: T.self)) }
        }

        cursor = load(UInt64.self, 0)
        dropped = load(UInt32.self, 8)
        let count = Int(load(UInt32.self, 12))
        #expect(bytes.count == 16 + count * components * 8)
        samples = (0..<count).map { sample in
            (0..<components).map { Double(bitPattern: load(UInt64.self, 16 + (sample * components + $0) * 8)) }
        }
    }
}

private func vector(_ timestamp: Double, x first: Double = 0) -> [Double] { [timestamp, first, 0, 9.80665] }

@Test
@MainActor
func unavailableHardwareAndClosedSensorsNeverFabricateReadings() async throws {
    #if os(macOS) || targetEnvironment(simulator)

    for kind in SensorKind.allCases {
        let plugin = MotionSensorPlugin(name: "test", kind: kind)
        let availability = try await plugin.invoke(method: 0, payload: payload([]))
        #expect(availability == Data([0, 0, 0, 0]))
        await #expect(throws: PluginError.unavailable) {
            try await plugin.invoke(method: 1, payload: payload([16, 0, 0, 0]))
        }
        await #expect(throws: PluginError.unavailable) {
            try await plugin.invoke(method: 2, payload: payload([16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]))
        }
        await #expect(throws: PluginError.invalidArgument) {
            try await plugin.invoke(method: 1, payload: payload([4, 0, 0, 0]))
        }
        await #expect(throws: PluginError.invalidArgument) {
            try await plugin.invoke(method: 0, payload: payload([0]))
        }
        await #expect(throws: PluginError.invalidArgument) {
            try await plugin.invoke(method: 2, payload: payload([16, 0, 0, 0]))
        }
        await #expect(throws: PluginError.unavailable) {
            try await plugin.invoke(method: 9, payload: payload([]))
        }
        plugin.close()
        plugin.close()
        await #expect(throws: PluginError.cancelled) {
            try await plugin.invoke(method: 0, payload: payload([]))
        }
    }

    #endif
}

@Test
@MainActor
func aOneShotReadTakesTheNextSampleAndReleasesTheHardware() async throws {
    let source = FakeSource()
    let channel = SensorChannel(kind: .accelerometer, feeds: MotionFeeds(source: source))
    let read = Task { try await channel.handle(method: 1, payload: request(20)) }
    await settle()
    #expect(source.running(.accelerometer))
    #expect(source.starts.last?.1 == 0.02)
    await #expect(throws: PluginError.busy) { try await channel.handle(method: 1, payload: request(20)) }

    source.emit(.accelerometer, vector(1.5, x: -0.25))
    let bytes = try await read.value
    #expect(bytes.count == 32)
    #expect(bytes.withUnsafeBytes { $0.loadUnaligned(fromByteOffset: 8, as: Double.self) } == -0.25)
    #expect(!source.running(.accelerometer))

    // A sample the wire cannot carry fails the read instead of being sent.
    let invalid = Task { try await channel.handle(method: 1, payload: request(20)) }
    await settle()
    source.emit(.accelerometer, [1, .nan, 0, 0])
    await #expect(throws: PluginError.native) { try await invalid.value }

    // Cancelling the caller stops the hardware too.
    let cancelled = Task { try await channel.handle(method: 1, payload: request(20)) }
    await settle()
    cancelled.cancel()
    await settle()
    #expect(!source.running(.accelerometer))
    await #expect(throws: CancellationError.self) { try await cancelled.value }
}

@Test
@MainActor
func pluginsShareOneFeedAtTheFastestRequestedInterval() async throws {
    let source = FakeSource()
    let feeds = MotionFeeds(source: source)
    let single = SensorChannel(kind: .gyroscope, feeds: feeds)
    let combined = SensorChannel(kind: .gyroscope, feeds: feeds)

    let slow = Task { try await single.handle(method: 1, payload: request(200)) }
    await settle()
    let fast = Task { try await combined.handle(method: 1, payload: request(20)) }
    await settle()
    #expect(source.starts.map(\.1) == [0.2, 0.02])
    #expect(feeds.runningInterval(.gyroscope) == 0.02)

    // One sample answers both readers, and the feed stops once.
    source.emit(.gyroscope, vector(3))
    _ = try await slow.value
    _ = try await fast.value
    #expect(source.stops == [.gyroscope])
}

@Test
@MainActor
func aStreamDeliversEverySampleAfterItsCursorAndCountsWhatItMissed() async throws {
    let source = FakeSource()
    let channel = SensorChannel(kind: .accelerometer, feeds: MotionFeeds(source: source))

    let first = Task { try await channel.handle(method: 2, payload: watch(20, after: 0)) }
    await settle()
    await #expect(throws: PluginError.busy) { try await channel.handle(method: 2, payload: watch(20, after: 0)) }
    source.emit(.accelerometer, vector(1, x: 1))
    let opening = Batch(try await first.value)
    #expect(opening.cursor == 1)
    #expect(opening.samples == [vector(1, x: 1)])

    // Samples that arrive between requests wait in the ring.
    source.emit(.accelerometer, vector(2, x: 2))
    source.emit(.accelerometer, vector(3, x: 3))
    let buffered = Batch(try await channel.handle(method: 2, payload: watch(20, after: opening.cursor)))
    #expect(buffered.cursor == 3)
    #expect(buffered.dropped == 0)
    #expect(buffered.samples.map { $0[1] } == [2, 3])

    // A reader that falls behind gets the newest samples and the count it lost.
    for step in 0..<(SampleRing.capacity + 5) { source.emit(.accelerometer, vector(4 + Double(step), x: Double(step))) }

    let behind = Batch(try await channel.handle(method: 2, payload: watch(20, after: buffered.cursor)))
    #expect(behind.samples.count == SampleRing.capacity)
    #expect(behind.dropped == 5)
    #expect(behind.samples.first?[1] == 5)
    #expect(source.running(.accelerometer))

    // A cursor the channel never issued is refused rather than waited on.
    await #expect(throws: PluginError.invalidArgument) {
        try await channel.handle(method: 2, payload: watch(20, after: behind.cursor + 1))
    }

    // Cancelling the pending request is how a reader says it is done.
    let pending = Task { try await channel.handle(method: 2, payload: watch(20, after: behind.cursor)) }
    await settle()
    pending.cancel()
    await settle()
    #expect(!channel.streaming)
    #expect(!source.running(.accelerometer))
}

@Test
@MainActor
func aStreamStopsWhenItsReaderStopsAsking() async throws {
    let source = FakeSource()
    let channel = SensorChannel(kind: .magnetometer, feeds: MotionFeeds(source: source), lingerFloor: .milliseconds(30))
    let first = Task { try await channel.handle(method: 2, payload: watch(5, after: 0)) }
    await settle()
    source.emit(.magnetometer, vector(1))
    _ = try await first.value
    #expect(source.running(.magnetometer))

    try await Task.sleep(for: .milliseconds(120))
    #expect(!source.running(.magnetometer))
    #expect(!channel.streaming)
}

@Test
@MainActor
func leavingTheAppEndsReadsButStreamsWaitForItsReturn() async throws {
    let source = FakeSource()
    let feeds = MotionFeeds(source: source)
    let channel = SensorChannel(kind: .userAccelerometer, feeds: feeds)
    let stream = Task { try await channel.handle(method: 2, payload: watch(20, after: 0)) }
    let read = Task { try await channel.handle(method: 1, payload: request(20)) }
    await settle()

    feeds.suspend()
    await #expect(throws: PluginError.cancelled) { try await read.value }
    #expect(!source.running(.userAccelerometer))
    await #expect(throws: PluginError.cancelled) { try await channel.handle(method: 1, payload: request(20)) }

    feeds.resume()
    #expect(source.running(.userAccelerometer))
    source.emit(.userAccelerometer, vector(9, x: 0.5))
    #expect(Batch(try await stream.value).samples == [vector(9, x: 0.5)])
    channel.close()
}

@Test
@MainActor
func streamsReportHardwareFailuresAndPermissionDenial() async throws {
    let source = FakeSource()
    let channel = SensorChannel(kind: .barometer, feeds: MotionFeeds(source: source))
    let stream = Task { try await channel.handle(method: 2, payload: watch(200, after: 0)) }
    await settle()
    source.emit(.barometer, [1, 1013.25])
    #expect(Batch(try await stream.value, components: 2).samples == [[1, 1013.25]])

    let failing = Task { try await channel.handle(method: 2, payload: watch(200, after: 1)) }
    await settle()
    source.fail(.barometer, .permissionDenied)
    await #expect(throws: PluginError.permissionDenied) { try await failing.value }
    #expect(!source.running(.barometer))

    source.denied = true
    await #expect(throws: PluginError.permissionDenied) {
        try await channel.handle(method: 2, payload: watch(200, after: 0))
    }
    #expect(!source.running(.barometer))
}

@Test
@MainActor
func theCombinedPluginRoutesEachKindToItsOwnChannel() async throws {
    let source = FakeSource()
    let plugin = MultiSensorPlugin(name: "sensors", feeds: MotionFeeds(source: source))
    #expect(try await plugin.invoke(method: 0, payload: payload([4, 0, 0, 0])) == Data([1, 0, 0, 0]))
    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 0, payload: payload([5, 0, 0, 0]))
    }
    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 0, payload: payload([0, 0]))
    }

    let accelerometer = Task { try await plugin.invoke(method: 1, payload: payload([0, 0, 0, 0, 20, 0, 0, 0])) }
    let gyroscope = Task { try await plugin.invoke(method: 1, payload: payload([2, 0, 0, 0, 20, 0, 0, 0])) }
    await settle()
    #expect(source.running(.accelerometer) && source.running(.gyroscope))
    source.emit(.gyroscope, vector(1, x: 2))
    source.emit(.accelerometer, vector(1, x: 1))
    #expect(try await accelerometer.value.withUnsafeBytes { $0.loadUnaligned(fromByteOffset: 8, as: Double.self) } == 1)
    #expect(try await gyroscope.value.withUnsafeBytes { $0.loadUnaligned(fromByteOffset: 8, as: Double.self) } == 2)

    let magnetometerWatch: [UInt8] = [3, 0, 0, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    let stream = Task { try await plugin.invoke(method: 2, payload: payload(magnetometerWatch)) }
    await settle()
    plugin.close()
    await #expect(throws: PluginError.cancelled) { try await stream.value }
    #expect(!source.running(.magnetometer))
}

@Test
func ringNumbersKeepCountingAcrossRestarts() throws {
    var ring = SampleRing()
    #expect(ring.batch(after: 0) == nil)
    ring.append(Data([1]))
    ring.append(Data([2]))
    ring.clear()
    #expect(ring.batch(after: 2) == nil)
    ring.append(Data([3]))
    let bytes = try #require(ring.batch(after: 2))
    #expect(bytes.count == 17)
    #expect(bytes.last == 3)
    #expect(ring.lastSequence == 3)
}
