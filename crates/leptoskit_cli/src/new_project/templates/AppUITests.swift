import XCTest

@MainActor
final class AppUITests: XCTestCase {
    func testSharedCounter() {
        let app = XCUIApplication()
        app.launch()
        let button = app.buttons["Add one"]
        XCTAssertTrue(button.waitForExistence(timeout: 15))
        button.tap()
        XCTAssertTrue(app.staticTexts["Count: 1"].waitForExistence(timeout: 10))
    }
}
