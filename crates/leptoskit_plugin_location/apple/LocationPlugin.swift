import CoreLocation
import Foundation
import LeptosKitBridge
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

@MainActor
public final class LocationPlugin: NSObject, NativePlugin, @preconcurrency CLLocationManagerDelegate {
    public let name = "location"
    public let methods: [UInt32] = [0, 1, 2, 3]
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

    public override init() {
        super.init()
        manager.delegate = self
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        if method == 0 {
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            return permission()
        }
        if method == 1 {
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
        guard method == 2 || method == 3 else { throw PluginError.unavailable }
        let options = try payload.withUnsafeBytes { try LocationOptions(bytes: $0, hasPrevious: method == 3) }
        guard isAuthorized else { throw PluginError.permissionDenied }
        try requireUsageDescription()
        try requireForeground()
        return try await wait(options: options)
    }

    private func requireForeground() throws {
        #if os(iOS)
        guard UIApplication.shared.applicationState == .active else { throw PluginError.cancelled }
        #elseif os(macOS)
        guard NSApplication.shared.isActive else { throw PluginError.cancelled }
        #endif
    }

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
                scheduleTimeout(milliseconds: options?.timeout ?? 120_000, identifier: identifier)
                observeBackground()
                if let options {
                    manager.desiredAccuracy = options.highAccuracy
                        ? kCLLocationAccuracyBest : kCLLocationAccuracyHundredMeters
                    manager.distanceFilter = kCLDistanceFilterNone
                    manager.startUpdatingLocation()
                } else {
                    manager.requestWhenInUseAuthorization()
                }
            }
        } onCancel: { [weak self] in
            Task { @MainActor in
                guard self?.pendingID == identifier else { return }
                self?.finish(.failure(CancellationError()))
            }
        }
    }

    private func scheduleTimeout(milliseconds: UInt32, identifier: UUID) {
        timeout = Task { @MainActor [weak self] in
            do { try await Task.sleep(for: .milliseconds(milliseconds)) } catch { return }
            guard let self, self.pendingID == identifier else { return }
            if self.authorizationRequest {
                self.finish(.success(self.permission()))
            } else {
                self.finish(.success(Data([1, 0, 0, 0])))
            }
        }
    }

    private func observeBackground() {
        #if os(iOS)
        let notification = UIApplication.didEnterBackgroundNotification
        #else
        // A system authorization dialog can itself remove focus; hiding the
        // app still cancels its waiter without attempting to dismiss that dialog.
        let notification = authorizationRequest
            ? NSApplication.didHideNotification : NSApplication.didResignActiveNotification
        #endif
        backgroundObserver = NotificationCenter.default.addObserver(
            forName: notification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.finish(.failure(PluginError.cancelled)) }
        }
    }

    public func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        guard pending != nil else { return }
        if authorizationRequest {
            if manager.authorizationStatus != .notDetermined { finish(.success(permission())) }
        } else if !canDeliverLocation {
            finish(.failure(PluginError.permissionDenied))
        }
    }

    public func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        guard let options else { return }
        guard canDeliverLocation else {
            finish(.failure(PluginError.permissionDenied))
            return
        }
        let oldest = options.maximumAge == 0
            ? started : Date().timeIntervalSince1970 * 1000 - Double(options.maximumAge)
        guard let location = locations.last(where: {
            let timestamp = $0.timestamp.timeIntervalSince1970 * 1000
            return timestamp >= oldest && options.after.map { timestamp > $0 } != false
        }) else { return }
        do { finish(.success(try encodeLocation(location))) } catch { finish(.failure(error)) }
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

    private func permission() -> Data {
        let status: UInt32
        switch manager.authorizationStatus {
        case .notDetermined: status = 1
        case .denied: status = 2
        case .authorizedAlways: status = 3
        #if os(iOS)
        case .authorizedWhenInUse: status = 3
        #endif
        case .restricted: status = 4
        @unknown default: status = 0
        }
        let accuracy: UInt32 = status == 3 ? (manager.accuracyAuthorization == .fullAccuracy ? 2 : 1) : 0
        var result = Data()
        for var field in [status.littleEndian, accuracy.littleEndian] {
            withUnsafeBytes(of: &field) { result.append(contentsOf: $0) }
        }
        return result
    }

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
    let highAccuracy: Bool
    let timeout: UInt32
    let maximumAge: UInt32
    let after: Double?

    init(bytes: UnsafeRawBufferPointer, hasPrevious: Bool) throws {
        guard bytes.count == (hasPrevious ? 20 : 12) else { throw PluginError.invalidArgument }
        let accuracy = UInt32(littleEndian: bytes.loadUnaligned(as: UInt32.self))
        timeout = UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: 4, as: UInt32.self))
        maximumAge = UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: 8, as: UInt32.self))
        guard accuracy <= 1, (1...120_000).contains(timeout), maximumAge <= 3_600_000 else {
            throw PluginError.invalidArgument
        }
        highAccuracy = accuracy == 1
        if hasPrevious {
            let bits = UInt64(littleEndian: bytes.loadUnaligned(fromByteOffset: 12, as: UInt64.self))
            let timestamp = Double(bitPattern: bits)
            guard timestamp.isFinite, timestamp >= 0 else { throw PluginError.invalidArgument }
            after = timestamp
        } else {
            after = nil
        }
    }
}

/// Pure conversion also used by tests with synthetic coordinates; never starts location acquisition.
func encodeLocation(_ location: CLLocation) throws -> Data {
    let optional: [Double?] = [
        location.verticalAccuracy >= 0 ? location.altitude : nil,
        location.verticalAccuracy >= 0 ? location.verticalAccuracy : nil,
        location.course >= 0 && location.course < 360 ? location.course : nil,
        location.speed >= 0 ? location.speed : nil
    ]
    let values = [location.timestamp.timeIntervalSince1970 * 1000,
                  location.coordinate.latitude, location.coordinate.longitude, location.horizontalAccuracy]
        + optional.map { $0 ?? 0 }
    guard values.allSatisfy(\.isFinite), values[0] >= 0,
          (-90...90).contains(values[1]), (-180...180).contains(values[2]), values[3] >= 0 else {
        throw PluginError.native
    }
    var mask: UInt32 = 0
    for (index, value) in optional.enumerated() where value != nil { mask |= 1 << index }
    let simulated: UInt32 = location.sourceInformation.map { $0.isSimulatedBySoftware ? 2 : 1 } ?? 0
    var result = Data(capacity: 84)
    for var field in [UInt32(0), mask, 0, simulated, 0].map(\.littleEndian) {
        withUnsafeBytes(of: &field) { result.append(contentsOf: $0) }
    }
    for value in values {
        var bits = value.bitPattern.littleEndian
        withUnsafeBytes(of: &bits) { result.append(contentsOf: $0) }
    }
    return result
}
