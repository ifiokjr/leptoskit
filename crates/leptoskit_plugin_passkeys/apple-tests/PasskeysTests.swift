import AuthenticationServices
import CLeptosKitBridge
import Foundation
import LeptosKitBridge
@testable import LeptosKitPasskeys
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

private func text(_ value: String) -> Data { Data(value.utf8) }

@Test
func base64URLRoundTripsWithoutPadding() {
    let data = Data([0xfb, 0xff, 0x01])
    #expect(Base64URL.encode(data) == "-_8B")
    #expect(Base64URL.decode("-_8B") == data)
    #expect(Base64URL.decode("AA") == Data([0]))
    #expect(Base64URL.decode("a+b/") == nil)
}

@Test
func creationAndRequestOptionsAreRead() throws {
    let creation = try CreationOptions(payload: buffer(text("""
    {"rp":{"id":"notes.example","name":"Notes"},"user":{"id":"AQID","name":"ada@notes.example",
     "displayName":"Ada"},"challenge":"BAUG","pubKeyCredParams":[],
     "excludeCredentials":[{"id":"Bwg","type":"public-key"}]}
    """)))
    #expect(creation.relyingParty == "notes.example")
    #expect(creation.userID == Data([1, 2, 3]))
    #expect(creation.challenge == Data([4, 5, 6]))
    #expect(creation.excluded == [Data([7, 8])])
    let request = try RequestOptions(payload: buffer(text(#"{"rpId":"notes.example","challenge":"CQ"}"#)))
    #expect(request.allowed.isEmpty)
    #expect(request.challenge == Data([9]))

    for bad in [#"{"challenge":"AA"}"#, #"{"rpId":"","challenge":"AA"}"#, #"{"rpId":"a","challenge":""}"#,
                #"{"rpId":"a","challenge":"A+"}"#, #"{"rpId":"a","challenge":"AA","allowCredentials":[{}]}"#,
                "[]", "nope"] {
        #expect(throws: PluginError.invalidArgument) { try RequestOptions(payload: buffer(text(bad))) }
    }
    #expect(throws: PluginError.invalidArgument) {
        try CreationOptions(payload: buffer(text(#"{"rp":{"id":"a"},"challenge":"AA","user":{"id":"AA"}}"#)))
    }
}

@Test
func answersAreWebAuthnJSON() throws {
    let made = try registrationJSON(credentialID: Data([1]), clientData: Data([2]), attestationObject: Data([3]))
    let registration = try #require(try JSONSerialization.jsonObject(with: made) as? [String: Any])
    #expect(registration["id"] as? String == "AQ")
    #expect(registration["type"] as? String == "public-key")
    let response = try #require(registration["response"] as? [String: Any])
    #expect(response["attestationObject"] as? String == "Aw")
    let signed = try assertionJSON(
        credentialID: Data([1]), clientData: Data([2]), authenticatorData: Data([3]), signature: Data([4]),
        userID: Data([5]))
    let assertion = try #require(try JSONSerialization.jsonObject(with: signed) as? [String: Any])

    let fields = try #require(assertion["response"] as? [String: Any])
    #expect(fields["userHandle"] as? String == "BQ")
    #expect(fields["signature"] as? String == "BA")
    let apple = try appleIDJSON(identityToken: text("a.b.c"), givenName: "Ada", familyName: "")
    #expect(String(data: apple, encoding: .utf8) == #"{"given_name":"Ada","id_token":"a.b.c"}"#)
    #expect(throws: PluginError.native) { try appleIDJSON(identityToken: text("a.b"), givenName: nil, familyName: nil) }
}

@Test
func noncesAndFailuresAreMapped() throws {
    #expect(try appleNonce(buffer(text("abc"))) == "abc")
    #expect(throws: PluginError.invalidArgument) { try appleNonce(buffer(text("a b"))) }
    #expect(throws: PluginError.invalidArgument) { try appleNonce(buffer(text(String(repeating: "a", count: 129)))) }
    #expect(PasskeysPlugin.failure(ASAuthorizationError(.canceled)) == .cancelled)
    #expect(PasskeysPlugin.failure(ASAuthorizationError(.notHandled)) == .unavailable)
    #expect(PasskeysPlugin.failure(ASAuthorizationError(.failed)) == .native)
    #expect(PasskeysPlugin.failure(CocoaError(.fileNoSuchFile)) == .native)
}

@Test
@MainActor
func nothingIsShownWithoutAWindow() async throws {
    #if os(iOS)
    let owner = UIViewController()
    #else
    let owner = NSViewController()
    #endif
    let plugin = PasskeysPlugin(context: NativePluginContext(owner: owner))
    #expect(plugin.name == "passkeys")
    await #expect(throws: PluginError.unavailable) {
        try await plugin.invoke(method: 1, payload: buffer(text(#"{"rpId":"notes.example","challenge":"AA"}"#)))
    }
    await #expect(throws: PluginError.unavailable) { try await plugin.invoke(method: 2, payload: buffer(text("n"))) }
    await #expect(throws: PluginError.unavailable) { try await plugin.invoke(method: 3, payload: buffer()) }
    await #expect(throws: PluginError.invalidArgument) {
        try await plugin.invoke(method: 0, payload: buffer(text("{}")))
    }
    plugin.close()
    await #expect(throws: PluginError.cancelled) { try await plugin.invoke(method: 2, payload: buffer(text("n"))) }
}
