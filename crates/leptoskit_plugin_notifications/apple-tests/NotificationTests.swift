import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitNotifications
import Testing
import UserNotifications

@MainActor
private final class Service: NotificationService {
    var pause = false
    var continuation: CheckedContinuation<Void, any Error>?
    var posted: [String] = []
    var removed: [String] = []
    var existing: [String] = []
    func authorization() async -> UNAuthorizationStatus { .authorized }
    func requestAuthorization() async throws {}
    func add(_ request: UNNotificationRequest) async throws {
        posted.append(request.identifier)

        if pause { try await withCheckedThrowingContinuation { continuation = $0 } }
    }
    func identifiers() async -> [String] { existing }
    func remove(_ identifiers: [String]) { removed += identifiers }
    func complete() { continuation?.resume(); continuation = nil }
}

private func bytes(_ fields: [String]) -> [UInt8] {
    var result: [UInt8] = []

    for field in fields {
        let value = Array(field.utf8)
        var length = UInt32(value.count).littleEndian
        withUnsafeBytes(of: &length) { result += $0 }
        result += value
    }

    return result
}

private func payload(_ bytes: [UInt8]) throws -> NativeBuffer {
    let bridge = try #require(lk_bridge_create())
    defer { lk_bridge_destroy(bridge) }
    let name = Array("test".utf8)
    let methods: [UInt32] = [0]
    let id = name.withUnsafeBufferPointer { name in
        methods.withUnsafeBufferPointer { methods in
            lk_bridge_register(bridge, name.baseAddress, name.count, methods.baseAddress, methods.count)
        }
    }
    #expect(id == 0)
    #expect(lk_bridge_grant(bridge, 0, 0, true) == 0)
    let call = bytes.withUnsafeBufferPointer { lk_bridge_submit(bridge, 0, 0, $0.baseAddress, $0.count) }

    #expect(call > 0)
    var request = lk_request()
    #expect(lk_bridge_next_request(bridge, &request) == 1)

    return NativeBuffer(taking: request.payload)
}

@Test
@MainActor
func cancellationRemovesLatePostAndPreventsReplacementRace() async throws {
    let service = Service()
    service.pause = true
    let plugin = NotificationsPlugin(service: service)
    let content = try payload(bytes(["job", "Title", "Body", "general"]))
    let task = Task { try await plugin.invoke(method: 2, payload: content) }
    defer { service.complete(); task.cancel(); plugin.close() }

    for _ in 0..<100 where service.continuation == nil { await Task.yield() }
    try #require(service.continuation != nil)
    _ = try await plugin.invoke(method: 3, payload: payload(bytes(["job"])))
    await #expect(throws: PluginError.busy) { try await plugin.invoke(method: 2, payload: content) }
    service.complete()
    await #expect(throws: PluginError.cancelled) { try await task.value }
    #expect(service.removed.filter { $0 == "leptoskit.notifications.job" }.count >= 2)
    service.pause = false
    let removalCount = service.removed.count
    _ = try await plugin.invoke(method: 2, payload: content)
    plugin.close()
    #expect(service.removed.count == removalCount, "Closing must preserve completed notifications")
}

@Test
@MainActor
func closeCancelsPendingPostsAndCancellationIsNamespaceScoped() async throws {
    let service = Service()
    service.existing = ["other-provider", "leptoskit.notifications.old"]
    let plugin = NotificationsPlugin(service: service)
    _ = try await plugin.invoke(method: 6, payload: payload([]))
    #expect(service.removed == ["leptoskit.notifications.old"])
    service.pause = true
    let task = Task { try await plugin.invoke(method: 2, payload: payload(bytes(["new", "Title", "", "general"]))) }
    defer { service.complete(); task.cancel(); plugin.close() }

    for _ in 0..<100 where service.continuation == nil { await Task.yield() }
    try #require(service.continuation != nil)
    plugin.close()
    service.complete()
    await #expect(throws: PluginError.cancelled) { try await task.value }
    #expect(!service.removed.contains("other-provider"))
    await #expect(throws: PluginError.cancelled) { try await plugin.invoke(method: 0, payload: payload([])) }
}

@Test
@MainActor
func taskCancellationRetractsAnUnabortableLatePost() async throws {
    let service = Service()
    service.pause = true
    let plugin = NotificationsPlugin(service: service)
    let task = Task { try await plugin.invoke(method: 2, payload: payload(bytes(["job", "Title", "", "general"]))) }
    defer { service.complete(); task.cancel(); plugin.close() }

    for _ in 0..<100 where service.continuation == nil { await Task.yield() }
    try #require(service.continuation != nil)
    task.cancel()

    for _ in 0..<100 where service.removed.isEmpty { await Task.yield() }
    #expect(service.removed == ["leptoskit.notifications.job"])
    service.complete()
    await #expect(throws: PluginError.cancelled) { try await task.value }
    #expect(service.removed.count == 2)
}

@Test
@MainActor
func malformedMessagesNeverReachTheNotificationService() async throws {
    let service = Service()
    let plugin = NotificationsPlugin(service: service)
    let invalidMessages = [
        bytes(["id", "", "body", "general"]), bytes(["id", "Title", "bad\0body", "general"]), [255, 0, 0, 0]
    ]

    for invalid in invalidMessages {
        await #expect(throws: PluginError.invalidArgument) {
            try await plugin.invoke(method: 2, payload: payload(invalid))
        }
    }

    #expect(service.posted.isEmpty)
    #expect(service.removed.isEmpty)
    plugin.close()
}
