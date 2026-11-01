import Foundation
import LeptosKitBridge
#if os(iOS)
import UIKit
#elseif os(macOS)
import IOKit.ps
#endif

/// The wire snapshot: four little-endian u32 fields.
private enum Wire {
    static let snapshotBytes = 16
    static let fieldBytes = 4

    /// Charge is carried in thousandths of full.
    static let thousandths: Float = 1000
    static let fullCharge: UInt32 = 1000

    /// The reserved maximum stands in for a value the platform cannot measure.
    static let unknown: UInt32 = .max

    /// Tri-state encodings shared by every boolean field.
    static let flagUnknown: UInt32 = 0
    static let flagFalse: UInt32 = 1
    static let flagTrue: UInt32 = 2

    /// Presence and charging state codes.
    static let absent: UInt32 = 1
    static let present: UInt32 = 2
    static let stateUnknown: UInt32 = 0
    static let discharging: UInt32 = 1
    static let charging: UInt32 = 2
    static let full: UInt32 = 3

    /// Method identifiers.
    static let snapshot: UInt32 = 0
    static let waitForChange: UInt32 = 1
}

/// Battery state notifications use one cancellable request, never an unbounded event queue.
@MainActor
public final class BatteryPlugin: NativePlugin {
    public let name = "battery"
    public let methods: [UInt32] = [Wire.snapshot, Wire.waitForChange]
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

    /// Runs one snapshot request, or waits for the next different reading.
    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard method <= Wire.waitForChange else { throw PluginError.unavailable }

        if method == Wire.snapshot {
            return try snapshotCall(payload)
        }

