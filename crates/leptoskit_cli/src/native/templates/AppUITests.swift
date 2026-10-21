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
        app.buttons["Copy to clipboard"].tap()
        XCTAssertTrue(app.staticTexts["Copied to clipboard"].waitForExistence(timeout: 10))
        app.terminate()
        app.launch()
        XCTAssertTrue(app.staticTexts["Ready"].waitForExistence(timeout: 10))
    }
}
