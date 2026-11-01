import Foundation
import LeptosKitBridge
#if os(iOS)
import UIKit
#elseif os(macOS)
import IOKit.ps
#endif

/// Battery state notifications use one cancellable request, never an unbounded event queue.
@MainActor
public final class BatteryPlugin: NativePlugin {
    public let name = "battery"
    public let methods: [UInt32] = [0, 1]
    private var closed = false
    private var pending: CheckedContinuation<Data, any Error>?
    private var pendingID: UUID?
    private var previous = Data()
    private var observers: [NSObjectProtocol] = []

    #if os(iOS)
    private var monitoring = false
    #elseif os(macOS)
    private var powerSource: CFRunLoopSource?
    #endif

    public init() {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard method <= 1 else { throw PluginError.unavailable }

        if method == 0 {
            guard payload.count == 0 else { throw PluginError.invalidArgument }
        } else {
            try payload.withUnsafeBytes { try validateSnapshot($0) }
        }

        if method == 0 {
            #if os(iOS)

            BatteryMonitoring.acquire()
            defer { BatteryMonitoring.release() }
            #endif

            return try snapshot()
        }

        guard pending == nil else { throw PluginError.busy }
        try Task.checkCancellation()
        let identifier = UUID()

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                // No suspension between reserving the waiter and installing observers.
                pending = continuation
                pendingID = identifier
                previous = payload.withUnsafeBytes { Data($0) }
                do {
                    try observe()
                    changed()
                } catch {
                    finish(.failure(error))
                }
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

    private func changed() {
        guard pending != nil else { return }
        do {
            let current = try snapshot()

            if current != previous { finish(.success(current)) }
        } catch {
            finish(.failure(error))
        }
    }

    private func finish(_ result: Result<Data, any Error>) {
        guard let continuation = pending else { return }
        pending = nil
        pendingID = nil

        for observer in observers { NotificationCenter.default.removeObserver(observer) }
        observers.removeAll()

        #if os(iOS)

        if monitoring { BatteryMonitoring.release() }
        monitoring = false

        #elseif os(macOS)

        if let powerSource { CFRunLoopSourceInvalidate(powerSource) }
        powerSource = nil
        #endif
        previous.removeAll(keepingCapacity: false)
        continuation.resume(with: result)
    }

    private func observe() throws {
        var names = [Notification.Name.NSProcessInfoPowerStateDidChange]

        #if os(iOS)
        BatteryMonitoring.acquire()
        monitoring = true
        names += [UIDevice.batteryLevelDidChangeNotification, UIDevice.batteryStateDidChangeNotification]
        #elseif os(macOS)
        // The suspended invoke retains self until finish removes this run-loop source.
        let context = Unmanaged.passUnretained(self).toOpaque()
        guard let source = IOPSNotificationCreateRunLoopSource({ context in
            guard let context else { return }
            MainActor.assumeIsolated {
                Unmanaged<BatteryPlugin>.fromOpaque(context).takeUnretainedValue().changed()
            }
        }, context)?.takeRetainedValue() else { throw PluginError.native }

        powerSource = source
        CFRunLoopAddSource(CFRunLoopGetMain(), source, .commonModes)
        #endif
        observers = names.map { name in
            NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.changed() }
            }
        }
    }

    private func snapshot() throws -> Data {
        #if os(iOS)
        let device = UIDevice.current
        let level = device.batteryLevel
        let charge = level >= 0 && level <= 1 ? UInt32((level * 1000).rounded()) : UInt32.max
        let state: UInt32 = switch device.batteryState {
        case .unknown: 0
        case .unplugged: 1
        case .charging: 2
        case .full: 3
        @unknown default: 0
        }

        return encode(present: charge == UInt32.max ? 0 : 2, charge: charge, state: state)
        #elseif os(macOS)

        return try macSnapshot()

        #else
        throw PluginError.unavailable
        #endif
    }

    #if os(macOS)
    private func macSnapshot() throws -> Data {
        guard let info = IOPSCopyPowerSourcesInfo()?.takeRetainedValue(),
              let sources = IOPSCopyPowerSourcesList(info)?.takeRetainedValue() as? [CFTypeRef] else {
            throw PluginError.native
        }

        for source in sources {
            guard let description = IOPSGetPowerSourceDescription(info, source)?.takeUnretainedValue()
                    as? [String: Any] else { throw PluginError.native }

            guard description[kIOPSTypeKey] as? String == kIOPSInternalBatteryType else { continue }

            return encode(present: 2, charge: macCharge(description), state: macState(description))
        }

        return encode(present: 1, charge: UInt32.max, state: 0)
    }

    private func macCharge(_ description: [String: Any]) -> UInt32 {
        guard let capacity = (description[kIOPSCurrentCapacityKey] as? NSNumber)?.doubleValue,
              let maximum = (description[kIOPSMaxCapacityKey] as? NSNumber)?.doubleValue,
              capacity.isFinite, maximum.isFinite, maximum > 0, capacity >= 0, capacity <= maximum else {
            return UInt32.max
        }

        return UInt32((capacity / maximum * 1000).rounded())
    }

    private func macState(_ description: [String: Any]) -> UInt32 {
        if description[kIOPSIsChargedKey] as? Bool == true { return 3 }

        if description[kIOPSIsChargingKey] as? Bool == true { return 2 }

        if description[kIOPSPowerSourceStateKey] as? String == kIOPSBatteryPowerValue { return 1 }
        return 0
    }
    #endif

    private func encode(present: UInt32, charge: UInt32, state: UInt32) -> Data {
        let fields: [UInt32] = [present, charge, state, ProcessInfo.processInfo.isLowPowerModeEnabled ? 2 : 1]
        var result = Data(capacity: 16)

        for field in fields {
            var little = field.littleEndian
            withUnsafeBytes(of: &little) { result.append(contentsOf: $0) }
        }

        return result
    }
}

private func validateSnapshot(_ bytes: UnsafeRawBufferPointer) throws {
    guard bytes.count == 16 else { throw PluginError.invalidArgument }
    let fields = stride(from: 0, to: 16, by: 4).map {
        UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: $0, as: UInt32.self))
    }

    guard fields[0] <= 2, fields[1] <= 1000 || fields[1] == UInt32.max,
          fields[2] <= 3, fields[3] <= 2 else { throw PluginError.invalidArgument }
}

#if os(iOS)
/// UIDevice monitoring is process-wide, including hosts in separate windows.
@MainActor
private enum BatteryMonitoring {
    private static var users = 0
    private static var previous = false

    static func acquire() {
        if users == 0 {
            previous = UIDevice.current.isBatteryMonitoringEnabled
            UIDevice.current.isBatteryMonitoringEnabled = true
        }

        users += 1
    }

    static func release() {
        precondition(users > 0)
        users -= 1

        if users == 0 { UIDevice.current.isBatteryMonitoringEnabled = previous }
    }
}
#endif
