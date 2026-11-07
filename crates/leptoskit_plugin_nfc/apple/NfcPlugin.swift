import Foundation
import LeptosKitBridge
#if os(iOS)
import CoreNFC
#endif

public enum NfcAvailability: UInt32 {
    case unsupported = 0
    case hardwareAbsent = 1
    case ready = 2
    case busy = 3
}

public struct NfcRecord: Equatable, Sendable {
    public let kind: UInt32
    public let value: String

    public init(kind: UInt32, value: String) {
        self.kind = kind
        self.value = value
    }
}

@MainActor
protocol NfcReader {
    func availability() -> NfcAvailability
    func read() async throws -> NfcRecord
}

#if os(iOS)
@MainActor
private final class CoreNfcReader: NSObject, NfcReader, NFCNDEFReaderSessionDelegate {
    private var continuation: CheckedContinuation<NfcRecord, Error>?
    private var session: NFCNDEFReaderSession?

    func availability() -> NfcAvailability {
        if continuation != nil { return .busy }
        return NFCNDEFReaderSession.readingAvailable ? .ready : .hardwareAbsent
    }

    func read() async throws -> NfcRecord {
        guard continuation == nil else { throw PluginError.busy }
        guard NFCNDEFReaderSession.readingAvailable else { throw PluginError.unavailable }
        return try await withCheckedThrowingContinuation { continuation in
            self.continuation = continuation
            let session = NFCNDEFReaderSession(delegate: self, queue: nil, invalidateAfterFirstRead: true)
            self.session = session
            session.begin()
        }
    }

    func readerSession(_ session: NFCNDEFReaderSession, didInvalidateWithError error: Error) {
        finish(.failure(map(error)))
    }

    func readerSession(_ session: NFCNDEFReaderSession, didDetectNDEFs messages: [NFCNDEFMessage]) {
        guard let record = messages.first?.records.first else {
            finish(.failure(PluginError.unavailable))
            return
        }
        finish(Result { try Self.decode(record) })
    }

    private func finish(_ result: Result<NfcRecord, Error>) {
        let continuation = self.continuation
        self.continuation = nil
        session = nil
        continuation?.resume(with: result)
    }

    private func map(_ error: Error) -> PluginError {
        let nfc = error as NSError
        if nfc.domain == NFCErrorDomain, nfc.code == NFCReaderError.readerSessionInvalidationErrorUserCanceled.rawValue {
            return .cancelled
        }
        return .native
    }

    private static func decode(_ record: NFCNDEFPayload) throws -> NfcRecord {
        switch record.typeNameFormat {
        case .nfcWellKnown:
            if record.type == Data("U".utf8) {
                let uri = record.wellKnownTypeURIPayload()?.absoluteString ?? ""
                guard !uri.isEmpty, uri.utf8.count <= 4096 else { throw PluginError.invalidArgument }
                return NfcRecord(kind: 1, value: uri)
            }
            if record.type == Data("T".utf8) {
                let text = record.wellKnownTypeTextPayload().0 ?? ""
                guard !text.isEmpty, text.utf8.count <= 4096 else { throw PluginError.invalidArgument }
                return NfcRecord(kind: 0, value: text)
            }
            fallthrough
        default:
            throw PluginError.unavailable
        }
    }
}
#endif

@MainActor
private struct UnavailableNfcReader: NfcReader {
    func availability() -> NfcAvailability { .unsupported }
    func read() async throws -> NfcRecord { throw PluginError.unavailable }
}

/// One-shot foreground NDEF text/URI reads. Writing tags is out of scope.
@MainActor
public final class NfcPlugin: NativePlugin {
    public let name = "nfc"
    public let methods: [UInt32] = [0, 1]

    private let reader: any NfcReader
    private var closed = false

    public init() {
        #if os(iOS)
        reader = CoreNfcReader()
        #else
        reader = UnavailableNfcReader()
        #endif
    }

    init(reader: any NfcReader) {
        self.reader = reader
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        guard methods.contains(method) else { throw PluginError.unavailable }
        guard payload.count == 0 else { throw PluginError.invalidArgument }

        if method == 0 {
            var value = reader.availability().rawValue.littleEndian
            return withUnsafeBytes(of: &value) { Data($0) }
        }

        let record = try await reader.read()
        return try Self.encode(record)
    }

    public func close() {
        closed = true
    }

    private static func encode(_ record: NfcRecord) throws -> Data {
        guard !record.value.isEmpty, record.value.utf8.count <= 4096 else {
            throw PluginError.invalidArgument
        }
        var data = Data()
        var kind = record.kind.littleEndian
        withUnsafeBytes(of: &kind) { data.append(contentsOf: $0) }
        var length = UInt32(record.value.utf8.count).littleEndian
        withUnsafeBytes(of: &length) { data.append(contentsOf: $0) }
        data.append(contentsOf: record.value.utf8)
        return data
    }
}
