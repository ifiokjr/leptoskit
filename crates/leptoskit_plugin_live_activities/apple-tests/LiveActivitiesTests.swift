import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitLiveActivities
import XCTest

@MainActor
private final class FakeLiveActivities: LiveActivityService {
    var areEnabled = true
    var frequentUpdatesEnabled = false
    var activities: [String: (kind: String, title: String, fields: [String: String], push: Bool)] = [:]
    var tokens: [String: Data] = [:]
    var tokenWaiters: [String: CheckedContinuation<Data, any Error>] = [:]
    var startError: PluginError?
    var updateError: PluginError?
    var endError: PluginError?

    func start(
        id: String,
        kind: String,
        title: String,
        fields: [String: String],
        pushUpdates: Bool
    ) async throws {
        if let startError { throw startError }
        activities[id] = (kind, title, fields, pushUpdates)
    }

    func update(id: String, fields: [String: String]) async throws {
        if let updateError { throw updateError }
        guard var existing = activities[id] else { throw PluginError.unavailable }
        existing.fields = fields
        activities[id] = existing
    }

    func end(
        id: String,
        fields: [String: String]?,
        dismissal: DismissalKind,
        afterSeconds: UInt32
    ) async throws {
        _ = dismissal
        _ = afterSeconds
        if let endError { throw endError }
        guard activities.removeValue(forKey: id) != nil else { throw PluginError.unavailable }
        if let fields {
            _ = fields
        }
        if let waiter = tokenWaiters.removeValue(forKey: id) {
            waiter.resume(throwing: PluginError.cancelled)
        }
        tokens.removeValue(forKey: id)
    }

    func waitPushToken(id: String) async throws -> Data {
        if let existing = tokens.removeValue(forKey: id) { return existing }
        guard activities[id] != nil else { throw PluginError.unavailable }
        return try await withCheckedThrowingContinuation { continuation in
            tokenWaiters[id] = continuation
        }
    }

    func list() -> [String] {
        Array(activities.keys).sorted()
    }

    func deliverToken(_ id: String, _ token: Data) {
        if let waiter = tokenWaiters.removeValue(forKey: id) {
            waiter.resume(returning: token)
        } else {
            tokens[id] = token
        }
    }
}

@MainActor
final class LiveActivitiesTests: XCTestCase {
    func testEnabledFlagsAndLifecycleWithInjectedService() async throws {
        let service = FakeLiveActivities()
        let plugin = LiveActivitiesPlugin(service: service)

        XCTAssertEqual(try await flag(plugin, method: 0), 1)
        service.areEnabled = false
        XCTAssertEqual(try await flag(plugin, method: 0), 0)
        XCTAssertEqual(try await flag(plugin, method: 1), 0)

        let started = try await plugin.invoke(method: 2, payload: startPayload())
        XCTAssertEqual(try identifier(started), "ride-1")
        XCTAssertEqual(service.activities["ride-1"]?.title, "Driver en route")
        XCTAssertEqual(service.activities["ride-1"]?.fields["eta"], "4")

        _ = try await plugin.invoke(method: 3, payload: updatePayload())
        XCTAssertEqual(service.activities["ride-1"]?.fields["eta"], "2")

        let listed = try await plugin.invoke(method: 6, payload: NativeBuffer(taking: lk_buffer()))
        XCTAssertEqual(try identifiers(listed), ["ride-1"])

        _ = try await plugin.invoke(method: 4, payload: endPayload())
        XCTAssertTrue(service.activities.isEmpty)
        plugin.close()
        do {
            _ = try await flag(plugin, method: 0)
            XCTFail("Closed plugin answered")
        } catch {
            XCTAssertEqual(error as? PluginError, .cancelled)
        }
    }

    func testPushTokenWaitAndUnavailableWithoutService() async throws {
        let service = FakeLiveActivities()
        let plugin = LiveActivitiesPlugin(service: service)
        _ = try await plugin.invoke(method: 2, payload: startPayload(push: true))
        let waiting = Task {
            try await plugin.invoke(method: 5, payload: idPayload("ride-1"))
        }
        await Task.yield()
        service.deliverToken("ride-1", Data([0xDE, 0xAD]))
        let token = try await waiting.value
        XCTAssertEqual(try bytes(token), Data([0xDE, 0xAD]))

        let unavailable = LiveActivitiesPlugin(service: nil)
        do {
            _ = try await unavailable.invoke(method: 0, payload: NativeBuffer(taking: lk_buffer()))
            XCTFail("Missing service should be unavailable")
        } catch {
            XCTAssertEqual(error as? PluginError, .unavailable)
        }
    }

