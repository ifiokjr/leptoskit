import Foundation
import LeptosKitBridge

/// WebAuthn's base64url, without padding.
enum Base64URL {
    static func decode(_ text: String) -> Data? {
        guard text.utf8.allSatisfy({ byte in
            (65...90).contains(byte) || (97...122).contains(byte) || (48...57).contains(byte)
                || byte == 45 || byte == 95 || byte == 61
        }) else { return nil }
        var base64 = text.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        base64 = base64.trimmingCharacters(in: CharacterSet(charactersIn: "="))
        base64 += String(repeating: "=", count: (4 - base64.count % 4) % 4)
        return Data(base64Encoded: base64)
    }

    static func encode(_ data: Data) -> String {
        data.base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
}

private func object(_ payload: NativeBuffer) throws -> [String: Any] {
    let bytes = payload.withUnsafeBytes { Data($0) }
    guard !bytes.isEmpty, bytes.count <= 64 * 1024,
          let value = try? JSONSerialization.jsonObject(with: bytes),
          let object = value as? [String: Any] else { throw PluginError.invalidArgument }
    return object
}

private func bytes(_ value: Any?) throws -> Data {
    guard let text = value as? String, let data = Base64URL.decode(text), !data.isEmpty else {
        throw PluginError.invalidArgument
    }
    return data
}

private func credentials(_ value: Any?) throws -> [Data] {
    guard let value else { return [] }
    guard let list = value as? [[String: Any]] else { throw PluginError.invalidArgument }
    return try list.map { try bytes($0["id"]) }
}

/// Passkey creation options, from `PublicKeyCredentialCreationOptionsJSON`.
struct CreationOptions {
    let relyingParty: String
    let challenge: Data
    let userID: Data
    let userName: String
    let excluded: [Data]

    init(payload: NativeBuffer) throws {
        let options = try object(payload)
        guard let party = options["rp"] as? [String: Any], let relyingParty = party["id"] as? String,
              !relyingParty.isEmpty,
              let user = options["user"] as? [String: Any], let userName = user["name"] as? String,
              !userName.isEmpty else { throw PluginError.invalidArgument }
        self.relyingParty = relyingParty
        self.userName = userName
        challenge = try bytes(options["challenge"])
        userID = try bytes(user["id"])
        excluded = try credentials(options["excludeCredentials"])
    }
}

/// Passkey request options, from `PublicKeyCredentialRequestOptionsJSON`.
struct RequestOptions {
    let relyingParty: String
    let challenge: Data
    let allowed: [Data]

    init(payload: NativeBuffer) throws {
        let options = try object(payload)
        guard let relyingParty = options["rpId"] as? String, !relyingParty.isEmpty else {
            throw PluginError.invalidArgument
        }
        self.relyingParty = relyingParty
        challenge = try bytes(options["challenge"])
        allowed = try credentials(options["allowCredentials"])
    }
}

/// Sign in with Apple's nonce: 1 to 128 printable ASCII characters.
func appleNonce(_ payload: NativeBuffer) throws -> String {
    let nonce = try payload.utf8()
    guard (1...128).contains(nonce.utf8.count), nonce.utf8.allSatisfy({ $0 >= 33 && $0 <= 126 }) else {
        throw PluginError.invalidArgument
    }
    return nonce
}

private func json(_ value: [String: Any]) throws -> Data {
    try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
}

/// `RegistrationResponseJSON`.
func registrationJSON(credentialID: Data, clientData: Data, attestationObject: Data) throws -> Data {
    let id = Base64URL.encode(credentialID)
    return try json([
        "id": id, "rawId": id, "type": "public-key", "clientExtensionResults": [String: Any](),
        "response": [
            "clientDataJSON": Base64URL.encode(clientData),
            "attestationObject": Base64URL.encode(attestationObject),
            "transports": ["internal", "hybrid"]
        ]
    ])
}

/// `AuthenticationResponseJSON`.
func assertionJSON(
    credentialID: Data, clientData: Data, authenticatorData: Data, signature: Data, userID: Data?
) throws -> Data {
    let id = Base64URL.encode(credentialID)
    var response: [String: Any] = [
        "clientDataJSON": Base64URL.encode(clientData),
        "authenticatorData": Base64URL.encode(authenticatorData),
        "signature": Base64URL.encode(signature)
    ]
    if let userID { response["userHandle"] = Base64URL.encode(userID) }
    return try json([
        "id": id, "rawId": id, "type": "public-key", "clientExtensionResults": [String: Any](),
        "response": response
    ])
}

/// The answer to Sign in with Apple: its ID token, and the name it shared.
func appleIDJSON(identityToken: Data, givenName: String?, familyName: String?) throws -> Data {
    guard let token = String(data: identityToken, encoding: .utf8), token.split(separator: ".").count == 3 else {
        throw PluginError.native
    }
    var answer: [String: Any] = ["id_token": token]
    if let givenName, !givenName.isEmpty { answer["given_name"] = givenName }
    if let familyName, !familyName.isEmpty { answer["family_name"] = familyName }
    return try json(answer)
}
