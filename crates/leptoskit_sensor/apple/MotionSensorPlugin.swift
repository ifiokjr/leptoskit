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
    #endif

    public init(name: String, kind: Kind) {
        self.name = name
        self.kind = kind
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        if method == 0 {
            guard payload.count == 0 else { throw PluginError.invalidArgument }

            return Data([available ? 1 : 0, 0, 0, 0])
        }

        guard method == 1 else { throw PluginError.unavailable }
        let interval = try payload.withUnsafeBytes { bytes -> UInt32 in

            guard bytes.count == 4 else { throw PluginError.invalidArgument }
            let value = UInt32(littleEndian: bytes.loadUnaligned(as: UInt32.self))

            guard (5...60_000).contains(value) else { throw PluginError.invalidArgument }

            return value
        }

        guard available else { throw PluginError.unavailable }

        guard pending == nil else { throw PluginError.busy }

        try Task.checkCancellation()
        try preflight()
        let identifier = UUID()

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                pending = continuation
                pendingID = identifier
                start(interval: Double(interval) / 1000, identifier: identifier)
            }
        } onCancel: { [weak self] in
            Task { @MainActor in

                guard self?.pendingID == identifier else { return }
                self?.finish(.failure(CancellationError()))
            }
        }
    }

    public func close() {
        closed = true
        finish(.failure(PluginError.cancelled))
    }

    private func preflight() throws {
        #if os(iOS)

        guard UIApplication.shared.applicationState == .active else { throw PluginError.cancelled }
        // Raw accelerometer, gyroscope and magnetometer samples need no
        // authorization. The altimeter is motion data that iOS gates behind
        // NSMotionUsageDescription and the user's motion permission.
        if kind == .barometer {
            guard let description = Bundle.main.object(forInfoDictionaryKey: "NSMotionUsageDescription") as? String,
                  !description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw PluginError.permissionDenied
            }
            let status = CMAltimeter.authorizationStatus()

            guard status != .denied && status != .restricted else { throw PluginError.permissionDenied }
        }

        #endif
    }

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

    private func start(interval: Double, identifier: UUID) {
        #if os(iOS)
        backgroundObserver = NotificationCenter.default.addObserver(
            forName: UIApplication.willResignActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.finish(.failure(PluginError.cancelled)) }
        }
        switch kind {
        case .accelerometer:
            motion.accelerometerUpdateInterval = interval
            motion.startAccelerometerUpdates(to: .main) { [weak self] sample, error in
                let values = sample.map { [$0.timestamp, $0.acceleration.x * 9.80665,
                                           $0.acceleration.y * 9.80665, $0.acceleration.z * 9.80665] }
                MainActor.assumeIsolated { self?.receive(values, error: error, identifier: identifier) }
            }
        case .gyroscope:
            motion.gyroUpdateInterval = interval
            motion.startGyroUpdates(to: .main) { [weak self] sample, error in
                let values = sample.map { [$0.timestamp, $0.rotationRate.x, $0.rotationRate.y, $0.rotationRate.z] }
                MainActor.assumeIsolated { self?.receive(values, error: error, identifier: identifier) }
            }
        case .magnetometer:
            motion.magnetometerUpdateInterval = interval
            motion.startMagnetometerUpdates(to: .main) { [weak self] sample, error in
                let values = sample.map { [$0.timestamp, $0.magneticField.x, $0.magneticField.y, $0.magneticField.z] }
                MainActor.assumeIsolated { self?.receive(values, error: error, identifier: identifier) }
            }
        case .barometer:
            // CoreMotion owns the barometer rate; the requested interval is a hint only.
            altimeter.startRelativeAltitudeUpdates(to: .main) { [weak self] sample, error in
                let values = sample.map { [$0.timestamp, $0.pressure.doubleValue * 10] }
                MainActor.assumeIsolated { self?.receive(values, error: error, identifier: identifier) }
            }
        }
        #else
        finish(.failure(PluginError.unavailable))
        #endif
    }

    #if os(iOS)
    private func receive(_ values: [Double]?, error: (any Error)?, identifier: UUID) {
        guard pendingID == identifier else { return }

        if let error {
            let native = error as NSError
            let denied = native.domain == CMErrorDomain
                && [CMErrorMotionActivityNotAuthorized, CMErrorNotAuthorized].contains {
                    native.code == Int($0.rawValue)
                }
            finish(.failure(denied ? PluginError.permissionDenied : PluginError.native))

            return
        }

        guard let values, values.allSatisfy(\.isFinite), let timestamp = values.first, timestamp >= 0,
              kind != .barometer || values[1] > 0 else {

            finish(.failure(PluginError.native))

            return
        }
        var bytes = Data(capacity: values.count * 8)

        for value in values {
            var bits = value.bitPattern.littleEndian
            withUnsafeBytes(of: &bits) { bytes.append(contentsOf: $0) }
        }

        finish(.success(bytes))
    }
    #endif

    private func finish(_ result: Result<Data, any Error>) {
        guard let continuation = pending else { return }
        pending = nil
        pendingID = nil

        #if os(iOS)
        motion.stopAccelerometerUpdates()
        motion.stopGyroUpdates()
        motion.stopMagnetometerUpdates()
        altimeter.stopRelativeAltitudeUpdates()

        if let backgroundObserver { NotificationCenter.default.removeObserver(backgroundObserver) }
        backgroundObserver = nil
        #endif
        continuation.resume(with: result)
    }
}