    func testMalformedStartIsRejected() async {
        let plugin = LiveActivitiesPlugin(service: FakeLiveActivities())
        do {
            _ = try await plugin.invoke(method: 2, payload: NativeBuffer(taking: lk_buffer()))
            XCTFail("Empty start accepted")
        } catch {
            XCTAssertEqual(error as? PluginError, .invalidArgument)
        }
    }

    private func flag(_ plugin: LiveActivitiesPlugin, method: UInt32) async throws -> UInt8 {
        let data = try await plugin.invoke(method: method, payload: NativeBuffer(taking: lk_buffer()))
        XCTAssertEqual(data.count, 4)
        return data[0]
    }

    private func startPayload(push: Bool = false) throws -> NativeBuffer {
        var data = Data()
        try append("ride-1", to: &data)
        try append("ride", to: &data)
        try append("Driver en route", to: &data)
        append(UInt32(push ? 1 : 0), to: &data)
        append(UInt32(1), to: &data)
        try append("eta", to: &data)
        try append("4", to: &data)
        return buffer(data)
    }

    private func updatePayload() throws -> NativeBuffer {
        var data = Data()
        try append("ride-1", to: &data)
        append(UInt32(1), to: &data)
        try append("eta", to: &data)
        try append("2", to: &data)
        return buffer(data)
    }

    private func endPayload() throws -> NativeBuffer {
        var data = Data()
        try append("ride-1", to: &data)
        append(UInt32(1), to: &data) // immediate
        append(UInt32(0), to: &data)
        append(UInt32(0), to: &data) // no final content
        return buffer(data)
    }

    private func idPayload(_ id: String) throws -> NativeBuffer {
        var data = Data()
        try append(id, to: &data)
        return buffer(data)
    }

    private func buffer(_ data: Data) -> NativeBuffer {
        data.withUnsafeBytes { raw in
            var buffer = lk_buffer()
            let pointer = UnsafeMutablePointer<UInt8>.allocate(capacity: data.count)
            pointer.initialize(from: raw.bindMemory(to: UInt8.self).baseAddress!, count: data.count)
            buffer.data = pointer
            buffer.len = data.count
            buffer.capacity = data.count
            return NativeBuffer(taking: buffer)
        }
    }

    private func append(_ value: String, to data: inout Data) throws {
        let utf8 = Array(value.utf8)
        append(UInt32(utf8.count), to: &data)
        data.append(contentsOf: utf8)
    }

    private func append(_ value: UInt32, to data: inout Data) {
        var little = value.littleEndian
        withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
    }

    private func identifier(_ data: Data) throws -> String {
        try string(data, offset: 0).value
    }

    private func identifiers(_ data: Data) throws -> [String] {
        let count = Int(UInt32(littleEndian: data.prefix(4).withUnsafeBytes {
            $0.loadUnaligned(as: UInt32.self)
        }))
        var offset = 4
        var ids: [String] = []
        for _ in 0..<count {
            let next = try string(data, offset: offset)
            ids.append(next.value)
            offset = next.next
        }
        return ids
    }

    private func bytes(_ data: Data) throws -> Data {
        let length = Int(UInt32(littleEndian: data.prefix(4).withUnsafeBytes {
            $0.loadUnaligned(as: UInt32.self)
        }))
        return data.subdata(in: 4..<(4 + length))
    }

    private func string(_ data: Data, offset: Int) throws -> (value: String, next: Int) {
        let length = Int(UInt32(littleEndian: data.subdata(in: offset..<(offset + 4)).withUnsafeBytes {
            $0.loadUnaligned(as: UInt32.self)
        }))
        let start = offset + 4
        let end = start + length
        guard let value = String(data: data.subdata(in: start..<end), encoding: .utf8) else {
            throw PluginError.invalidArgument
        }
        return (value, end)
    }
}
