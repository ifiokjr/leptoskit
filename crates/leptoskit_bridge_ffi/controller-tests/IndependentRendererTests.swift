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

    @Test func riveUploadOwnsItsPixelsAndSurvivesStyleChanges() throws {
        let host = NSView(frame: NSRect(x: 0, y: 0, width: 128, height: 128))
        let renderer = MacWidgetRenderer(view: host)
        try renderer.render(decode("""
        {"version":1,"root":1,"mutations":[
          {"Create":{"id":1,"kind":"rive","props":{"accessibility_label":"Triangle"}}}
        ]}
        """))
        let widget = try #require(host.subviews.first as? NSImageView)
        var pixels: [UInt8] = [128, 0, 0, 128, 0, 0, 255, 255]
        try pixels.withUnsafeBufferPointer { try renderer.presentRive(1, width: 2, height: 1, pixels: $0) }
        let image = try #require(widget.image)
        pixels = Array(repeating: 0, count: pixels.count)

        let cg = try #require(image.cgImage(forProposedRect: nil, context: nil, hints: nil))
        let data = try #require(cg.dataProvider?.data)
        let actual = Array(UnsafeBufferPointer(start: CFDataGetBytePtr(data), count: CFDataGetLength(data)))
        #expect(actual == [128, 0, 0, 128, 0, 0, 255, 255])
        try renderer.render(decode("""
        {"version":1,"mutations":[{"UpdateProps":{"id":1,"props":{"aria":{"aria_hidden":true}}}}]}
        """))
        #expect(widget.image === image)
        #expect(!widget.isAccessibilityElement())
        renderer.close()
        #expect(widget.image == nil)
    }

    private func decode(_ source: String) throws -> Transaction {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase

        return try decoder.decode(Transaction.self, from: Data(source.utf8))
    }
}
