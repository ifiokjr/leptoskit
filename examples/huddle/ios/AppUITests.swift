import XCTest

@MainActor
final class AppUITests: XCTestCase {
    private static let launchTimeout: TimeInterval = 15

    /// The app opens on the sign-in screen, whose title and email field a
    /// screen reader can reach.
    func testOpensOnSignIn() {
        let app = XCUIApplication()
        app.launch()
        XCTAssertTrue(app.staticTexts["Huddle"].waitForExistence(timeout: Self.launchTimeout))
        XCTAssertTrue(app.textFields.firstMatch.waitForExistence(timeout: 10))
    }
}
