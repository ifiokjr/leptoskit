import Foundation
import LeptosKitBridge
#if os(iOS)
import CoreMotion
import UIKit
#endif

/// Where motion samples come from: CoreMotion on iOS, and a stand-in in tests.
///
/// Values are SI: a monotonic timestamp in seconds, then each component.
@MainActor
public protocol MotionSource: AnyObject {
    func isAvailable(_ kind: SensorKind) -> Bool
    /// Throws when the app may not read this kind, for example without a usage description.
    func authorize(_ kind: SensorKind) throws
    /// Starts delivery, or retunes it when it is already running.
    func start(_ kind: SensorKind, interval: TimeInterval,
               deliver: @escaping @Sendable @MainActor (Result<[Double], PluginError>) -> Void)
    func stop(_ kind: SensorKind)
}

/// Shares one hardware feed per sensor kind among every plugin that reads it.
///
/// Apple asks apps to keep a single motion manager, and a feed can only have
/// one handler, so plugins subscribe here instead of starting CoreMotion
/// themselves. A feed runs at the fastest interval any subscriber asked for.
/// While the app is inactive every feed stops; one-shot readers are told to
/// give up and streams wait for the app to return.
@MainActor
public final class MotionFeeds {
    public enum Event {
        case sample([Double])
        case failed(PluginError)
        case suspended
    }

    /// Identifies one subscription for retuning and removal.
    public final class Token {
        let kind: SensorKind
        fileprivate var interval: TimeInterval
        fileprivate let handler: @MainActor (Event) -> Void

        fileprivate init(kind: SensorKind, interval: TimeInterval, handler: @escaping @MainActor (Event) -> Void) {
            self.kind = kind
            self.interval = interval
            self.handler = handler
        }
    }

    public static let shared: MotionFeeds = {
        #if os(iOS)
        let feeds = MotionFeeds(source: CoreMotionSource(),
                                active: UIApplication.shared.applicationState == .active)
        feeds.observeApplication()
        return feeds
        #else
        return MotionFeeds(source: UnavailableMotionSource())
        #endif
    }()

    private let source: MotionSource
    private var subscribers: [SensorKind: [ObjectIdentifier: Token]] = [:]
    private var running: [SensorKind: TimeInterval] = [:]
    private var observers: [NSObjectProtocol] = []
    public private(set) var active: Bool

    public init(source: MotionSource, active: Bool = true) {
        self.source = source
        self.active = active
    }

    public func isAvailable(_ kind: SensorKind) -> Bool {
        source.isAvailable(kind)
    }

    public func authorize(_ kind: SensorKind) throws {
        try source.authorize(kind)
    }

    /// The interval the hardware currently runs at, if it runs.
    public func runningInterval(_ kind: SensorKind) -> TimeInterval? {
        running[kind]
    }

    /// Adds a consumer, starting the hardware or speeding it up.
    public func subscribe(_ kind: SensorKind, interval: TimeInterval,
                          handler: @escaping @MainActor (Event) -> Void) -> Token {
        let token = Token(kind: kind, interval: interval, handler: handler)
        subscribers[kind, default: [:]][ObjectIdentifier(token)] = token
        reconcile(kind)

        return token
    }

    /// Changes how often a consumer wants samples.
    public func retune(_ token: Token, interval: TimeInterval) {
        guard subscribers[token.kind]?[ObjectIdentifier(token)] != nil else { return }

        token.interval = interval
        reconcile(token.kind)
    }

    /// Removes a consumer, slowing or stopping the hardware.
    public func unsubscribe(_ token: Token) {
        subscribers[token.kind]?[ObjectIdentifier(token)] = nil
        reconcile(token.kind)
    }

    /// Stops every feed while the app is not in use.
    public func suspend() {
        guard active else { return }

        active = false

        for kind in SensorKind.allCases { reconcile(kind) }

        for token in subscribers.values.flatMap(\.values) { token.handler(.suspended) }
    }

    /// Restarts the feeds someone still wants.
    public func resume() {
        guard !active else { return }

        active = true

        for kind in SensorKind.allCases { reconcile(kind) }
    }

    /// Runs the hardware at the fastest interval asked for, or stops it.
    private func reconcile(_ kind: SensorKind) {
        let wanted = active ? subscribers[kind]?.values.map(\.interval).min() : nil

        guard wanted != running[kind] else { return }

        guard let wanted else {
            running[kind] = nil
            source.stop(kind)

            return
        }

        running[kind] = wanted
        source.start(kind, interval: wanted) { [weak self] result in
            self?.deliver(kind, result)
        }
    }

    /// Hands one validated sample, or the failure, to every consumer.
    private func deliver(_ kind: SensorKind, _ result: Result<[Double], PluginError>) {
        guard active, running[kind] != nil else { return }

        let event: Event
        switch result {
        case .success(let values) where kind.accepts(values): event = .sample(values)
        case .success: event = .failed(.native)
        case .failure(let error): event = .failed(error)
        }

        // A handler may unsubscribe, so walk a copy.
        let tokens = subscribers[kind].map { Array($0.values) } ?? []

        for token in tokens { token.handler(event) }
    }

    #if os(iOS)
    /// Follows the app in and out of use.
    private func observeApplication() {
        let center = NotificationCenter.default
        observers.append(center.addObserver(forName: UIApplication.willResignActiveNotification,
                                            object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.suspend() }
        })
        observers.append(center.addObserver(forName: UIApplication.didBecomeActiveNotification,
                                            object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.resume() }
        })
    }
    #endif
}

/// Platforms without app-readable motion hardware.
@MainActor
public final class UnavailableMotionSource: MotionSource {
    public init() {}

