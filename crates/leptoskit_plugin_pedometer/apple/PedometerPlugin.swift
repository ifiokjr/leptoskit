import Foundation
import LeptosKitBridge
#if os(iOS)
import CoreMotion
import UIKit
#endif

@MainActor
public final class PedometerPlugin: NativePlugin {
    public let name = "pedometer"
    public let methods: [UInt32] = [0, 1, 2, 3, 4]
    private var closed = false
    private var pending: CheckedContinuation<Data, any Error>?
    private var pendingID: UUID?
    private var timer: Task<Void, Never>?
    #if os(iOS)
    private let pedometer = CMPedometer()
    private var historyInFlight: UUID?
    private var backgroundObserver: NSObjectProtocol?
    #endif
    public init() {}

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard methods.contains(method) else { throw PluginError.unavailable }
        let request = try Request(method: method, payload: payload)

        if method == 0 { return Data([available ? 1 : 0, 0, 0, 0]) }

        guard available else { throw PluginError.unavailable }

        #if os(iOS)

        if method == 1 { return permissionData }

        if method == 2, CMPedometer.authorizationStatus() != .notDetermined { return permissionData }
        try preflight(request)
        try Task.checkCancellation()
        let identifier = UUID()

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                pending = continuation
                pendingID = identifier
                begin(request, identifier: identifier)
            }
        } onCancel: { [weak self] in
            Task { @MainActor in

                guard self?.pendingID == identifier else { return }
                self?.finish(.failure(CancellationError()))
            }
        }
        #else
        _ = request
        throw PluginError.unavailable
        #endif
    }

    private var available: Bool {
        #if os(iOS)
        CMPedometer.isStepCountingAvailable()
        #else
        false
        #endif
    }

    public func close() {
        closed = true
        finish(.failure(PluginError.cancelled))
    }

    private func finish(_ result: Result<Data, any Error>) {
        guard let continuation = pending else { return }
        pending = nil
        pendingID = nil
        timer?.cancel()
        timer = nil

        #if os(iOS)
        pedometer.stopUpdates()

        if let backgroundObserver { NotificationCenter.default.removeObserver(backgroundObserver) }

        backgroundObserver = nil
        #endif
        continuation.resume(with: result)
    }

    #if os(iOS)
    private func preflight(_ request: Request) throws {
        let method = request.method

        guard pending == nil, historyInFlight == nil else { throw PluginError.busy }

        guard let description = Bundle.main.object(forInfoDictionaryKey: "NSMotionUsageDescription") as? String,
              !description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw PluginError.permissionDenied
        }

        guard UIApplication.shared.applicationState == .active else { throw PluginError.cancelled }

        if method != 2, CMPedometer.authorizationStatus() != .authorized { throw PluginError.permissionDenied }

        if let start = request.start, let end = request.end {
            let now = Date().timeIntervalSince1970 * 1000

            guard start >= now - 7 * 24 * 60 * 60 * 1000, end <= now else { throw PluginError.invalidArgument }
        }
    }

    private var permissionData: Data {
        let value: UInt8
        switch CMPedometer.authorizationStatus() {
        case .notDetermined: value = 1
        case .denied: value = 2
        case .authorized: value = 3
        case .restricted: value = 4
        @unknown default: value = 0
        }

        return Data([value, 0, 0, 0])
    }

    private func startTimer(_ request: Request, identifier: UUID) {
        timer = Task { [weak self] in
            let ticks = request.method == 2 ? request.timeout / 100 : 1

            for _ in 0..<ticks {
                do {
                    try await Task.sleep(for: .milliseconds(request.method == 2 ? 100 : request.timeout))
                } catch { return }

                guard let self, self.pendingID == identifier else { return }

                if request.method == 2, CMPedometer.authorizationStatus() != .notDetermined {
                    self.finish(.success(self.permissionData))

                    return
                }
            }

            guard let self, self.pendingID == identifier else { return }
            self.finish(request.method == 2 ? .failure(PluginError.unavailable) : .success(Data([1, 0, 0, 0])))
        }
    }

    private func receive(_ sample: StepSample?, failed: Bool, request: Request) {
        if request.method == 2 {
            if CMPedometer.authorizationStatus() != .notDetermined {
                finish(.success(permissionData))
            } else if failed { finish(.failure(PluginError.native)) }

            return
        }

        guard CMPedometer.authorizationStatus() == .authorized else {
            finish(.failure(PluginError.permissionDenied))

            return
        }

        guard UIApplication.shared.applicationState == .active else {
            finish(.failure(PluginError.cancelled))

            return
        }

        guard !failed, let sample else { finish(.failure(PluginError.native)); return }
        finish(Result { try sample.encode() })
    }

    private func begin(_ request: Request, identifier: UUID) {
        guard !Task.isCancelled else { finish(.failure(CancellationError())); return }
        backgroundObserver = NotificationCenter.default.addObserver(
            forName: request.method == 2 ? UIApplication.didEnterBackgroundNotification
                : UIApplication.willResignActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.finish(.failure(PluginError.cancelled)) }
        }
        startTimer(request, identifier: identifier)
        let callback: CMPedometerHandler = { [weak self] data, error in
            // Extract immutable primitives before crossing onto the UI actor.
            let sample = data.map { StepSample(start: $0.startDate.timeIntervalSince1970 * 1000,
                                               end: $0.endDate.timeIntervalSince1970 * 1000,
                                               steps: $0.numberOfSteps.int64Value) }
            let failed = error != nil
            Task { @MainActor in

                guard let self else { return }

                if self.historyInFlight == identifier { self.historyInFlight = nil }

                guard self.pendingID == identifier else { return }
                self.receive(sample, failed: failed, request: request)
            }
        }

        if let start = request.start, let end = request.end {
            historyInFlight = identifier
            pedometer.queryPedometerData(from: Date(timeIntervalSince1970: start / 1000),
                                        to: Date(timeIntervalSince1970: end / 1000), withHandler: callback)

        } else {
            pedometer.startUpdates(from: Date(), withHandler: callback)
        }
    }
    #endif
}

