import Foundation
import LeptosKitBridge
@testable import LeptosKitScreenOrientation
import XCTest

final class ScreenOrientationTests: XCTestCase {
    func testPoliciesRejectUnknownTruncatedAndTrailingValues() throws {
        let policies = UInt32(0)...OrientationPolicy.any.rawValue

        for raw in policies {
            let bytes = withUnsafeBytes(of: raw.littleEndian) { Data($0) }
            XCTAssertEqual(try bytes.withUnsafeBytes { try OrientationPolicy.decode($0).rawValue }, raw)
        }
        for bytes in [Data(), Data([0, 0, 0]), Data([0, 0, 0, 0, 0]), Data([7, 0, 0, 0])] {
            XCTAssertThrowsError(try bytes.withUnsafeBytes { try OrientationPolicy.decode($0) }) { error in
                XCTAssertEqual(error as? PluginError, .invalidArgument)
            }
        }
    }
}
