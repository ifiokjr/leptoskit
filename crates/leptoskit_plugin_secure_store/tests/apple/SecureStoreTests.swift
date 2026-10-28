import Foundation
import LeptosKitBridge
@testable import LeptosKitSecureStore
import XCTest

final class SecureStoreTests: XCTestCase {
    func testArchiveQuotaSurvivesReloadAndRejectsBeforeCommit() throws {
        let quota = SecureStoreQuota(maxBytes: 64, maxEntries: 2)
        var archive = SecureArchive()
        archive.set(service: "login", key: "a", value: Data([1, 2]))
        archive.set(service: "login", key: "b", value: Data())
        let saved = try archive.encode(quota: quota)
        var reloaded = try SecureArchive(data: saved)
        reloaded.set(service: "login", key: "c", value: Data())
        XCTAssertThrowsError(try reloaded.encode(quota: quota)) { error in
            XCTAssertEqual(error as? PluginError, .tooLarge)
        }
        XCTAssertEqual(try SecureArchive(data: saved).value(service: "login", key: "a"), Data([1, 2]))
        reloaded.remove(service: "login", key: "c")
        reloaded.set(service: "login", key: "a", value: Data(repeating: 0, count: 40))
        XCTAssertThrowsError(try reloaded.encode(quota: quota))
        reloaded.remove(service: "login", key: "a")
        reloaded.set(service: "login", key: "c", value: Data([3]))
        XCTAssertNoThrow(try reloaded.encode(quota: quota))
        for length in 0..<saved.count {
            XCTAssertThrowsError(try SecureArchive(data: saved.prefix(length)))
        }
    }

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
