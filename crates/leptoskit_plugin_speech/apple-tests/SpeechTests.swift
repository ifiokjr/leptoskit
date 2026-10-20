import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitSpeech
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

private func mutedRequest() -> Data {
    var result = Data()
    for value: UInt32 in [1, 1000, 1000, 0] {
        var little = value.littleEndian
        withUnsafeBytes(of: &little) { result.append(contentsOf: $0) }
    }
    for text in ["Muted cancellation test", "", ""] {
        let data = Data(text.utf8)
        var count = UInt32(data.count).littleEndian
        withUnsafeBytes(of: &count) { result.append(contentsOf: $0) }
        result.append(data)
    }
    return result
}

@Test
func speechWireRejectsMalformedRequests() throws {
    let bytes = mutedRequest()
    #expect(try SpeechRequest(payload: buffer(bytes)).volume == 0)
    for count in 0..<bytes.count {
        #expect(throws: PluginError.self) { try SpeechRequest(payload: buffer(bytes.prefix(count))) }
    }
    var invalid = bytes
    invalid[4] = 0
    invalid[5] = 0
    #expect(throws: PluginError.invalidArgument) { try SpeechRequest(payload: buffer(invalid)) }
    invalid = bytes
    invalid[20] = 255
    #expect(throws: PluginError.invalidArgument) { try SpeechRequest(payload: buffer(invalid)) }
}

@Test
@MainActor
func cancellationBeforeDispatchAndCloseNeverStartAudio() async throws {
    let plugin = SpeechPlugin()
    let payload = try buffer(mutedRequest())
    let job = Task { try await plugin.invoke(method: 1, payload: payload) }
    job.cancel()
    do {
        _ = try await job.value
        Issue.record("A pre-cancelled speech request completed successfully")
    } catch {
        #expect(error as? PluginError == .cancelled)
    }
    #expect(try await plugin.invoke(method: 3, payload: buffer()) == Data([0, 0, 0, 0]))
    #expect(try await plugin.invoke(method: 2, payload: buffer()).isEmpty)
    plugin.close()
    plugin.close()
    do {
        _ = try await plugin.invoke(method: 0, payload: buffer())
        Issue.record("A closed speech plugin accepted work")
    } catch {
        #expect(error as? PluginError == .cancelled)
    }
}
