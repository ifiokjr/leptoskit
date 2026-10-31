import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitWebAuth
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

private func authenticate(_ session: UInt32, _ url: String) -> Data {
    withUnsafeBytes(of: session.littleEndian) { Data($0) } + Data(url.utf8)
}

@Test
func returnAddressesAreTheBundleIdentifierAsAScheme() {
    #expect(webAuthReturnAddress(bundleIdentifier: "com.Example.App") == "com.example.app:/auth/native/return")
    #expect(webAuthReturnAddress(bundleIdentifier: "app") == nil)
    #expect(webAuthReturnAddress(bundleIdentifier: "com.my-app") == nil)
    #expect(webAuthReturnAddress(bundleIdentifier: "com.1app") == nil)
    let session = encodeWebAuthSession(id: 258, returnTo: "com.example.app:/auth/native/return")
    #expect(Array(session.prefix(4)) == [2, 1, 0, 0])
    #expect(String(data: session.dropFirst(4), encoding: .utf8) == "com.example.app:/auth/native/return")
}

@Test
func onlyProviderPagesOpen() throws {
    let request = try WebAuthRequest(payload: buffer(authenticate(7, "https://accounts.example/authorize?state=x")))
    #expect(request.session == 7)
    #expect(request.url.host == "accounts.example")
    #expect(try WebAuthRequest(payload: buffer(authenticate(1, "http://localhost:8080/authorize"))).session == 1)
    for page in ["http://example.com/authorize", "file:///etc/passwd", "javascript:alert(1)",
                 "https://user:pass@example.com/", "https://example.com/a b"] {
        #expect(throws: PluginError.invalidArgument) { try WebAuthRequest(payload: buffer(authenticate(1, page))) }
    }
    #expect(throws: PluginError.invalidArgument) { try WebAuthRequest(payload: buffer(Data([1, 0, 0]))) }
    #expect(throws: PluginError.invalidArgument) { try WebAuthRequest(payload: buffer(Data([1, 0, 0, 0, 0xff]))) }
}

@Test
func theBrowserMustComeBackToTheSessionsAddress() throws {
    let returnTo = "com.example.app:/auth/native/return"
    let back = try #require(URL(string: "com.example.app:/auth/native/return?state=a&code=b"))
    #expect(webAuthCameBack(back, to: returnTo))
    #expect(!webAuthCameBack(try #require(URL(string: "com.evil.app:/auth/native/return?state=a")), to: returnTo))
    #expect(!webAuthCameBack(try #require(URL(string: "com.example.app:/auth/native/other?state=a")), to: returnTo))
    #expect(!webAuthCameBack(try #require(URL(string: "com.example.app:/auth/native/return#a")), to: returnTo))
}

@Test
@MainActor
func sessionsMustBeginBeforeTheyOpen() async throws {
    #if os(iOS)
    let owner = UIViewController()
    #else
    let owner = NSViewController()
    #endif
    let plugin = WebAuthPlugin(context: NativePluginContext(owner: owner), bundleIdentifier: "com.example.App")
    #expect(plugin.name == "web-auth")
    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 0, payload: buffer(Data([1])))
    }
    await #expect(throws: PluginError.invalidCall) {
        try await plugin.invoke(method: 1, payload: buffer(authenticate(1, "https://example.com/")))
    }
    let begun = try await plugin.invoke(method: 0, payload: buffer())
    #expect(String(data: begun.dropFirst(4), encoding: .utf8) == "com.example.app:/auth/native/return")
    await #expect(throws: PluginError.invalidCall) {
        try await plugin.invoke(method: 1, payload: buffer(authenticate(99, "https://example.com/")))
    }
    // The detached owner has no window, so nothing is presented.
    let again = try await plugin.invoke(method: 0, payload: buffer())
    await #expect(throws: PluginError.unavailable) {
        try await plugin.invoke(method: 1, payload: buffer(authenticate(UInt32(again[0]), "https://example.com/")))
    }
    await #expect(throws: PluginError.unavailable) { try await plugin.invoke(method: 2, payload: buffer()) }
    let unnamed = WebAuthPlugin(context: NativePluginContext(owner: owner), bundleIdentifier: nil)
    await #expect(throws: PluginError.unavailable) { try await unnamed.invoke(method: 0, payload: buffer()) }
    plugin.close()
    await #expect(throws: PluginError.cancelled) { try await plugin.invoke(method: 0, payload: buffer()) }
}
