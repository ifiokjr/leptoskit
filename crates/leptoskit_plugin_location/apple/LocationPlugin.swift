import CoreLocation
import Foundation
import LeptosKitBridge
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

// swiftlint:disable file_length
// The plugin keeps its location manager delegate, bridge methods, and wire
// validation in one file because the bridge resolves the plugin by name.

/// Bounds the wire record enforces.
private enum Wire {
    /// One record is five u32 fields and eight doubles.
    static let recordBytes = 84
    /// Coordinates are real degrees on the sphere.
    static let maxLatitude: Double = 90
    static let maxLongitude: Double = 180
    /// A course is a clockwise angle that stays under a full turn.
    static let fullTurn: Double = 360
}

@MainActor
// swiftlint:disable:next type_body_length
public final class LocationPlugin: NSObject, NativePlugin, @preconcurrency CLLocationManagerDelegate {
    public let name = "location"
    public let methods: [UInt32] = [0, 1, 2, 3]
    /// A permission prompt gets the longest timeout the Rust crate accepts.
    private static let promptTimeoutMilliseconds: UInt32 = 120_000
    /// The granted status code the permission reply carries.
    private static let granted: UInt32 = 3
    private let manager = CLLocationManager()
    private var closed = false
    private var pending: CheckedContinuation<Data, any Error>?
    private var pendingID: UUID?
    private var authorizationRequest = false
    private var options: LocationOptions?
    private var preciseAtStart = false
    private var started = 0.0
    private var timeout: Task<Void, Never>?
    private var backgroundObserver: NSObjectProtocol?

    /// The wire methods the Rust crate calls.
    private enum Method: UInt32 {
        case permission = 0
        case requestPermission = 1
        case current = 2
        case next = 3
    }

    public override init() {
        super.init()
        manager.delegate = self
    }

