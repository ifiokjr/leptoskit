import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitAppShortcuts
import Testing

@MainActor
private final class PublisherSpy: ShortcutPublisher {
    var items: [[AppShortcutItem]] = []
    func set(_ items: [AppShortcutItem]) throws { self.items.append(items) }
    func clear() throws { items.append([]) }
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

private func encodeList(_ items: [(String, String, String)]) -> [UInt8] {
    var bytes: [UInt8] = []
    func appendU32(_ value: UInt32) {
        var little = value.littleEndian
        withUnsafeBytes(of: &little) { bytes.append(contentsOf: $0) }
    }
    func appendString(_ value: String) {
        let utf8 = Array(value.utf8)
        appendU32(UInt32(utf8.count))
        bytes.append(contentsOf: utf8)
    }
    appendU32(UInt32(items.count))
    for item in items {
        appendString(item.0)
        appendString(item.1)
        appendString(item.2)
    }
    return bytes
}

@Test
@MainActor
func setClearAndConsumeLaunchUsePublisherWithoutSystemUI() async throws {
    let publisher = PublisherSpy()
    let plugin = AppShortcutsPlugin(publisher: publisher)
    let encoded = encodeList([("inbox", "Inbox", "Mail"), ("compose", "Compose", "")])
    #expect(try await plugin.invoke(method: 0, payload: payload(encoded)) == Data())
    #expect(publisher.items.last?.count == 2)
    #expect(publisher.items.last?.first?.id == "inbox")
    try plugin.noteLaunch(id: "inbox")
    let launch = try await plugin.invoke(method: 2, payload: payload())
    #expect(String(data: launch.dropFirst(4), encoding: .utf8) == "inbox")
    let empty = try await plugin.invoke(method: 2, payload: payload())
    #expect(empty == Data([0, 0, 0, 0]))
    #expect(try await plugin.invoke(method: 1, payload: payload()) == Data())
    #expect(publisher.items.last?.isEmpty == true)
    plugin.close()
    await #expect(throws: PluginError.cancelled) {
        try await plugin.invoke(method: 0, payload: payload(encoded))
    }
}

@Test
@MainActor
func malformedPayloadNeverReachesPublisher() async throws {
    let publisher = PublisherSpy()
    let plugin = AppShortcutsPlugin(publisher: publisher)
    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 0, payload: payload([1]))
    }
    #expect(publisher.items.isEmpty)
}
