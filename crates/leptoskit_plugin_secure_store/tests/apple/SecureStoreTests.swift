import Foundation
import LeptosKitBridge
@testable import LeptosKitSecureStore
import XCTest

final class SecureStoreTests: XCTestCase {
    func testBinaryFramingAndEmptySecret() throws {
        let payload = request(service: "login", key: "token", secret: Data([0, 127, 255]))
        let decoded = try payload.withUnsafeBytes { try SecureRequest(method: 1, bytes: $0) }
        XCTAssertEqual(decoded.service, "login")
        XCTAssertEqual(decoded.key, "token")
        XCTAssertEqual(decoded.secret, Data([0, 127, 255]))
        let empty = request(service: "login", key: "token", secret: Data())
        let emptyDecoded = try empty.withUnsafeBytes { try SecureRequest(method: 1, bytes: $0) }
        XCTAssertEqual(emptyDecoded.secret, Data())
    }

    func testRejectsTruncationAndTrailers() {
        let payload = request(service: "login", key: "token", secret: Data([255]))
        for count in 0..<payload.count {
            XCTAssertThrowsError(try payload.prefix(count).withUnsafeBytes {
                try SecureRequest(method: 1, bytes: $0)
            })
        }
        var trailing = payload
        trailing.append(0)
        XCTAssertThrowsError(try trailing.withUnsafeBytes { try SecureRequest(method: 1, bytes: $0) })
        XCTAssertThrowsError(try payload.withUnsafeBytes { try SecureRequest(method: 0, bytes: $0) })
    }

    func testRejectsInvalidNamesAndOversizedSecret() {
        for name in ["", "../escape", "é", "a b", String(repeating: "a", count: 129)] {
            let payload = request(service: "test", key: name)
            XCTAssertThrowsError(try payload.withUnsafeBytes { try SecureRequest(method: 0, bytes: $0) })
        }
        let payload = request(service: "test", key: "large", secret: Data(repeating: 0, count: 2049))
        XCTAssertThrowsError(try payload.withUnsafeBytes { try SecureRequest(method: 1, bytes: $0) }) { error in
            XCTAssertEqual(error as? PluginError, .tooLarge)
        }
    }

    private func request(service: String, key: String, secret: Data? = nil) -> Data {
        var result = Data()
        func append(_ value: Data) {
            var length = UInt32(value.count).littleEndian
            withUnsafeBytes(of: &length) { result.append(contentsOf: $0) }
            result.append(value)
        }
        append(Data(service.utf8))
        append(Data(key.utf8))
        if let secret { append(secret) }
        return result
    }
}
