@testable import LeptosKitMacControllers
import AppKit
import Foundation
import Testing

@Suite(.serialized)
@MainActor
struct IndependentRendererTests {
    @Test func buttonPropertiesResetAndNativeTargetSurvivesUpdates() throws {
        let host = NSView()
        let renderer = MacWidgetRenderer(view: host)
        var events: [UInt32] = []
        renderer.onEvent = { _, kind, _ in events.append(kind) }
        try renderer.render(decode("""
        {"version":1,"root":1,"mutations":[
          {"Create":{"id":1,"kind":"Button","props":{"text":"Old","enabled":false,"hidden":true}}}
        ]}
        """))
        let button = try #require(host.subviews.first as? NSButton)
        #expect(!button.isEnabled)
        #expect(button.isHidden)
        try renderer.render(decode("""
        {"version":1,"mutations":[{"UpdateProps":{"id":1,"props":{"text":"New"}}}]}
        """))
        #expect(host.subviews.first === button)
        #expect(button.title == "New")
        #expect(button.isEnabled)
        #expect(!button.isHidden)

        #expect(events.isEmpty)
        button.performClick(nil)
        #expect(events == [0])
    }

    private func decode(_ source: String) throws -> Transaction {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase

        return try decoder.decode(Transaction.self, from: Data(source.utf8))
    }
}
