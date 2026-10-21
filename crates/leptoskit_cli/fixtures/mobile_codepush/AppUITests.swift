import Foundation
import XCTest

@MainActor
final class AppUITests: XCTestCase {
    private let app = XCUIApplication()

    func testDownloadedGuestSurvivesOfflineLaunchAndRejectsStartupTrap() throws {
        continueAfterFailure = false
        app.launch()
        assertText("Counter 1")
        app.buttons["Increment"].tap()
        assertText("Count: 1")
        capture("01-bundled-counter")

        try control("replacement")
        activate()
        assertText("Counter 2")
        app.buttons["Increment"].tap()
        assertText("Count: 10")
        capture("02-downloaded-counter")

        try control("offline")
        app.terminate()
        app.launch()
        assertText("Counter 2")
        app.buttons["Increment"].tap()
        assertText("Count: 10")
        capture("03-offline-relaunch")

        try control("broken")
        activate()
        assertStatus("Update could not start")
        assertText("Counter 2")
        assertText("Count: 10")
        app.buttons["Increment"].tap()
        assertText("Count: 20")
        capture("04-startup-trap-rollback")

        try control("offline")
        app.terminate()
        app.launch()
        assertText("Counter 2")
        app.buttons["Increment"].tap()
        assertText("Count: 10")
        capture("05-persisted-rollback")
    }

    private func activate() {
        let action = app.buttons["leptoskit-update-action"]
        XCTAssertTrue(action.waitForExistence(timeout: 15))
        action.tap()
        let ready = NSPredicate(format: "label == %@", "Activate update")
        expectation(for: ready, evaluatedWith: action)
        waitForExpectations(timeout: 30)
        action.tap()
    }

    private func assertText(_ value: String) {
        XCTAssertTrue(app.staticTexts[value].waitForExistence(timeout: 20), value)
    }

    private func assertStatus(_ prefix: String) {
        let status = app.staticTexts["leptoskit-update-status"]
        let matches = NSPredicate(format: "label BEGINSWITH %@", prefix)
        expectation(for: matches, evaluatedWith: status)
        waitForExpectations(timeout: 30)
    }

    private func capture(_ name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func control(_ operation: String) throws {
        let complete = expectation(description: operation)
        let request = URLRequest(url: URL(string: "__SERVER__/control/\(operation)")!)
        let task = URLSession.shared.dataTask(with: request) { _, response, error in
            XCTAssertNil(error)
            XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
            complete.fulfill()
        }
        task.resume()
        wait(for: [complete], timeout: 15)
    }
}