    /// Answers one call: a permission query never prompts, and a fix request needs
    /// access the OS already granted.
    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        switch Method(rawValue: method) {
        case .permission: return try permissionReply(payload)
        case .requestPermission: return try await requestAccess(payload)
        case .current: return try await locate(payload, hasPrevious: false)
        case .next: return try await locate(payload, hasPrevious: true)
        case nil: throw PluginError.unavailable
        }
    }

    /// Reports the current scope without prompting.
    private func permissionReply(_ payload: NativeBuffer) throws -> Data {
        guard payload.count == 0 else { throw PluginError.invalidArgument }

        return permission()
    }

    /// Asks the OS once, or reports the scope when it has already answered.
    private func requestAccess(_ payload: NativeBuffer) async throws -> Data {
        try payload.withUnsafeBytes { bytes in

            guard bytes.count == 4, UInt32(littleEndian: bytes.loadUnaligned(as: UInt32.self)) <= 1 else {
                throw PluginError.invalidArgument
            }
        }

        guard manager.authorizationStatus == .notDetermined else { return permission() }

        try requireUsageDescription()
        try requireForeground()

        return try await wait(options: nil)
    }

    /// Waits for a bounded fix the caller's options describe.
    private func locate(_ payload: NativeBuffer, hasPrevious: Bool) async throws -> Data {
        let options = try payload.withUnsafeBytes { try LocationOptions(bytes: $0, hasPrevious: hasPrevious) }

        guard isAuthorized else { throw PluginError.permissionDenied }

        try requireUsageDescription()
        try requireForeground()

        return try await wait(options: options)
    }

    #if os(iOS)
    /// A prompt needs the app in the foreground, where iOS will show it.
    private func requireForeground() throws {
        guard UIApplication.shared.applicationState == .active else { throw PluginError.cancelled }
    }
    #elseif os(macOS)
    /// A prompt needs the app in the foreground, where macOS will show it.
    private func requireForeground() throws {
        guard NSApplication.shared.isActive else { throw PluginError.cancelled }
    }
    #endif

    public func close() {
        closed = true
        finish(.failure(PluginError.cancelled))
        manager.delegate = nil
    }

    private var isAuthorized: Bool {
        #if os(macOS)
        manager.authorizationStatus == .authorizedAlways
        #else
        manager.authorizationStatus == .authorizedAlways || manager.authorizationStatus == .authorizedWhenInUse
        #endif
    }

    /// The usage description Apple requires before it shows the permission prompt.
    private func requireUsageDescription() throws {
        #if os(macOS)
        let key = "NSLocationUsageDescription"
        #else
        let key = "NSLocationWhenInUseUsageDescription"
        #endif

        guard let description = Bundle.main.object(forInfoDictionaryKey: key) as? String,
              !description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw PluginError.invalidArgument
        }
    }

    private func wait(options: LocationOptions?) async throws -> Data {
        guard pending == nil else { throw PluginError.busy }

        try Task.checkCancellation()
        let identifier = UUID()

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                begin(continuation, options: options, identifier: identifier)
            }
        } onCancel: { [weak self] in
            Task { @MainActor in self?.cancelIfCurrent(identifier) }
        }
    }

    /// Registers the waiter and starts the acquisition it belongs to.
    private func begin(
        _ continuation: CheckedContinuation<Data, any Error>,
        options: LocationOptions?,
        identifier: UUID
    ) {
        pending = continuation
        pendingID = identifier
        authorizationRequest = options == nil
        self.options = options
        preciseAtStart = manager.accuracyAuthorization == .fullAccuracy

        if Task.isCancelled {
            finish(.failure(CancellationError()))

            return
        }

        started = Date().timeIntervalSince1970 * 1000
        scheduleTimeout(milliseconds: options?.timeout ?? Self.promptTimeoutMilliseconds, identifier: identifier)
        observeBackground()

        if let options {
            startUpdates(options)
        } else {
            manager.requestWhenInUseAuthorization()
        }
    }

    /// Starts the updates the request's accuracy choice asks for.
    private func startUpdates(_ options: LocationOptions) {
        manager.desiredAccuracy = options.highAccuracy ? kCLLocationAccuracyBest : kCLLocationAccuracyHundredMeters
        manager.distanceFilter = kCLDistanceFilterNone

        manager.startUpdatingLocation()
    }

    /// Cancels the waiter the caller abandoned, if it is still the one that runs.
    private func cancelIfCurrent(_ identifier: UUID) {
        guard pendingID == identifier else { return }

        finish(.failure(CancellationError()))
    }

    private func scheduleTimeout(milliseconds: UInt32, identifier: UUID) {
        timeout = Task { @MainActor [weak self] in

            guard await Self.sleep(milliseconds) else { return }

            guard let self, self.pendingID == identifier else { return }

            self.finish(.success(self.timeoutAnswer()))
        }
    }

    /// Sleeps for the wait's deadline, saying whether it ran to completion.
    private static func sleep(_ milliseconds: UInt32) async -> Bool {
        do {
            try await Task.sleep(for: .milliseconds(milliseconds))
        } catch {
            return false
        }

        return true
    }

    /// The answer a waiter gets when its deadline passes first.
    private func timeoutAnswer() -> Data {
        if authorizationRequest { return permission() }

        return Data([1, 0, 0, 0])
    }

    /// The notification the host posts when the app can no longer show a prompt.
    private var departureNotification: Notification.Name {
        #if os(iOS)

        return UIApplication.didEnterBackgroundNotification
        #else
        // A system authorization dialog can itself remove focus; hiding the
        // app still cancels its waiter without attempting to dismiss that dialog.
        return authorizationRequest
            ? NSApplication.didHideNotification : NSApplication.didResignActiveNotification
        #endif
    }

    private func observeBackground() {
        backgroundObserver = NotificationCenter.default.addObserver(
            forName: departureNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.finish(.failure(PluginError.cancelled)) }
        }
    }

    /// Answers a permission change; a downgrade cannot satisfy a waiting fix.
    public func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        guard pending != nil else { return }

        if authorizationRequest {
            answeredAuthorization(manager)

            return
        }

        if !canDeliverLocation { finish(.failure(PluginError.permissionDenied)) }
    }

    /// Answers a pending permission request once the OS has made a choice.
    private func answeredAuthorization(_ manager: CLLocationManager) {
        guard manager.authorizationStatus != .notDetermined else { return }

        finish(.success(permission()))
    }

    /// Answers with the newest fix inside the request's age and cursor window.
    public func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        guard let options else { return }

        guard canDeliverLocation else {
            finish(.failure(PluginError.permissionDenied))

            return
        }

        guard let location = newest(locations, options) else { return }

        deliver(location)
    }

    /// Answers with an encoded fix, or the encoding failure.
    private func deliver(_ location: CLLocation) {
        do {
            finish(.success(try encodeLocation(location)))
        } catch {
            finish(.failure(error))
        }
    }

    /// The newest reported fix that is fresh enough and past the caller's cursor.
    private func newest(_ locations: [CLLocation], _ options: LocationOptions) -> CLLocation? {
        let oldest = options.maximumAge == 0
            ? started : Date().timeIntervalSince1970 * 1000 - Double(options.maximumAge)

        return locations.last(where: { location in
            let timestamp = location.timestamp.timeIntervalSince1970 * 1000

            if timestamp < oldest { return false }

            return options.after.map { timestamp > $0 } ?? true
        })
    }

    public func locationManager(_ manager: CLLocationManager, didFailWithError error: any Error) {
        guard pending != nil else { return }
        let native = error as NSError

        if native.domain == kCLErrorDomain && native.code == CLError.locationUnknown.rawValue {
            // A transient unavailable fix can recover within the caller's bounded deadline.
            return
        }

        let denied = native.domain == kCLErrorDomain && native.code == CLError.denied.rawValue
        finish(.failure(denied ? PluginError.permissionDenied : PluginError.native))
    }

    /// Reports the authorization state and the accuracy left to the app.
    private func permission() -> Data {
        let status = statusCode(manager.authorizationStatus)
        let accuracy = grantAccuracy(status)
        var result = Data()

        for var field in [status.littleEndian, accuracy.littleEndian] {
            withUnsafeBytes(of: &field) { result.append(contentsOf: $0) }
        }

        return result
    }

    /// Accuracy matters only once access is granted; until then it is unknown.
    private func grantAccuracy(_ status: UInt32) -> UInt32 {
        guard status == Self.granted else { return 0 }

        return manager.accuracyAuthorization == .fullAccuracy ? 2 : 1
    }

    #if os(iOS)
    /// Reads the authorization state, including iOS's "when in use" grant.
    private func statusCode(_ status: CLAuthorizationStatus) -> UInt32 {
        switch status {
        case .notDetermined: 1
        case .denied: 2
        case .authorizedAlways, .authorizedWhenInUse: 3
        case .restricted: 4
        @unknown default: 0
        }
    }
    #else
    /// Reads the authorization state; macOS has no "when in use" grant.
    private func statusCode(_ status: CLAuthorizationStatus) -> UInt32 {
        switch status {
        case .notDetermined: 1
        case .denied: 2
        case .authorizedAlways: 3
        case .restricted: 4
        @unknown default: 0
        }
    }
    #endif

    private var canDeliverLocation: Bool {
        permitsLocationDelivery(
            startedPrecise: preciseAtStart,
            authorized: isAuthorized,
            precise: manager.accuracyAuthorization == .fullAccuracy
        )
    }

    private func finish(_ result: Result<Data, any Error>) {
        guard let continuation = pending else { return }
        pending = nil
        pendingID = nil
        options = nil
        preciseAtStart = false
        authorizationRequest = false
        timeout?.cancel()
        timeout = nil

        manager.stopUpdatingLocation()

        if let backgroundObserver { NotificationCenter.default.removeObserver(backgroundObserver) }

        backgroundObserver = nil
        continuation.resume(with: result)
    }
}

