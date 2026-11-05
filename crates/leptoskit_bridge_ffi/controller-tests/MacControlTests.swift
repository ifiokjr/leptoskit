@testable import LeptosKitMacControllers
import AppKit
import Foundation
import Testing

@Suite(.serialized)
@MainActor
struct MacControlTests {
    private func transaction(_ changes: String) throws -> Transaction {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let json = "{\"version\":1,\"root\":1,\"mutations\":[\(changes)]}"
        return try decoder.decode(Transaction.self, from: Data(json.utf8))
    }

    @Test func selectionProposalsRestoreAndAcceptSynchronousGuestState() throws {
        let host = NSView()
        let renderer = MacWidgetRenderer(view: host, devtools: nil)
        try renderer.render(transaction("""
        {"Create":{"id":1,"kind":"view","props":{}}},
        {"Create":{"id":2,"kind":"switch","props":{}}},
        {"Create":{"id":3,"kind":"checkbox","props":{}}},
        {"Create":{"id":4,"kind":"radio","props":{"checked":true}}},
        {"InsertChild":{"parent":1,"child":2}},
        {"InsertChild":{"parent":1,"child":3}},
        {"InsertChild":{"parent":1,"child":4}}
        """))
        let toggle = try #require(renderer.widgets[2] as? NSSwitch)
        let check = try #require(renderer.widgets[3] as? NSButton)
        let radio = try #require(renderer.widgets[4] as? NSButton)
        var events: [String] = []
        renderer.onEvent = { id, kind, payload in events.append("\(id):\(kind):\(payload)") }
        toggle.performClick(nil)
        #expect(events.last == "2:6:{\"checked\":true}")
        #expect(toggle.state == .off)
        check.performClick(nil)
        #expect(events.last == "3:6:{\"checked\":true}")
        #expect(check.state == .off)
        radio.performClick(nil)
        #expect(events.last == "4:6:{\"checked\":true}")
        #expect(radio.state == .on)
        var error: Error?
        renderer.onEvent = { id, _, payload in
            do {
                try renderer.render(transaction("{\"UpdateProps\":{\"id\":\(id),\"props\":\(payload)}}"))
            } catch let caught { error = caught }
        }
        toggle.performClick(nil)
        #expect(toggle.state == .on)
        toggle.performClick(nil)
        #expect(toggle.state == .off)
        check.performClick(nil)
        #expect(check.state == .on)
        check.performClick(nil)
        #expect(check.state == .off)
        #expect(error == nil)
        renderer.close()
        toggle.performClick(nil)
        #expect(error == nil)
    }

    @Test func sliderAndProgressApplyGuestValues() throws {
        let host = NSView()
        let renderer = MacWidgetRenderer(view: host, devtools: nil)
        try renderer.render(transaction("""
        {"Create":{"id":1,"kind":"view","props":{}}},
        {"Create":{"id":2,"kind":"slider","props":{"slider":{"min":10,"max":20,"value":12,"step":2}}}},
        {"Create":{"id":3,"kind":"linear_progress_indicator","props":{"progress":{"determinate":0.75}}}},
        {"Create":{"id":4,"kind":"activity_indicator","props":{"running":false}}}
        """))
        let slider = try #require(renderer.widgets[2] as? NSSlider)
        let progress = try #require(renderer.widgets[3] as? NSProgressIndicator)
        let activity = try #require(renderer.widgets[4] as? NSProgressIndicator)
        #expect(slider.doubleValue == 12 && slider.minValue == 10 && slider.maxValue == 20)
        #expect(progress.doubleValue == 0.75 && !progress.isIndeterminate)
        #expect(activity.style == .spinning)
        var payloads: [String] = []
        renderer.onEvent = { _, kind, text in
            #expect(kind == 7)
            payloads.append(text)
        }
        slider.doubleValue = 15.7
        slider.sendAction(slider.action, to: slider.target)
        #expect(payloads == ["{\"value\":16.0}"])
        #expect(slider.doubleValue == 12)
        #expect(slider.accessibilityPerformIncrement())
        #expect(payloads.last == "{\"value\":14.0}")
        #expect(slider.accessibilityPerformDecrement())
        #expect(payloads.last == "{\"value\":10.0}")
        try renderer.render(transaction("""
        {"UpdateProps":{"id":2,"props":{"enabled":false}}},
        {"UpdateProps":{"id":3,"props":{"progress":"indeterminate"}}}
        """))
        slider.sendAction(slider.action, to: slider.target)
        #expect(payloads.count == 3 && !slider.isEnabled)
        #expect(progress.isIndeterminate)
    }
}