struct Request: Sendable {
    let method: UInt32
    let timeout: UInt32
    let start: Double?
    let end: Double?
    private struct Fields { let timeout: UInt32; let start: Double?; let end: Double? }
    init(method: UInt32, payload: NativeBuffer) throws {
        self.method = method
        let values = try payload.withUnsafeBytes { bytes -> Fields in

            if method <= 2 {
                guard bytes.isEmpty else { throw PluginError.invalidArgument }

                return Fields(timeout: 30_000, start: nil, end: nil)
            }

            guard bytes.count == (method == 3 ? 4 : 20) else { throw PluginError.invalidArgument }
            let offset = method == 3 ? 0 : 16
            let timeout = UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: offset, as: UInt32.self))

            guard (1...120_000).contains(timeout) else { throw PluginError.invalidArgument }

            if method == 3 { return Fields(timeout: timeout, start: nil, end: nil) }
            let start = Double(bitPattern: UInt64(littleEndian: bytes.loadUnaligned(as: UInt64.self)))
            let end = Double(bitPattern: UInt64(littleEndian: bytes.loadUnaligned(fromByteOffset: 8, as: UInt64.self)))

            guard start.isFinite, end.isFinite, start >= 0, end > start else { throw PluginError.invalidArgument }

            return Fields(timeout: timeout, start: start, end: end)
        }
        timeout = values.timeout
        start = values.start
        end = values.end
    }
}

struct StepSample: Sendable {
    let start: Double
    let end: Double
    let steps: Int64
    func encode() throws -> Data {
        guard start.isFinite, end.isFinite, start >= 0, end >= start, steps >= 0 else { throw PluginError.native }
        var bytes = Data([0, 0, 0, 0]) // bounded success

        for value in [UInt64(0), UInt64(steps), start.bitPattern, end.bitPattern] {
            var little = value.littleEndian
            withUnsafeBytes(of: &little) { bytes.append(contentsOf: $0) }
        }

        return bytes
    }
}