/// Rejects queued precise coordinates after authorization is downgraded.
func permitsLocationDelivery(startedPrecise: Bool, authorized: Bool, precise: Bool) -> Bool {
    authorized && (!startedPrecise || precise)
}

struct LocationOptions {
    /// Byte counts and value bounds the request enforces.
    private enum Bound {
        static let bytes = 12
        static let previousBytes = 20
        static let maxTimeout: UInt32 = 120_000
        static let maxAge: UInt32 = 3_600_000
    }

    /// Where each field starts in the request bytes.
    private enum Field {
        static let timeout = 4
        static let maximumAge = 8
        static let after = 12
    }

    let highAccuracy: Bool
    let timeout: UInt32
    let maximumAge: UInt32
    let after: Double?

    init(bytes: UnsafeRawBufferPointer, hasPrevious: Bool) throws {
        let expected = hasPrevious ? Bound.previousBytes : Bound.bytes

        guard bytes.count == expected else { throw PluginError.invalidArgument }
        let accuracy = UInt32(littleEndian: bytes.loadUnaligned(as: UInt32.self))
        timeout = UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: Field.timeout, as: UInt32.self))
        maximumAge = UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: Field.maximumAge, as: UInt32.self))

        guard accuracy <= 1, (1...Bound.maxTimeout).contains(timeout), maximumAge <= Bound.maxAge else {
            throw PluginError.invalidArgument
        }

        highAccuracy = accuracy == 1

        if hasPrevious {
            let bits = UInt64(littleEndian: bytes.loadUnaligned(fromByteOffset: Field.after, as: UInt64.self))

            after = try timestamp(bits)
        } else {
            after = nil
        }
    }
}

