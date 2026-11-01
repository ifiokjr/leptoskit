import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitStoreReview
import Testing

#if canImport(AppKit)
import AppKit

@MainActor
private final class ReviewSpy: ReviewService {
    var owners: [NSViewController] = []
    func request(from owner: NativePresentationOwner) throws { owners.append(owner) }
}

private func payload(_ bytes: [UInt8] = []) throws -> NativeBuffer {
    let bridge = try #require(lk_bridge_create())
    defer { lk_bridge_destroy(bridge) }
    let name = Array("test".utf8)
    let methods: [UInt32] = [0]
    let plugin = name.withUnsafeBufferPointer { name in
        methods.withUnsafeBufferPointer { methods in
            lk_bridge_register(bridge, name.baseAddress, name.count, methods.baseAddress, methods.count)
        }
    }
    #expect(plugin == 0)
    #expect(lk_bridge_grant(bridge, 0, 0, true) == 0)
    let call = bytes.withUnsafeBufferPointer { lk_bridge_submit(bridge, 0, 0, $0.baseAddress, $0.count) }
    #expect(call > 0)
    var request = lk_request()
    #expect(lk_bridge_next_request(bridge, &request) == 1)

    return NativeBuffer(taking: request.payload)
}

@Test
@MainActor
func ownedWindowReadinessAndCloseNeverCallRealStoreKit() async throws {
    _ = NSApplication.shared
    let owner = NSViewController()
    owner.view = NSView(frame: NSRect(x: 0, y: 0, width: 100, height: 100))
    let context = NativePluginContext(owner: owner)
    let service = ReviewSpy()
    let plugin = StoreReviewPlugin(context: context, service: service)
    let window = NSWindow(contentViewController: owner)
    window.isReleasedWhenClosed = false

    defer { plugin.close(); window.orderOut(nil); window.contentViewController = nil }
    #expect(try await plugin.invoke(method: 0, payload: payload()) == Data([2, 0, 0, 0]))
    await #expect(throws: PluginError.unavailable) { try await plugin.invoke(method: 1, payload: payload()) }
    #expect(service.owners.isEmpty)
    window.orderFront(nil)
    #expect(try await plugin.invoke(method: 0, payload: payload()) == Data([1, 0, 0, 0]))
    #expect(service.owners.isEmpty, "Availability must never request a review")
    #expect(try await plugin.invoke(method: 1, payload: payload()) == Data([1, 0, 0, 0]))
    #expect(service.owners.count == 1)
    #expect(service.owners.first === owner)
    context.invalidate()
    await #expect(throws: PluginError.unavailable) { try await plugin.invoke(method: 1, payload: payload()) }
    plugin.close()
    await #expect(throws: PluginError.cancelled) { try await plugin.invoke(method: 0, payload: payload()) }
    #expect(service.owners.count == 1)
}

@Test
@MainActor
func invalidPayloadAndCancelledTaskNeverReachReviewService() async throws {
    let owner = NSViewController()
    let service = ReviewSpy()
    let plugin = StoreReviewPlugin(context: NativePluginContext(owner: owner), service: service)

    for method: UInt32 in [0, 1] {
        await #expect(throws: PluginError.invalidArgument) {
            try await plugin.invoke(method: method, payload: payload([0]))
        }
    }

    let task = Task {
        withUnsafeCurrentTask { $0?.cancel() }

        return try await plugin.invoke(method: 1, payload: payload())
    }
    await #expect(throws: CancellationError.self) { try await task.value }
    #expect(service.owners.isEmpty)
    plugin.close()
}
#endif
