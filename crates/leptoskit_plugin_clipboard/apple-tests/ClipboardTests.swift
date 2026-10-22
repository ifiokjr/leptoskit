import AppKit
import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitClipboard
import Testing

private func buffer(_ bytes: Data = Data()) throws -> NativeBuffer {
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
    let call = bytes.withUnsafeBytes {
        lk_bridge_submit(bridge, 0, 0, $0.baseAddress?.assumingMemoryBound(to: UInt8.self), $0.count)
    }
    #expect(call > 0)
    var request = lk_request()
    #expect(lk_bridge_next_request(bridge, &request) == 1)
    return NativeBuffer(taking: request.payload)
}

/// Uses the real general pasteboard. This one test owns every write and
/// restores the text that was there before.
@Test
@MainActor
func textRoundTripsThroughThePasteboardAndClears() async throws {
    let pasteboard = NSPasteboard.general
    let previous = pasteboard.string(forType: .string)
    defer {
        pasteboard.clearContents()
        if let previous { pasteboard.setString(previous, forType: .string) }
    }
    let plugin = ClipboardPlugin()
    #expect(plugin.name == "clipboard")
    #expect(plugin.methods == [0, 1, 2])
    let text = "LeptosKit clipboard \u{2713}"
    #expect(try await plugin.invoke(method: 1, payload: buffer(Data(text.utf8))).isEmpty)
    #expect(pasteboard.string(forType: .string) == text)
    #expect(try await plugin.invoke(method: 0, payload: buffer()) == Data(text.utf8))
    #expect(try await plugin.invoke(method: 2, payload: buffer()).isEmpty)
    #expect(pasteboard.string(forType: .string) == nil)
    #expect(try await plugin.invoke(method: 0, payload: buffer()).isEmpty)
}

@Test
@MainActor
func malformedRequestsFailBeforeReachingThePasteboard() async throws {
    let plugin = ClipboardPlugin()
    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 0, payload: buffer(Data([1])))
    }
    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 2, payload: buffer(Data([1])))
    }
    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 1, payload: buffer(Data([0xFF, 0xFE])))
    }
    await #expect(throws: PluginError.unavailable) {
        try await plugin.invoke(method: 3, payload: buffer())
    }
}