    public func isAvailable(_ kind: SensorKind) -> Bool { false }

    public func authorize(_ kind: SensorKind) throws { throw PluginError.unavailable }

    public func start(_ kind: SensorKind, interval: TimeInterval,
                      deliver: @escaping @Sendable @MainActor (Result<[Double], PluginError>) -> Void) {
        deliver(.failure(.unavailable))
    }

    public func stop(_ kind: SensorKind) {}
}

#if os(iOS)
/// CoreMotion behind one motion manager and one altimeter for the whole app.
///
/// Acceleration is reported in m/s² on Android's axes: CoreMotion's units of
/// gravity are scaled by standard gravity and negated, so a phone lying face up
/// reads about +9.8 on z on every platform.
@MainActor
final class CoreMotionSource: MotionSource {
    private let motion = CMMotionManager()
    private let altimeter = CMAltimeter()

    /// Converts CoreMotion's units of gravity to m/s² on Android's axes.
    private nonisolated static let gravityToAndroid = -9.80665

    /// CoreMotion reports kilopascals; the wire carries hectopascals.
    private nonisolated static let hectopascalsPerKilopascal = 10.0

    /// The CoreMotion codes that mean the user withheld motion access.
    private nonisolated static let deniedCodes: Set<Int> = [
        Int(CMErrorMotionActivityNotAuthorized.rawValue),
        Int(CMErrorNotAuthorized.rawValue)
    ]

    func isAvailable(_ kind: SensorKind) -> Bool {
        switch kind {
        case .accelerometer: motion.isAccelerometerAvailable
        case .userAccelerometer: motion.isDeviceMotionAvailable
        case .gyroscope: motion.isGyroAvailable
        case .magnetometer: motion.isMagnetometerAvailable
        case .barometer: CMAltimeter.isRelativeAltitudeAvailable()
        }
    }

    /// Raw accelerometer, gyroscope, magnetometer and device-motion samples
    /// need no authorization. The altimeter is motion data that iOS gates
    /// behind NSMotionUsageDescription and the user's motion permission.
    func authorize(_ kind: SensorKind) throws {
        guard kind == .barometer else { return }

        guard let description = Bundle.main.object(forInfoDictionaryKey: "NSMotionUsageDescription") as? String,
              !description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw PluginError.permissionDenied
        }

        let status = CMAltimeter.authorizationStatus()

        guard status != .denied && status != .restricted else { throw PluginError.permissionDenied }
    }

    func start(_ kind: SensorKind, interval: TimeInterval,
               deliver: @escaping @Sendable @MainActor (Result<[Double], PluginError>) -> Void) {
        switch kind {
        case .accelerometer:
            motion.accelerometerUpdateInterval = interval
            motion.startAccelerometerUpdates(to: .main) { sample, error in
                let values = sample.map { [$0.timestamp] + Self.android($0.acceleration) }
                MainActor.assumeIsolated { deliver(Self.result(values, error)) }
            }
        case .userAccelerometer:
            motion.deviceMotionUpdateInterval = interval
            motion.startDeviceMotionUpdates(to: .main) { sample, error in
                let values = sample.map { [$0.timestamp] + Self.android($0.userAcceleration) }
                MainActor.assumeIsolated { deliver(Self.result(values, error)) }
            }
        case .gyroscope:
            motion.gyroUpdateInterval = interval
            motion.startGyroUpdates(to: .main) { sample, error in
                let values = sample.map { [$0.timestamp, $0.rotationRate.x, $0.rotationRate.y, $0.rotationRate.z] }
                MainActor.assumeIsolated { deliver(Self.result(values, error)) }
            }
        case .magnetometer:
            motion.magnetometerUpdateInterval = interval
            motion.startMagnetometerUpdates(to: .main) { sample, error in
                let values = sample.map { [$0.timestamp, $0.magneticField.x, $0.magneticField.y, $0.magneticField.z] }
                MainActor.assumeIsolated { deliver(Self.result(values, error)) }
            }
        case .barometer:
            // CoreMotion owns the altimeter's rate; the interval is a hint only.
            // Starting again would only replace the handler, so leave it running.
            guard !altimeterRunning else { return }

            altimeterRunning = true
            altimeter.startRelativeAltitudeUpdates(to: .main) { sample, error in
                let values = sample.map { [$0.timestamp, $0.pressure.doubleValue * Self.hectopascalsPerKilopascal] }
                MainActor.assumeIsolated { deliver(Self.result(values, error)) }
            }
        }
    }

    private var altimeterRunning = false

    func stop(_ kind: SensorKind) {
        switch kind {
        case .accelerometer: motion.stopAccelerometerUpdates()
        case .userAccelerometer: motion.stopDeviceMotionUpdates()
        case .gyroscope: motion.stopGyroUpdates()
        case .magnetometer: motion.stopMagnetometerUpdates()
        case .barometer:
            altimeterRunning = false
            altimeter.stopRelativeAltitudeUpdates()
        }
    }

    /// Scales units of gravity to m/s² and flips them onto Android's axes.
    private nonisolated static func android(_ acceleration: CMAcceleration) -> [Double] {
        [acceleration.x * gravityToAndroid, acceleration.y * gravityToAndroid, acceleration.z * gravityToAndroid]
    }

    /// Permission failures are reported as such; every other error is native.
    private nonisolated static func result(_ values: [Double]?,
                                           _ error: (any Error)?) -> Result<[Double], PluginError> {
        if let error {
            let native = error as NSError
            let denied = native.domain == CMErrorDomain && deniedCodes.contains(native.code)

            return .failure(denied ? .permissionDenied : .native)
        }

        guard let values else { return .failure(.native) }

        return .success(values)
    }
}
#endif
