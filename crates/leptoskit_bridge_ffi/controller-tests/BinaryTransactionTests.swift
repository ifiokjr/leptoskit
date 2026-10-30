@testable import LeptosKitMacControllers
import Foundation
import Testing

/// The binary transaction decodes to exactly what the JSON one does, for the fixture the Rust encoder's
/// tests write: every typed property, properties sent in the JSON form, and every kind of mutation.
struct BinaryTransactionTests {
    private static let fixtures = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .appendingPathComponent("tests/fixtures")

    private static func fixture(_ name: String) throws -> Data {
        try Data(contentsOf: fixtures.appendingPathComponent(name))
    }

    @Test func decodesWhatTheJSONTransactionDecodes() throws {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let expected = try decoder.decode(Transaction.self, from: try Self.fixture("render-transaction.json"))
        let decoded = try Self.fixture("render-transaction.bin").withUnsafeBytes { try Transaction(binary: $0) }

        #expect(decoded.mutations.count == expected.mutations.count)
        #expect(String(describing: decoded) == String(describing: expected))
    }

    @Test func rejectsATruncatedTransaction() throws {
        let binary = try Self.fixture("render-transaction.bin")

        for length in [0, 3, 12, 16, binary.count / 2, binary.count - 1] {
            #expect(throws: (any Error).self) {
                try binary.prefix(length).withUnsafeBytes { try Transaction(binary: $0) }
            }
        }
    }
}
