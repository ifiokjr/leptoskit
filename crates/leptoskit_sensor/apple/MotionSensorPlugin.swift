import Foundation
import LeptosKitBridge
#if os(iOS)
import CoreMotion
import UIKit
#endif

/// One hardware acquisition per outstanding call. No native sample queue is retained.
@MainActor
open class MotionSensorPlugin: NativePlugin {
    public enum Kind: Sendable {
        case accelerometer, gyroscope, magnetometer, barometer
    }

    public let name: String
    public let methods: [UInt32] = [0, 1]
    private let kind: Kind
    private var closed = false
    private var pending: CheckedContinuation<Data, any Error>?
    private var pendingID: UUID?

    #if os(iOS)
    private let motion = CMMotionManager()
    private let altimeter = CMAltimeter()
    private var backgroundObserver: NSObjectProtocol?

    /// Research-grade gravity, the unit accelerometers report in.
    private static let standardGravity = 9.80665

    /// CoreMotion reports kilopascals; the wire carries hectopascals.
    private static let hectopascalsPerKilopascal = 10.0

    /// Bytes in one f64 on the wire.
    private static let bytesPerDouble = 8

    /// The CoreMotion codes that mean the user withheld motion access.
    private static let deniedCodes: Set<Int> = [
        Int(CMErrorMotionActivityNotAuthorized.rawValue),
        Int(CMErrorNotAuthorized.rawValue)
    ]
    #endif

    public init(name: String, kind: Kind) {
        self.name = name
        self.kind = kind
    }

