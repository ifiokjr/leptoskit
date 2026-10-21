import CLeptosKitBridge
import LeptosKitBridge
@testable import LeptosKitKeepAwake
import Testing

@Test
@MainActor
func cancellationReleasesOnlyItsLeaseAndCloseCancelsTheRest() async throws {
    let plugin = KeepAwakePlugin()
    let empty = NativeBuffer(taking: lk_buffer())
    let first = Task { try await plugin.invoke(method: 0, payload: empty) }
    let second = Task { try await plugin.invoke(method: 0, payload: empty) }
    defer { first.cancel(); second.cancel(); plugin.close() }
    for _ in 0..<100 where plugin.activeLeaseCount < 2 { await Task.yield() }
    try #require(plugin.activeLeaseCount == 2)
    first.cancel()
    await #expect(throws: PluginError.cancelled) { try await first.value }
    #expect(plugin.activeLeaseCount == 1)
    plugin.close()
    await #expect(throws: PluginError.cancelled) { try await second.value }
    #expect(plugin.activeLeaseCount == 0)
    plugin.close()
    await #expect(throws: PluginError.cancelled) { try await plugin.invoke(method: 0, payload: empty) }
}

@Test
@MainActor
func cancelledBeforeInvocationDoesNotAcquireAnAssertion() async throws {
    let plugin = KeepAwakePlugin()
    let task = Task { try await plugin.invoke(method: 0, payload: NativeBuffer(taking: lk_buffer())) }
    task.cancel()
    await #expect(throws: CancellationError.self) { try await task.value }
    #expect(plugin.activeLeaseCount == 0)
    plugin.close()
}
