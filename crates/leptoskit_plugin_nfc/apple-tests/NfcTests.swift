import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitNfc
import Testing

@MainActor
private final class ReaderSpy: NfcReader {
    var status: NfcAvailability = .ready
    var record: NfcRecord = NfcRecord(kind: 0, value: "hello")
    var reads = 0

    func availability() -> NfcAvailability { status }
    func read() async throws -> NfcRecord {
        reads += 1
        return record
    }
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
func availabilityAndReadUseInjectedReader() async throws {
    let reader = ReaderSpy()
    let plugin = NfcPlugin(reader: reader)
    #expect(try await plugin.invoke(method: 0, payload: payload()) == Data([2, 0, 0, 0]))
    let reply = try await plugin.invoke(method: 1, payload: payload())
    #expect(reader.reads == 1)
    #expect(reply.starts(with: [0, 0, 0, 0, 5, 0, 0, 0]))
    #expect(String(data: reply.dropFirst(8), encoding: .utf8) == "hello")
    plugin.close()
    await #expect(throws: PluginError.cancelled) {
        try await plugin.invoke(method: 0, payload: payload())
    }
}

@Test
@MainActor
func trailingPayloadIsRejected() async throws {
    let reader = ReaderSpy()
    let plugin = NfcPlugin(reader: reader)
    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 0, payload: payload([0]))
    }
    #expect(reader.reads == 0)
}
