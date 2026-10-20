import Foundation
import LeptosKitBridge
@testable import LeptosKitSharing
import XCTest

final class SharingWireTests: XCTestCase {
    private func wire(_ value: String, kind: UInt32 = 0) -> Data {
        var result = Data()
        for var number in [kind.littleEndian, UInt32(value.utf8.count).littleEndian] {
            withUnsafeBytes(of: &number) { result.append(contentsOf: $0) }
        }
        result.append(contentsOf: value.utf8)
        return result
    }

    func testBoundsAndHostileURLsBeforePresentation() throws {
        for (value, kind): (String, UInt32) in [("hello\n世界", 0), ("https://example.test/a%20b", 1)] {
            let bytes = wire(value, kind: kind)
            _ = try bytes.withUnsafeBytes(ShareContent.decode)
            for count in 0..<bytes.count {
                XCTAssertThrowsError(try bytes.prefix(count).withUnsafeBytes(ShareContent.decode))
            }
        }
        for value in ["file:///private", "content://provider/item", "https://user:secret@example.test",
                      "https:///empty", "https://example.test/%xy", "https://example.test/a b"] {
            XCTAssertThrowsError(try wire(value, kind: 1).withUnsafeBytes(ShareContent.decode))
        }
        XCTAssertThrowsError(try wire(String(repeating: "a", count: 65537)).withUnsafeBytes(ShareContent.decode))
        XCTAssertThrowsError(try wire("").withUnsafeBytes(ShareContent.decode))
    }
}

@MainActor
final class SharingSessionTests: XCTestCase {
    func testCancellationRetiresCallerAndLateCompletionIsHarmless() async {
        var session: ShareSession?
        do {
            _ = try await withCheckedThrowingContinuation { continuation in
                let pending = ShareSession(continuation)
                session = pending
                pending.complete(.failure(PluginError.cancelled))
                pending.complete(.success(Data()))
            }
            XCTFail("Expected cancellation")
        } catch { XCTAssertEqual(error as? PluginError, .cancelled) }
        // A retained external service may finish after caller cancellation.
        session?.complete(.success(Data()))
        let first = session?.id
        let result = try? await withCheckedThrowingContinuation { continuation in
            let replacement = ShareSession(continuation)
            XCTAssertNotEqual(first, replacement.id)
            session?.complete(.failure(PluginError.native))
            replacement.complete(.success(Data()))
        }
        XCTAssertEqual(result, Data())
    }
}

#if canImport(AppKit)
import AppKit
import CLeptosKitBridge

@MainActor
final class SharingHostTests: XCTestCase {
    func testDetachedOwnerAndClosedPluginCannotPresent() async throws {
        let owner = NSViewController()
        let plugin = SharingPlugin(context: NativePluginContext(owner: owner))
        let payload = try ownedPayload()
        do {
            _ = try await plugin.invoke(method: 0, payload: payload)
            XCTFail("Detached presenter must fail")
        } catch { XCTAssertEqual(error as? PluginError, .unavailable) }
        XCTAssertFalse(owner.isViewLoaded)
        plugin.close()
        plugin.close()
        do {
            _ = try await plugin.invoke(method: 0, payload: payload)
            XCTFail("Closed plugin must fail")
        } catch { XCTAssertEqual(error as? PluginError, .cancelled) }
    }

    private func ownedPayload() throws -> NativeBuffer {
        let bridge = try XCTUnwrap(lk_bridge_create())
        defer { lk_bridge_destroy(bridge) }
        let name = Array("sharing".utf8)
        let methods: [UInt32] = [0]
        let plugin = name.withUnsafeBufferPointer { name in
            methods.withUnsafeBufferPointer { methods in
                lk_bridge_register(bridge, name.baseAddress, name.count, methods.baseAddress, methods.count)
            }
        }
        XCTAssertEqual(plugin, 0)
        XCTAssertEqual(lk_bridge_grant(bridge, 0, 0, true), 0)
        let bytes: [UInt8] = [0, 0, 0, 0, 1, 0, 0, 0, 120]
        let call = bytes.withUnsafeBufferPointer { lk_bridge_submit(bridge, 0, 0, $0.baseAddress, $0.count) }
        XCTAssertGreaterThan(call, 0)
        var request = lk_request()
        XCTAssertEqual(lk_bridge_next_request(bridge, &request), 1)
        return NativeBuffer(taking: request.payload)
    }
}
#endif
