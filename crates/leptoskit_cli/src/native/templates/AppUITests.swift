import XCTest

@MainActor
final class AppUITests: XCTestCase {
    func testRustAndNativePlugins() {
        let app = XCUIApplication()
        app.launch()
        let hash = app.buttons["Hash with Rust"]
        XCTAssertTrue(hash.waitForExistence(timeout: 15))
        hash.tap()
        XCTAssertTrue(app.staticTexts["SHA-256: 32 bytes"].waitForExistence(timeout: 10))
        app.buttons["Check clipboard"].tap()
        XCTAssertTrue(app.staticTexts["Clipboard roundtrip passed"].waitForExistence(timeout: 10))
        app.buttons["Play haptic feedback"].tap()
        // Simulators and accessibility settings may suppress physical feedback.
        XCTAssertTrue(app.staticTexts["Haptic request accepted"].waitForExistence(timeout: 10))
        app.buttons["Read battery"].tap()
        let battery = app.staticTexts.matching(
            NSPredicate(format: "label MATCHES %@", "Battery: (unknown|([0-9]|[1-9][0-9]|100)%)")
        ).firstMatch
        XCTAssertTrue(battery.waitForExistence(timeout: 10))
        app.buttons["Read device"].tap()
        let device = app.staticTexts.matching(
            NSPredicate(format: "label MATCHES %@", "Device: iOS; memory: [1-9][0-9]* MiB")
        ).firstMatch
        XCTAssertTrue(device.waitForExistence(timeout: 10))
        verifySecureStorage(app)
        app.buttons["Check app files"].tap()
        XCTAssertTrue(app.staticTexts["File roundtrip passed"].waitForExistence(timeout: 10))
        app.terminate()
        app.launch()
        XCTAssertTrue(app.staticTexts["Ready"].waitForExistence(timeout: 10))
        verifySecureStorage(app)
    }

    private func verifySecureStorage(_ app: XCUIApplication) {
        app.buttons["Check secure storage"].tap()
        XCTAssertTrue(app.staticTexts["Secure storage roundtrip passed"].waitForExistence(timeout: 10))
    }
}
