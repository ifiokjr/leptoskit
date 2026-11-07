import Foundation
import LeptosKitBridge

/// The sensors every plugin in this package reads. The raw value is the kind
/// that prefixes each request to the combined `sensors` plugin.
public enum SensorKind: UInt32, CaseIterable, Sendable {
    case accelerometer = 0
    case userAccelerometer = 1
    case gyroscope = 2
    case magnetometer = 3
    case barometer = 4

    /// Doubles in one sample: the timestamp, then each component.
    var components: Int { self == .barometer ? 2 : 4 }

    /// Whether a sample can travel on the wire: finite values, a clock that
    /// has started and, for pressure, a positive reading.
    func accepts(_ values: [Double]) -> Bool {
        guard values.count == components, values.allSatisfy(\.isFinite), values[0] >= 0 else { return false }

        return self != .barometer || values[1] > 0
    }
}

/// The newest samples of one stream, each numbered so a reader can ask for
/// everything after the last one it saw.
///
/// Numbers keep counting across restarts, so a reader's old cursor never
/// matches a sample it has not seen.
public struct SampleRing {
    public static let capacity = 64

    private var samples: [Data] = []
    public private(set) var lastSequence: UInt64 = 0

    public init() {}

    public var isEmpty: Bool { samples.isEmpty }

    /// Keeps a sample, forgetting the oldest once full.
    public mutating func append(_ sample: Data) {
        if samples.count == Self.capacity { samples.removeFirst() }

        samples.append(sample)
        lastSequence += 1
    }

    /// Forgets every sample but keeps counting from the last number.
    public mutating func clear() {
        samples.removeAll()
    }

    /// The reply to a watch from `cursor`: the newest number, how many samples
    /// were lost before the reader asked, the count, then each sample. Nil
    /// when nothing newer than the cursor is held.
    public func batch(after cursor: UInt64) -> Data? {
        guard cursor < lastSequence, !samples.isEmpty else { return nil }

        let oldest = lastSequence - UInt64(samples.count) + 1
        let skip = cursor >= oldest ? Int(cursor - oldest + 1) : 0
        let dropped = cursor + 1 < oldest ? UInt32(clamping: oldest - cursor - 1) : 0
        let kept = samples[skip...]
        var bytes = Data(capacity: 16 + kept.reduce(0) { $0 + $1.count })
        Wire.append(lastSequence, to: &bytes)
        Wire.append(dropped, to: &bytes)
        Wire.append(UInt32(kept.count), to: &bytes)

        for sample in kept { bytes.append(sample) }

        return bytes
    }
}

/// Little-endian encoding shared by every reply.
enum Wire {
    static func append<T: FixedWidthInteger>(_ value: T, to bytes: inout Data) {
        var little = value.littleEndian
        withUnsafeBytes(of: &little) { bytes.append(contentsOf: $0) }
    }

    static func sample(_ values: [Double]) -> Data {
        var bytes = Data(capacity: values.count * 8)

        for value in values { append(value.bitPattern, to: &bytes) }

        return bytes
    }

    static func uint32(_ bytes: Data, at offset: Int) -> UInt32 {
        bytes.withUnsafeBytes { UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: offset, as: UInt32.self)) }
    }

    static func uint64(_ bytes: Data, at offset: Int) -> UInt64 {
        bytes.withUnsafeBytes { UInt64(littleEndian: $0.loadUnaligned(fromByteOffset: offset, as: UInt64.self)) }
    }
}

/// One plugin's reads of one sensor kind: availability, one-shot reads and a
/// stream, all served by the shared [`MotionFeeds`].
///
/// One one-shot read and one stream request can be outstanding at a time.
/// After answering a stream request the sensor keeps running for a short grace
/// period, so a reader that asks again promptly misses nothing. Cancelling a
/// pending stream request, closing the plugin, or letting the grace period
/// lapse stops it.
@MainActor
public final class SensorChannel {
    private struct Waiter {
        let id: UUID
        let continuation: CheckedContinuation<Data, any Error>
        var cursor: UInt64 = 0
    }

    public let kind: SensorKind
    private let feeds: MotionFeeds
    private let lingerFloor: Duration
    private var closed = false

    private var read: Waiter?
    private var readToken: MotionFeeds.Token?

    private var watcher: Waiter?
    private var streamToken: MotionFeeds.Token?
    private var streamInterval: TimeInterval = 0
    private var ring = SampleRing()
    private var linger: Task<Void, Never>?

    /// Accepted sampling interval, in milliseconds. The fast end avoids
    /// Android's high-rate permission.
    private static let intervals: ClosedRange<UInt32> = 5...60_000

    public init(kind: SensorKind, feeds: MotionFeeds, lingerFloor: Duration = .seconds(2)) {
        self.kind = kind
        self.feeds = feeds
        self.lingerFloor = lingerFloor
    }

