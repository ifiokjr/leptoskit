import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitLocalAuthentication
import LocalAuthentication
import Testing
#if os(iOS)
import UIKit
#else
import AppKit
#endif

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

private func request() -> Data {
    var result = authenticationFields([1, 0])
    for text in ["Unlock your notes", "Cancel"] {
        let bytes = Data(text.utf8)
        result.append(authenticationFields([UInt32(bytes.count)]))
        result.append(bytes)
    }
    return result
}

@Test
func authenticationWireRejectsMalformedRequestsWithoutNativeEvaluation() throws {
    let bytes = request()
    #expect(try AuthenticationRequest(payload: buffer(bytes)).policy == 0)
    for count in 0..<bytes.count {
        #expect(throws: PluginError.self) { try AuthenticationRequest(payload: buffer(bytes.prefix(count))) }
    }
    var invalid = bytes
    invalid[4] = 2
    #expect(throws: PluginError.invalidArgument) { try AuthenticationRequest(payload: buffer(invalid)) }
    invalid = bytes
    invalid[12] = 255
    #expect(throws: PluginError.invalidArgument) { try AuthenticationRequest(payload: buffer(invalid)) }
}

@Test
@MainActor
func capabilityQueriesAndPreCancelledWorkDoNotShowAuthenticationUI() async throws {
    let plugin = LocalAuthenticationPlugin()
    defer { plugin.close() }
    let capabilities = try await plugin.invoke(method: 0, payload: buffer(authenticationFields([0])))
    #expect(capabilities.count == 20)
    #expect(capabilities.prefix(8) == authenticationFields([1, 0]))
    let payload = try buffer(request())
    let job = Task { try await plugin.invoke(method: 1, payload: payload) }
    job.cancel()
    do {
        _ = try await job.value
        Issue.record("Pre-cancelled authentication must not evaluate a policy")
    } catch {
        #expect(error as? PluginError == .cancelled)
    }
    #expect(try await plugin.invoke(method: 2, payload: buffer()).isEmpty)
    plugin.close()
    do {
        _ = try await plugin.invoke(method: 0, payload: buffer(authenticationFields([0])))
        Issue.record("A closed authentication plugin accepted work")
    } catch {
        #expect(error as? PluginError == .cancelled)
    }
}

@Test
@MainActor
func backgroundNotificationCancelsPendingAuthenticationWithoutSystemUI() async throws {
    let plugin = LocalAuthenticationPlugin()
    defer { plugin.close() }
    let decoded = try AuthenticationRequest(payload: buffer(request()))
    var callback: (@Sendable (Bool, (any Error)?) -> Void)?
    let pending = Task {
        try await plugin.evaluate(LAContext(), request: decoded) { callback = $0 }
    }
    defer { pending.cancel() }
    for _ in 0..<100 where callback == nil { await Task.yield() }
    let completion = try #require(callback)
    #if os(iOS)
    let notification = UIApplication.didEnterBackgroundNotification
    #else
    let notification = NSApplication.didHideNotification
    #endif
    NotificationCenter.default.post(name: notification, object: nil)
    do {
        _ = try await pending.value
        Issue.record("Backgrounded authentication remained active")
    } catch {
        #expect(error as? PluginError == .cancelled)
    }
    // A late OS callback must not revive a cancelled request.
    completion(true, nil)
    await Task.yield()
    #expect(try await plugin.invoke(method: 2, payload: buffer()).isEmpty)
}
