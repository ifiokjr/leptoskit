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

    @Test func decodesControlPropertiesFromBinaryExtras() throws {
        let json = Data(#"""
        {"checked":true,"slider":{"min":10,"max":20,"value":12,"step":2},
         "progress":{"determinate":0.75},"running":false,"focused":true,"aria":{"value_text":"Twelve"}}
        """#.utf8)
        var binary = Data("LKB1".utf8)
        func append<T: FixedWidthInteger>(_ value: T) {
            var little = value.littleEndian
            withUnsafeBytes(of: &little) { binary.append(contentsOf: $0) }
        }
        append(Int64(-1))
        append(UInt32(1))
        append(UInt8(2)) // UpdateProps
        append(UInt32(7))
        append(UInt32(1 << 31)) // JSON extras
        append(UInt32(json.count))
        binary.append(json)
        let transaction = try binary.withUnsafeBytes { try Transaction(binary: $0) }
        let props = try #require(transaction.mutations.first?.updateProps?.props)
        #expect(props.checked == true)
        #expect(props.slider?.value == 12)
        #expect(props.slider?.proposal(0.57) == 16)
        #expect(props.running == false)
        #expect(props.focused == true)
        #expect(props.aria?.valueText == "Twelve")
        guard case .determinate(let value) = props.progress else {
            Issue.record("Missing determinate progress")
            return
        }
        #expect(value == 0.75)
    }

    @Test func sliderKeepsFractionalUpperEndpointAndUnalignedMaximum() {
        let aligned = WidgetSlider(min: 0, max: 0.3, value: 0, step: 0.1)
        #expect(aligned.proposal(1) == 0.3)
        let unaligned = WidgetSlider(min: 0, max: 1, value: 0, step: 0.3)
        #expect(abs(unaligned.proposal(1) - 0.9) < 0.000001)
        let large = WidgetSlider(min: 1e100, max: 2e100, value: 1.5e100, step: nil)
        #expect(large.coordinate == 0.5)
        #expect(large.proposal(0.5).isFinite)
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
