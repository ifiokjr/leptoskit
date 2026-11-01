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

    /// Buttons and pressables report holds with the press-in and press-out
    /// event kinds. The button's `mouseDown` bracket around AppKit's tracking
    /// loop is exercised with real mouse events by the Rust AppKit backend's
    /// pressable tests, since a nested tracking loop cannot run inside this
    /// async test runner.
    @Test func buttonsAndPressablesReportHoldsAsPressPhases() throws {
        let view = NSView()
        let renderer = MacWidgetRenderer(view: view)
        var events: [Event] = []
        renderer.onEvent = { events.append(Event(node: $0, kind: $1, text: $2)) }
        try renderer.render(decode("""
        {"version":1,"root":1,"mutations":[
          {"Create":{"id":1,"kind":"View","props":{}}},
          {"Create":{"id":2,"kind":"Button","props":{"text":"Hold","background":4278190335}}},
          {"Create":{"id":3,"kind":"Pressable","props":{"accessibility_label":"Card"}}},
          {"InsertChild":{"parent":1,"child":2}},
          {"InsertChild":{"parent":1,"child":3}}
        ]}
        """))
        let children = try #require(view.subviews.first?.subviews)
        let button = try #require(children.first as? PressButton)
        button.onHold?(true)

        button.onHold?(false)
        let card = try #require(children.last as? PressView)
        let mouse = { (type: NSEvent.EventType) in
            try #require(NSEvent.mouseEvent(
                with: type, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0,
                context: nil, eventNumber: 0, clickCount: 1, pressure: 1
            ))
        }
        card.mouseDown(with: try mouse(.leftMouseDown))
        card.mouseUp(with: try mouse(.leftMouseUp))
        #expect(events.map { [$0.node, Int($0.kind)] } == [[2, 8], [2, 9], [3, 8], [3, 9]])
        #expect(events.allSatisfy { $0.text.isEmpty })
    }

    // swiftlint:disable function_body_length
    // The overlay's phases share one rendered tree; decoding it again for each
    // phase would test the fixture more than the overlay.
    /// The DevTools overlay outlines the hub's highlight over the view the
    /// platform draws for it, follows scrolling, and while the hub picks takes
    /// the next click for the hub instead of the app.
    @Test func devtoolsOverlayOutlinesAndPicks() throws {
        let window = NSWindow(
            contentRect: CGRect(x: 0, y: 0, width: 300, height: 300), styleMask: [.titled],
            backing: .buffered, defer: true)
        let view = FlippedView(frame: CGRect(x: 0, y: 0, width: 300, height: 300))
        window.contentView = view
        var highlight: DevtoolsHighlight?
        var picking = false
        var picks: [(node: Int, point: CGPoint)] = []
        let link = DevtoolsLink(
            highlight: { highlight }, picking: { picking },
            pick: { node, point in
                picks.append((node, point))
                return picking
            })
        let renderer = MacWidgetRenderer(view: view, devtools: link)

        try renderer.render(decode("""
        {"version":1,"root":1,"mutations":[
          {"Create":{"id":1,"kind":"View","props":{}}},
          {"Create":{"id":2,"kind":"ScrollView","props":{}}},
          {"Create":{"id":3,"kind":"View","props":{}}},
          {"Create":{"id":4,"kind":"Text","props":{"text":"Title"}}},
          {"InsertChild":{"parent":1,"child":2}},
          {"InsertChild":{"parent":2,"child":3}},
          {"InsertChild":{"parent":3,"child":4}},
          {"UpdateLayout":{"id":1,"x":0,"y":0,"width":300,"height":300}},
          {"UpdateLayout":{"id":2,"x":10,"y":20,"width":200,"height":100}},
          {"UpdateLayout":{"id":3,"x":0,"y":0,"width":200,"height":400}},
          {"UpdateLayout":{"id":4,"x":5,"y":6,"width":50,"height":10}}
        ]}
        """))
        let overlay = try #require(renderer.devtools)
        #expect(view.subviews.last === overlay, "the overlay stays above the app")

        overlay.refresh()
        #expect(overlay.outline.isHidden)

        // A drawn node is outlined around its own view.
        highlight = DevtoolsHighlight(node: 4, anchor: 4, frame: .zero)
        overlay.refresh()
        #expect(!overlay.outline.isHidden)
        #expect(overlay.outline.frame == CGRect(x: 13, y: 24, width: 54, height: 14))

        // A layout-only container is outlined in its drawn ancestor's content,
        // which moves as the scroll view scrolls.
        highlight = DevtoolsHighlight(node: 9, anchor: 3, frame: CGRect(x: 5, y: 30, width: 20, height: 10))
        overlay.refresh()
        #expect(overlay.outline.frame == CGRect(x: 13, y: 48, width: 24, height: 14))
        let scroll = try #require(view.subviews.first?.subviews.first as? NSScrollView)
        scroll.contentView.scroll(to: CGPoint(x: 0, y: 10))
        overlay.refresh()
        #expect(overlay.outline.frame == CGRect(x: 13, y: 38, width: 24, height: 14))

        highlight = nil
        overlay.refresh()
        #expect(overlay.outline.isHidden)

        // Clicks are the app's until the hub picks.
        let point = CGPoint(x: 20, y: 30)
        #expect(overlay.hitTest(point) == nil)
        picking = true
        #expect(overlay.hitTest(point) === overlay)
        #expect(overlay.acceptsFirstMouse(for: nil))
        #expect(overlay.pick(at: point))
        // The deepest drawn view under the click, with the point in its
        // content's coordinates, scrolled by 10.
        #expect(picks.count == 1)
        #expect(picks.first?.node == 3)

        #expect(picks.first?.point == CGPoint(x: 10, y: 20))

        renderer.close()
        #expect(overlay.superview == nil)
    }
    // swiftlint:enable function_body_length

    private func decode(_ source: String) throws -> Transaction {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase

        return try decoder.decode(Transaction.self, from: Data(source.utf8))
    }
}