        return try await changeCall(payload)
    }

    public func close() {
        closed = true
        finish(.failure(PluginError.cancelled))
    }

    #if os(iOS)
    /// Answers one reading, holding the process-wide monitor for this call only.
    private func snapshotCall(_ payload: NativeBuffer) throws -> Data {
        guard payload.count == 0 else { throw PluginError.invalidArgument }
        BatteryMonitoring.acquire()
        defer { BatteryMonitoring.release() }

        return try snapshot()
    }
    #else
    /// Answers one reading; only iOS needs the monitor held around the call.
    private func snapshotCall(_ payload: NativeBuffer) throws -> Data {
        guard payload.count == 0 else { throw PluginError.invalidArgument }

        return try snapshot()
    }
    #endif

    /**
     Answers the first reading the OS reports that differs from the caller's
     last one. Nothing suspends between reserving the waiter and installing the
     observers, so a change cannot slip into that gap.
     */
    private func changeCall(_ payload: NativeBuffer) async throws -> Data {
        try payload.withUnsafeBytes { try validateSnapshot($0) }

        guard pending == nil else { throw PluginError.busy }

        try Task.checkCancellation()
        let identifier = UUID()
        let expected = payload.withUnsafeBytes { Data($0) }

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                reserve(continuation, identifier: identifier, expected: expected)
            }
        } onCancel: { [weak self] in
            Task { @MainActor in
                self?.cancel(identifier: identifier)
            }
        }
    }

    /// Reserves the waiter, then installs observers before the caller can miss a change.
    private func reserve(_ continuation: CheckedContinuation<Data, any Error>, identifier: UUID, expected: Data) {
        pending = continuation
        pendingID = identifier
        previous = expected
        do {
            try observe()
            changed()
        } catch {
            finish(.failure(error))
        }
    }

    /// Finishes only this call; a newer waiter may already own the slot.
    private func cancel(identifier: UUID) {
        guard pendingID == identifier else { return }

        finish(.failure(CancellationError()))
    }

    /// Resumes the waiter once, and only when the reading actually differs.
    private func changed() {
        guard pending != nil else { return }
        do {
            let current = try snapshot()

            if current != previous { finish(.success(current)) }
        } catch {
            finish(.failure(error))
        }
    }

    /// Answers the waiting caller once, then releases every observer it installed.
    private func finish(_ result: Result<Data, any Error>) {
        guard let continuation = pending else { return }
        pending = nil
        pendingID = nil

        for observer in observers { NotificationCenter.default.removeObserver(observer) }
        observers.removeAll()
        stopPlatformObservation()
        previous.removeAll(keepingCapacity: false)
        continuation.resume(with: result)
    }

    /// Registers the process notifications with the change callback.
    private func installNotifications() {
        observers = notificationNames().map { name in
            NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.changed() }
            }
        }
    }

    /// Encodes the four wire fields; low-power mode is the one field every host reports.
    private func encode(present: UInt32, charge: UInt32, state: UInt32) -> Data {
        let fields: [UInt32] = [
            present,
            charge,
            state,
            ProcessInfo.processInfo.isLowPowerModeEnabled ? Wire.flagTrue : Wire.flagFalse,
        ]

        return fields.reduce(into: Data(capacity: Wire.snapshotBytes)) { result, field in
            var little = field.littleEndian
            withUnsafeBytes(of: &little) { result.append(contentsOf: $0) }
        }
    }

    #if os(iOS)
    /// Reads UIDevice; a reading is only current while monitoring is enabled.
    private func snapshot() throws -> Data {
        let device = UIDevice.current
        let charge = charge(device.batteryLevel)

        return encode(
            present: charge == Wire.unknown ? Wire.flagUnknown : Wire.present,
            charge: charge,
            state: state(device.batteryState),
        )
    }

    /// Charge in thousandths, or the reserved maximum when the device cannot measure it.
    private func charge(_ level: Float) -> UInt32 {
        guard level >= 0, level <= 1 else { return Wire.unknown }

        return UInt32((level * Wire.thousandths).rounded())
    }

    /// Maps UIDevice's battery state to the wire state.
    private func state(_ state: UIDevice.BatteryState) -> UInt32 {
        switch state {
        case .unplugged: Wire.discharging
        case .charging: Wire.charging
        case .full: Wire.full
        default: Wire.stateUnknown
        }
    }

    /// Enables the process-wide monitor, then listens for every iOS battery event.
    private func observe() throws {
        BatteryMonitoring.acquire()
        monitoring = true
        installNotifications()
    }

    private func stopPlatformObservation() {
        if monitoring { BatteryMonitoring.release() }
        monitoring = false
    }

    /// Every iOS notification that can change a battery reading.
    private func notificationNames() -> [Notification.Name] {
        [
            Notification.Name.NSProcessInfoPowerStateDidChange,
            UIDevice.batteryLevelDidChangeNotification,
            UIDevice.batteryStateDidChangeNotification,
        ]
    }
    #elseif os(macOS)
    /// Reads the first internal battery IOKit reports.
    private func snapshot() throws -> Data {
        guard let info = IOPSCopyPowerSourcesInfo()?.takeRetainedValue(),
              let sources = IOPSCopyPowerSourcesList(info)?.takeRetainedValue() as? [CFTypeRef] else {
            throw PluginError.native
        }

        for source in sources {
            guard let description = IOPSGetPowerSourceDescription(info, source)?.takeUnretainedValue()
                    as? [String: Any] else { throw PluginError.native }

            guard description[kIOPSTypeKey] as? String == kIOPSInternalBatteryType else { continue }

            return encode(present: Wire.present, charge: macCharge(description), state: macState(description))
        }

        return encode(present: Wire.absent, charge: Wire.unknown, state: Wire.stateUnknown)
    }

    /// Charge in thousandths, or the reserved maximum for a gauge that cannot say.
    private func macCharge(_ description: [String: Any]) -> UInt32 {
        guard let capacity = (description[kIOPSCurrentCapacityKey] as? NSNumber)?.doubleValue,
              let maximum = (description[kIOPSMaxCapacityKey] as? NSNumber)?.doubleValue,
              capacity.isFinite, maximum.isFinite, maximum > 0, capacity >= 0, capacity <= maximum else {
            return Wire.unknown
        }

        return UInt32((capacity / maximum * Double(Wire.fullCharge)).rounded())
    }

    /// Maps the power source's description to the wire state.
    private func macState(_ description: [String: Any]) -> UInt32 {
        if description[kIOPSIsChargedKey] as? Bool == true { return Wire.full }

        if description[kIOPSIsChargingKey] as? Bool == true { return Wire.charging }

        if description[kIOPSPowerSourceStateKey] as? String == kIOPSBatteryPowerValue { return Wire.discharging }

        return Wire.stateUnknown
    }

    /// Adds the power-source run-loop source, then listens for process changes.
    private func observe() throws {
        try startPowerSource()
        installNotifications()
    }

    /// Adds the power-source run-loop source IOKit reports changes on.
    private func startPowerSource() throws {
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
    }

    private func stopPlatformObservation() {
        if let powerSource { CFRunLoopSourceInvalidate(powerSource) }
        powerSource = nil
    }

    /// The one process notification every Apple host reports.
    private func notificationNames() -> [Notification.Name] {
        [Notification.Name.NSProcessInfoPowerStateDidChange]
    }
    #else
    private func snapshot() throws -> Data {
        throw PluginError.unavailable
    }

    private func observe() throws {
        throw PluginError.unavailable
    }

    private func stopPlatformObservation() {}

    private func notificationNames() -> [Notification.Name] {
        [Notification.Name.NSProcessInfoPowerStateDidChange]
    }
    #endif
}

/// Echoes the caller's snapshot back, refusing fields the wire cannot carry.
private func validateSnapshot(_ bytes: UnsafeRawBufferPointer) throws {
    guard bytes.count == Wire.snapshotBytes else { throw PluginError.invalidArgument }
    let fields = stride(from: 0, to: Wire.snapshotBytes, by: Wire.fieldBytes).map {
        UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: $0, as: UInt32.self))
    }

    guard fields[0] <= Wire.flagTrue,
          fields[1] <= Wire.fullCharge || fields[1] == Wire.unknown,
          fields[2] <= Wire.full,
          fields[3] <= Wire.flagTrue else { throw PluginError.invalidArgument }
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
