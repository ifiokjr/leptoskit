import CoreLocation
import Foundation
import LeptosKitBridge
@testable import LeptosKitLocation
import Testing

@Test
func queuedLocationsCannotCrossPermissionOrPrecisionRevocation() {
    #expect(permitsLocationDelivery(startedPrecise: true, authorized: true, precise: true))
    #expect(permitsLocationDelivery(startedPrecise: false, authorized: true, precise: false))
    #expect(permitsLocationDelivery(startedPrecise: false, authorized: true, precise: true))
    #expect(!permitsLocationDelivery(startedPrecise: true, authorized: true, precise: false))
    #expect(!permitsLocationDelivery(startedPrecise: false, authorized: false, precise: false))
    #expect(!permitsLocationDelivery(startedPrecise: true, authorized: false, precise: true))
}

@Test
func syntheticCoordinatesPreserveUnknownFieldsWithoutAccessingLocation() throws {
    let location = CLLocation(
        coordinate: CLLocationCoordinate2D(latitude: 37, longitude: -122),
        altitude: 0,
        horizontalAccuracy: 5,
        verticalAccuracy: -1,
        course: -1,
        speed: -1,
        timestamp: Date(timeIntervalSince1970: 1_800_000_000)
    )
    let result = try encodeLocation(location)
    #expect(result.count == 84)
    result.withUnsafeBytes { bytes in
        #expect(UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: 4, as: UInt32.self)) == 0)
        #expect(UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: 8, as: UInt32.self)) == 0)
        let bits = UInt64(littleEndian: bytes.loadUnaligned(fromByteOffset: 28, as: UInt64.self))
        let latitude = Double(bitPattern: bits)
        #expect(latitude == 37)
    }
    let invalid = CLLocation(
        coordinate: CLLocationCoordinate2D(latitude: 91, longitude: -122),
        altitude: 0, horizontalAccuracy: 5, verticalAccuracy: -1, timestamp: Date()
    )
    #expect(throws: PluginError.native) { try encodeLocation(invalid) }
}

@Test
func locationRequestBoundsRejectUnboundedAndNonfiniteRequests() throws {
    func request(timeout: UInt32, after: Double? = nil) -> Data {
        var bytes = Data()
        for var field in [UInt32(1), timeout, 0].map(\.littleEndian) {
            withUnsafeBytes(of: &field) { bytes.append(contentsOf: $0) }
        }
        if let after {
            var bits = after.bitPattern.littleEndian
            withUnsafeBytes(of: &bits) { bytes.append(contentsOf: $0) }
        }
        return bytes
    }
    let valid = try request(timeout: 1000).withUnsafeBytes { try LocationOptions(bytes: $0, hasPrevious: false) }
    #expect(valid.highAccuracy && valid.timeout == 1000 && valid.maximumAge == 0)
    for timeout in [UInt32(0), 120_001] {
        #expect(throws: PluginError.invalidArgument) {
            try request(timeout: timeout).withUnsafeBytes { try LocationOptions(bytes: $0, hasPrevious: false) }
        }
    }
    #expect(throws: PluginError.invalidArgument) {
        try request(timeout: 1000, after: .nan).withUnsafeBytes { try LocationOptions(bytes: $0, hasPrevious: true) }
    }
}