    /// Answers availability (`0`), one fresh sample (`1`) or a stream batch (`2`).
    public func handle(method: UInt32, payload: Data) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        switch method {
        case 0: return try availability(payload)
        case 1: return try await next(payload)
        case 2: return try await watch(payload)
        default: throw PluginError.unavailable
        }
    }

    /// Fails everything outstanding and stops the hardware this channel holds.
    public func close() {
        closed = true
        finishRead(.failure(PluginError.cancelled))
        stopStream(PluginError.cancelled)
    }

    private func availability(_ payload: Data) throws -> Data {
        guard payload.isEmpty else { throw PluginError.invalidArgument }

        return Data([feeds.isAvailable(kind) ? 1 : 0, 0, 0, 0])
    }

    private static func interval(_ payload: Data) throws -> TimeInterval {
        let milliseconds = Wire.uint32(payload, at: 0)

        guard intervals.contains(milliseconds) else { throw PluginError.invalidArgument }

        return Double(milliseconds) / 1000
    }

    // MARK: One-shot reads

    /// Waits for one fresh sample, then releases the hardware.
    private func next(_ payload: Data) async throws -> Data {
        guard payload.count == 4 else { throw PluginError.invalidArgument }

        let interval = try Self.interval(payload)

        guard feeds.isAvailable(kind) else { throw PluginError.unavailable }

        guard read == nil else { throw PluginError.busy }

        try Task.checkCancellation()

        // A one-shot read started while the app is away would never resolve.
        guard feeds.active else { throw PluginError.cancelled }

        try feeds.authorize(kind)
        let id = UUID()

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                read = Waiter(id: id, continuation: continuation)
                readToken = feeds.subscribe(kind, interval: interval) { [weak self] event in
                    self?.readEvent(event, id: id)
                }
            }
        } onCancel: { [weak self] in
            Task { @MainActor in self?.cancelRead(id) }
        }
    }

    private func readEvent(_ event: MotionFeeds.Event, id: UUID) {
        guard read?.id == id else { return }

        switch event {
        case .sample(let values): finishRead(.success(Wire.sample(values)))
        case .failed(let error): finishRead(.failure(error))
        case .suspended: finishRead(.failure(PluginError.cancelled))
        }
    }

    private func cancelRead(_ id: UUID) {
        guard read?.id == id else { return }

        finishRead(.failure(CancellationError()))
    }

    private func finishRead(_ result: Result<Data, any Error>) {
        if let readToken { feeds.unsubscribe(readToken) }

        readToken = nil

        guard let waiter = read else { return }

        read = nil
        waiter.continuation.resume(with: result)
    }

    // MARK: Streams

    /// Answers with every sample after the cursor, waiting for one if needed.
    private func watch(_ payload: Data) async throws -> Data {
        guard payload.count == 12 else { throw PluginError.invalidArgument }

        let interval = try Self.interval(payload)
        let requested = Wire.uint64(payload, at: 4)

        guard feeds.isAvailable(kind) else { throw PluginError.unavailable }

        guard watcher == nil else { throw PluginError.busy }

        // A cursor this channel never issued would wait forever.
        guard requested <= ring.lastSequence else { throw PluginError.invalidArgument }

        try Task.checkCancellation()
        try feeds.authorize(kind)
        linger?.cancel()
        linger = nil
        startStream(interval: interval)
        // Zero asks for samples from now on, not ones a previous reader left.
        let cursor = requested == 0 ? ring.lastSequence : requested

        if let batch = ring.batch(after: cursor) {
            scheduleLinger()

            return batch
        }

        let id = UUID()

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                watcher = Waiter(id: id, continuation: continuation, cursor: cursor)
            }
        } onCancel: { [weak self] in
            Task { @MainActor in self?.cancelWatch(id) }
        }
    }

    /// Subscribes the stream, or retunes it to a new interval.
    private func startStream(interval: TimeInterval) {
        if let streamToken {
            if streamInterval != interval { feeds.retune(streamToken, interval: interval) }
        } else {
            streamToken = feeds.subscribe(kind, interval: interval) { [weak self] event in
                self?.streamEvent(event)
            }
        }

        streamInterval = interval
    }

    private func streamEvent(_ event: MotionFeeds.Event) {
        switch event {
        case .sample(let values):
            ring.append(Wire.sample(values))

            guard let waiter = watcher, let batch = ring.batch(after: waiter.cursor) else { return }

            watcher = nil
            scheduleLinger()
            waiter.continuation.resume(returning: batch)
        case .failed(let error):
            stopStream(error)
        case .suspended:
            // The feed restarts when the app returns; the reader keeps waiting.
            break
        }
    }

    private func cancelWatch(_ id: UUID) {
        guard watcher?.id == id else { return }

        stopStream(CancellationError())
    }

    /// Keeps the sensor running a little while for the reader's next request.
    private func scheduleLinger() {
        let wait = max(lingerFloor, .milliseconds(Int64(streamInterval * 2000)))
        linger?.cancel()
        linger = Task { [weak self] in
            try? await Task.sleep(for: wait)

            guard !Task.isCancelled else { return }

            self?.lingerLapsed()
        }
    }

    private func lingerLapsed() {
        guard watcher == nil else { return }

        stopStream(PluginError.cancelled)
    }

    /// Releases the hardware and fails any waiting reader with `error`.
    private func stopStream(_ error: any Error) {
        linger?.cancel()
        linger = nil

        if let streamToken { feeds.unsubscribe(streamToken) }

        streamToken = nil
        ring.clear()

        guard let waiter = watcher else { return }

        watcher = nil
        waiter.continuation.resume(throwing: error)
    }

    /// Whether the stream currently holds the hardware, for tests.
    public var streaming: Bool { streamToken != nil }
}