/// Reads a follow-up cursor, which must be a real timestamp.
private func timestamp(_ bits: UInt64) throws -> Double {
    let value = Double(bitPattern: bits)

    guard value.isFinite, value >= 0 else { throw PluginError.invalidArgument }

    return value
}

/// Pure conversion also used by tests with synthetic coordinates; never starts location acquisition.
func encodeLocation(_ location: CLLocation) throws -> Data {
    let optional = optionalValues(location)
    let values = requiredValues(location) + optional.map { $0 ?? 0 }

    try requireDeliverable(values)
    var mask: UInt32 = 0

    for (index, value) in optional.enumerated() where value != nil {
        mask |= 1 << index
    }

    let simulated: UInt32 = location.sourceInformation.map { $0.isSimulatedBySoftware ? 2 : 1 } ?? 0
    var result = Data(capacity: Wire.recordBytes)

    for var field in [UInt32(0), mask, 0, simulated, 0].map(\.littleEndian) {
        withUnsafeBytes(of: &field) { result.append(contentsOf: $0) }
    }

    for value in values {
        var bits = value.bitPattern.littleEndian
        withUnsafeBytes(of: &bits) { result.append(contentsOf: $0) }
    }

    return result
}

/// The optional fields in wire order: altitude, vertical accuracy, course, speed.
private func optionalValues(_ location: CLLocation) -> [Double?] {
    return [
        location.verticalAccuracy >= 0 ? location.altitude : nil,
        location.verticalAccuracy >= 0 ? location.verticalAccuracy : nil,
        location.course >= 0 && location.course < Wire.fullTurn ? location.course : nil,
        location.speed >= 0 ? location.speed : nil
    ]
}

/// The timestamp, latitude, longitude, and horizontal accuracy every fix carries.
private func requiredValues(_ location: CLLocation) -> [Double] {
    return [location.timestamp.timeIntervalSince1970 * 1000,
            location.coordinate.latitude,
            location.coordinate.longitude,
            location.horizontalAccuracy]
}

/// Rejects a fix whose fields the wire cannot carry.
private func requireDeliverable(_ values: [Double]) throws {
    guard values.allSatisfy(\.isFinite),
          values[0] >= 0,
          (-Wire.maxLatitude...Wire.maxLatitude).contains(values[1]),
          (-Wire.maxLongitude...Wire.maxLongitude).contains(values[2]),
          values[3] >= 0 else { throw PluginError.native }
}
// swiftlint:enable file_length
