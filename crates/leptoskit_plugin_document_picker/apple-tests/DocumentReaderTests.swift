import Foundation
import LeptosKitBridge
@testable import LeptosKitDocumentPicker
import XCTest

final class DocumentReaderTests: XCTestCase {
    func testBoundedSyntheticDocumentAndCleanup() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: directory) }
        let file = directory.appendingPathComponent("fixture.txt")
        try Data([1, 2, 3]).write(to: file)
        let result = try DocumentReader.read(file, limit: 3)
        XCTAssertEqual(result.suffix(3), Data([1, 2, 3]))
        XCTAssertThrowsError(try DocumentReader.read(file, limit: 2)) { error in
            XCTAssertEqual(error as? PluginError, .tooLarge)
        }
        XCTAssertThrowsError(try DocumentReader.read(directory, limit: 3))
        let link = directory.appendingPathComponent("link.txt")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: file)
        XCTAssertThrowsError(try DocumentReader.readCoordinated(link, limit: 3))
        // The reader retains no file handle after success or oversize rejection.
        try FileManager.default.removeItem(at: file)
        try Data().write(to: file)
        XCTAssertFalse(try DocumentReader.read(file, limit: 0).isEmpty)
    }

    func testCancelledReadDoesNotOpenMissingFile() async {
        let task = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            return try DocumentReader.read(URL(fileURLWithPath: "/synthetic-missing-document"), limit: 3)
        }
        do {
            _ = try await task.value
            XCTFail("Expected cancellation")
        } catch {
            XCTAssertTrue(error is CancellationError)
        }
    }
}

@MainActor
final class DocumentSelectionTests: XCTestCase {
    func testCancelledAndLateCallbacksCannotConsumeReplacement() async throws {
        let selection = DocumentSelection()
        let old: UUID = try await withCheckedThrowingContinuation { completed in
            Task { @MainActor in
                do {
                    _ = try await withCheckedThrowingContinuation { pending in
                        let identifier = selection.begin(pending)
                        selection.finish(.failure(PluginError.cancelled), for: identifier)
                        selection.finish(.failure(PluginError.native), for: identifier)
                        completed.resume(returning: identifier)
                    }
                    XCTFail("Expected cancellation")
                } catch {
                    XCTAssertEqual(error as? PluginError, .cancelled)
                }
            }
        }
        let wanted = URL(fileURLWithPath: "/synthetic-owned-document")
        let result: URL = try await withCheckedThrowingContinuation { pending in
            let identifier = selection.begin(pending)
            selection.finish(.failure(PluginError.native), for: old)
            XCTAssertEqual(selection.identifier, identifier)
            selection.finish(.success(wanted), for: identifier)
            selection.finish(.failure(PluginError.native), for: identifier)
        }
        XCTAssertEqual(result, wanted)
        XCTAssertNil(selection.identifier)
    }
}
