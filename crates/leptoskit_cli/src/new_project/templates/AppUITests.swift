import XCTest

@MainActor
final class AppUITests: XCTestCase {
    private static let launchTimeout: TimeInterval = 15

    func testSharedCounter() {
        let app = XCUIApplication()
        app.launch()
        let button = app.buttons["Add one"]
        XCTAssertTrue(button.waitForExistence(timeout: Self.launchTimeout))
        button.tap()
        XCTAssertTrue(app.staticTexts["Count: 1"].waitForExistence(timeout: 10))
    }
}