    /// Answers availability (`0`) or blocks for one fresh sample (`1`).
    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        switch method {
        case 0: return try availability(payload)
        case 1: return try await next(payload)
        default: throw PluginError.unavailable
        }
    }

    /// Reports whether the platform exposes this sensor; takes no argument.
    private func availability(_ payload: NativeBuffer) throws -> Data {
        guard payload.count == 0 else { throw PluginError.invalidArgument }

        return Data([available ? 1 : 0, 0, 0, 0])
    }

    /// Blocks until one sample satisfies the requested interval.
    private func next(_ payload: NativeBuffer) async throws -> Data {
        let interval = try readInterval(from: payload)

        guard available else { throw PluginError.unavailable }

        guard pending == nil else { throw PluginError.busy }

        try Task.checkCancellation()
        try preflight()
        let identifier = UUID()

        return try await waitForSample(interval: interval, identifier: identifier)
    }

    /// Reads the milliseconds the caller asked for, bounded by what the OS accepts.
    private func readInterval(from payload: NativeBuffer) throws -> UInt32 {
        try payload.withUnsafeBytes { bytes -> UInt32 in

            guard bytes.count == 4 else { throw PluginError.invalidArgument }

            let value = UInt32(littleEndian: bytes.loadUnaligned(as: UInt32.self))

            guard (5...60_000).contains(value) else { throw PluginError.invalidArgument }

            return value
        }
    }

    /// Bridges the continuation to the native update stream, cancellably.
    private func waitForSample(interval: UInt32, identifier: UUID) async throws -> Data {
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                pending = continuation
                pendingID = identifier
                start(interval: Double(interval) / 1000, identifier: identifier)
            }
        } onCancel: { [weak self] in
            Task { @MainActor in
                self?.cancelPending(identifier)
            }
        }
    }

    /// Fails the outstanding read when the caller cancels it.
    private func cancelPending(_ identifier: UUID) {
        guard pendingID == identifier else { return }

        finish(.failure(CancellationError()))
    }

    public func close() {
        closed = true
        finish(.failure(PluginError.cancelled))
    }

    #if os(iOS)
    /// Checks the app may ask before any motion data is read.
    private func preflight() throws {
        guard UIApplication.shared.applicationState == .active else { throw PluginError.cancelled }
        // Raw accelerometer, gyroscope and magnetometer samples need no
        // authorization. The altimeter is motion data that iOS gates behind
        // NSMotionUsageDescription and the user's motion permission.
        if kind == .barometer { try requireAltimeterPermission() }
    }

    /// The altimeter needs a usage description and the user's motion grant.
    private func requireAltimeterPermission() throws {
        guard let description = Bundle.main.object(forInfoDictionaryKey: "NSMotionUsageDescription") as? String,
              !description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw PluginError.permissionDenied
        }

        let status = CMAltimeter.authorizationStatus()

        guard status != .denied && status != .restricted else { throw PluginError.permissionDenied }
    }
    #else
    /// No Apple platform besides iOS exposes motion samples to apps.
    private func preflight() throws {
        throw PluginError.unavailable
    }
    #endif

    private var available: Bool {
        #if os(iOS)
        switch kind {
        case .accelerometer: motion.isAccelerometerAvailable
        case .gyroscope: motion.isGyroAvailable
        case .magnetometer: motion.isMagnetometerAvailable
        case .barometer: CMAltimeter.isRelativeAltitudeAvailable()
        }
        #else
        false
        #endif
    }

    #if os(iOS)
    /// Starts the update stream for this kind and finishes when the app leaves.
    private func start(interval: Double, identifier: UUID) {
        backgroundObserver = NotificationCenter.default.addObserver(
            forName: UIApplication.willResignActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.finish(.failure(PluginError.cancelled)) }
        }

        switch kind {
        case .accelerometer: startAccelerometer(interval: interval, identifier: identifier)
        case .gyroscope: startGyroscope(interval: interval, identifier: identifier)
        case .magnetometer: startMagnetometer(interval: interval, identifier: identifier)
        case .barometer: startAltimeter(identifier: identifier)
        }
    }

    /// Accelerometer samples arrive in g; the wire carries m/s².
    private func startAccelerometer(interval: Double, identifier: UUID) {
        motion.accelerometerUpdateInterval = interval
        motion.startAccelerometerUpdates(to: .main) { [weak self] sample, error in
            let values = sample.map { [$0.timestamp, $0.acceleration.x * Self.standardGravity,
                                       $0.acceleration.y * Self.standardGravity,
                                       $0.acceleration.z * Self.standardGravity] }

            MainActor.assumeIsolated { self?.receive(values, error: error, identifier: identifier) }
        }
    }

    private func startGyroscope(interval: Double, identifier: UUID) {
        motion.gyroUpdateInterval = interval
        motion.startGyroUpdates(to: .main) { [weak self] sample, error in
            let values = sample.map { [$0.timestamp, $0.rotationRate.x, $0.rotationRate.y, $0.rotationRate.z] }

            MainActor.assumeIsolated { self?.receive(values, error: error, identifier: identifier) }
        }
    }

    private func startMagnetometer(interval: Double, identifier: UUID) {
        motion.magnetometerUpdateInterval = interval
        motion.startMagnetometerUpdates(to: .main) { [weak self] sample, error in
            let values = sample.map { [$0.timestamp, $0.magneticField.x, $0.magneticField.y, $0.magneticField.z] }

            MainActor.assumeIsolated { self?.receive(values, error: error, identifier: identifier) }
        }
    }

    /// CoreMotion owns the barometer rate; the requested interval is a hint only.
    private func startAltimeter(identifier: UUID) {
        altimeter.startRelativeAltitudeUpdates(to: .main) { [weak self] sample, error in
            let values = sample.map { [$0.timestamp, $0.pressure.doubleValue * Self.hectopascalsPerKilopascal] }

            MainActor.assumeIsolated { self?.receive(values, error: error, identifier: identifier) }
        }
    }
    #else
    /// No update stream exists without CoreMotion.
    private func start(interval: Double, identifier: UUID) {
        finish(.failure(PluginError.unavailable))
    }
    #endif

    #if os(iOS)
    /// Answers the waiting read with a sample, a mapped error or a refused one.
    private func receive(_ values: [Double]?, error: (any Error)?, identifier: UUID) {
        guard pendingID == identifier else { return }

        if let error {
            finish(.failure(Self.map(error)))

            return
        }

        guard let values, values.allSatisfy(\.isFinite), let timestamp = values.first, timestamp >= 0,
              kind != .barometer || values[1] > 0 else {
            finish(.failure(PluginError.native))

            return
        }

        finish(.success(Self.encode(values)))
    }

    /// A sample is a timestamp followed by every component, each little-endian.
    private static func encode(_ values: [Double]) -> Data {
        var bytes = Data(capacity: values.count * bytesPerDouble)

        for value in values { append(value, to: &bytes) }

        return bytes
    }

    /// Appends one f64 in the little-endian order the wire expects.
    private static func append(_ value: Double, to bytes: inout Data) {
        var bits = value.bitPattern.littleEndian

        withUnsafeBytes(of: &bits) { bytes.append(contentsOf: $0) }
    }

    /// Permission failures are reported as such; every other error is native.
    private static func map(_ error: any Error) -> PluginError {
        let native = error as NSError
        let denied = native.domain == CMErrorDomain && deniedCodes.contains(native.code)

        return denied ? .permissionDenied : .native
    }
    #endif

    #if os(iOS)
    /// Answers the waiting call once and stops the native update stream.
    private func finish(_ result: Result<Data, any Error>) {
        guard let continuation = pending else { return }

        pending = nil
        pendingID = nil
        stopUpdates()
        continuation.resume(with: result)
    }

    /// Stops every stream this plugin could have started.
    private func stopUpdates() {
        motion.stopAccelerometerUpdates()
        motion.stopGyroUpdates()
        motion.stopMagnetometerUpdates()
        altimeter.stopRelativeAltitudeUpdates()

        if let backgroundObserver { NotificationCenter.default.removeObserver(backgroundObserver) }

        backgroundObserver = nil
    }
    #else
    /// Answers the waiting call once; nothing was started elsewhere.
    private func finish(_ result: Result<Data, any Error>) {
        guard let continuation = pending else { return }

        pending = nil
        pendingID = nil
        continuation.resume(with: result)
    }
    #endif
}
