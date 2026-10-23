@testable import LeptosKitMacControllers
import AppKit
import Foundation
import LeptosKitBridge
import Testing

@Suite(.serialized)
@MainActor
struct MacRendererTests {
    private struct Event { let node: Int; let kind: UInt32; let text: String }

    /// Real AppKit controls retain IDs, text, layout and event routing after mutation.
    @Test func mutationOrderingAndNativeEvents() throws {
        let view = NSView()
        let renderer = MacWidgetRenderer(view: view)
        var events: [Event] = []
        renderer.onEvent = { events.append(Event(node: $0, kind: $1, text: $2)) }
        try renderer.render(decode("""
        {"version":1,"root":1,"mutations":[
          {"Create":{"id":1,"kind":"View","props":{}}},
          {"Create":{"id":2,"kind":"TextInput","props":{"value":"before"}}},
          {"InsertChild":{"parent":1,"child":2}},
          {"UpdateLayout":{"id":2,"x":3,"y":4,"width":80,"height":20}},
          {"UpdateProps":{"id":2,"props":{"value":"after","accessibility_label":"Input"}}}
        ]}
        """))
        let root = try #require(view.subviews.first)
        let field = try #require(root.subviews.first as? NSTextField)
        #expect(field.stringValue == "after")
        #expect(field.frame == CGRect(x: 3, y: 4, width: 80, height: 20))
        #expect(field.accessibilityIdentifier() == "Input")
        #expect(events.isEmpty)
        renderer.controlTextDidChange(Notification(name: NSText.didChangeNotification, object: field))
        #expect(events.count == 1)

        #expect(events.first?.node == 2)
        #expect(events.first?.kind == 1)
        #expect(events.first?.text == "after")
    }

    /// Delete removes native ownership; invalid transactions cannot mutate a valid tree.
    @Test func removalAndUnsupportedVersion() throws {
        let view = NSView()
        let renderer = MacWidgetRenderer(view: view)
        try renderer.render(decode("""
        {"version":1,"root":1,"mutations":[
          {"Create":{"id":1,"kind":"View","props":{}}},
          {"Create":{"id":2,"kind":"Text","props":{"text":"child"}}},
          {"InsertChild":{"parent":1,"child":2}},
          {"Delete":{"id":2}}
        ]}
        """))
        #expect(view.subviews.first?.subviews.isEmpty == true)
        #expect(throws: PluginError.self) {
            try renderer.render(decode("{\"version\":2,\"mutations\":[]}"))
        }
        #expect(view.subviews.count == 1)
    }

    /// Scroll documents preserve insertion order and expand to include laid-out children.
    @Test func scrollDocumentSizingAndChildOrder() throws {
        let view = NSView()
        let renderer = MacWidgetRenderer(view: view)
        try renderer.render(decode("""
        {"version":1,"root":1,"mutations":[
          {"Create":{"id":1,"kind":"ScrollView","props":{}}},
          {"Create":{"id":2,"kind":"View","props":{}}},
          {"Create":{"id":3,"kind":"View","props":{}}},
          {"InsertChild":{"parent":1,"child":2}},
          {"InsertChild":{"parent":1,"child":3,"index":0}},
          {"UpdateLayout":{"id":2,"x":10,"y":20,"width":300,"height":400}}
        ]}
        """))
        let scroll = try #require(view.subviews.first as? NSScrollView)
        let document = try #require(scroll.documentView)
        #expect(document.subviews.map { $0.identifier?.rawValue } == ["3", "2"])
        #expect(document.frame.width >= 310)
        #expect(document.frame.height >= 420)
    }

    private func decode(_ source: String) throws -> Transaction {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase

        return try decoder.decode(Transaction.self, from: Data(source.utf8))
    }
}
