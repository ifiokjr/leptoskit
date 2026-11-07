@testable import LeptosKitMacControllers
import AppKit
import Foundation
import Testing

/// Transforms and animations run on the widgets' layers, starting from what each showed before.
@Suite(.serialized)
@MainActor
struct MacMotionTests {
    private static let linear = #"{"timing":{"duration_ms":200,"easing":"linear"}}"#

    /// A transform turns and moves the widget about its center, with percentages of its own size.
    @Test func transformsTurnAboutTheCenterAndFollowTheSize() throws {
        let view = NSView()
        let renderer = MacWidgetRenderer(view: view)
        try renderer.render(decode("""
        {"version":1,"root":1,"mutations":[
          {"Create":{"id":1,"kind":"view","props":{"transform":{"translate_x":{"percent":50},"rotate":90}}}},
          {"UpdateLayout":{"id":1,"x":0,"y":0,"width":100,"height":50}}
        ]}
        """))
        let layer = try #require(view.subviews.first?.layer)
        // The center, (50, 25) from the layer's origin, moves half the width along x and stays there.
        let center = apply(layer.transform, CGPoint(x: 50, y: 25))
        #expect(abs(center.x - 100) < 0.001 && abs(center.y - 25) < 0.001)
        // A corner turns a quarter about the center.
        let corner = apply(layer.transform, .zero)
        #expect(abs(corner.x - 125) < 0.001 && abs(corner.y + 25) < 0.001)

        try renderer.render(decode("""
        {"version":1,"mutations":[{"UpdateLayout":{"id":1,"x":0,"y":0,"width":200,"height":50}}]}
        """))
        let moved = apply(layer.transform, CGPoint(x: 100, y: 25))
        #expect(abs(moved.x - 200) < 0.001)

        try renderer.render(decode(#"{"version":1,"mutations":[{"UpdateProps":{"id":1,"props":{}}}]}"#))
        #expect(CATransform3DIsIdentity(layer.transform))
    }

    /// A transition animates a change of a property it covers from the old value, and nothing else.
    @Test func transitionsStartFromTheOldValue() throws {
        let view = NSView()
        let renderer = MacWidgetRenderer(view: view)
        let animation = #"{"transition":{"properties":1,"curve":\#(Self.linear)}}"#
        try renderer.render(decode("""
        {"version":1,"root":1,"mutations":[
          {"Create":{"id":1,"kind":"view","props":{"animation":\(animation)}}}
        ]}
        """))
        let layer = try #require(view.subviews.first?.layer)
        #expect(layer.animation(forKey: "lk.transition.opacity") == nil)

        try renderer.render(decode("""
        {"version":1,"mutations":[
          {"UpdateProps":{"id":1,"props":{"opacity":0.5,"background":4278190335,"animation":\(animation)}}}
        ]}
        """))
        let opacity = try #require(layer.animation(forKey: "lk.transition.opacity") as? CABasicAnimation)
        #expect(opacity.fromValue as? Float == 1)
        #expect(opacity.toValue as? Float == 0.5)
        #expect(abs(opacity.duration - 0.2) < 0.0001)
        #expect(layer.animation(forKey: "lk.transition.backgroundColor") == nil)
    }

    /// Keyframes loop on the layer until the widget stops asking for them.
    @Test func keyframesLoopUntilRemoved() throws {
        let view = NSView()
        let renderer = MacWidgetRenderer(view: view)
        let spin = """
        {"keyframes":{"frames":[{"offset":0,"transform":{}},{"offset":1,"transform":{"rotate":360}}],
          "duration_ms":1000,"iterations":"infinite","easing":"linear"}}
        """
        try renderer.render(decode("""
        {"version":1,"root":1,"mutations":[
          {"Create":{"id":1,"kind":"view","props":{"animation":\(spin)}}},
          {"UpdateLayout":{"id":1,"x":0,"y":0,"width":20,"height":20}}
        ]}
        """))
        let layer = try #require(view.subviews.first?.layer)
        let animation = try #require(layer.animation(forKey: "lk.keyframes.transform") as? CAKeyframeAnimation)
        #expect(animation.repeatCount == .infinity)
        #expect(layer.animation(forKey: "lk.keyframes.opacity") == nil)
        // Halfway through, the widget has turned half a turn about its center.
        let values = try #require(animation.values as? [NSValue])
        let half = apply(values[values.count / 2].caTransform3DValue, .zero)
        #expect(abs(half.x - 20) < 0.001 && abs(half.y - 20) < 0.001)

        try renderer.render(decode(#"{"version":1,"mutations":[{"UpdateProps":{"id":1,"props":{}}}]}"#))
        #expect(layer.animation(forKey: "lk.keyframes.transform") == nil)
    }

    /// A widget added with an entering animation starts from its entering look.
    @Test func enteringStartsFromTheEnteringLook() throws {
        let view = NSView()
        let renderer = MacWidgetRenderer(view: view)
        try renderer.render(decode("""
        {"version":1,"root":1,"mutations":[
          {"Create":{"id":1,"kind":"view","props":{"opacity":0.8,
            "animation":{"entering":{"visual":{"opacity":0},"curve":\(Self.linear),"delay_ms":50}}}}}
        ]}
        """))
        let layer = try #require(view.subviews.first?.layer)
        let opacity = try #require(layer.animation(forKey: "lk.presence.opacity") as? CABasicAnimation)
        #expect(opacity.fromValue as? Float == 0)
        #expect(opacity.toValue as? Float == 0.8)
        #expect(opacity.fillMode == .backwards)
        #expect(layer.animation(forKey: "lk.presence.transform") == nil)
    }

    /// A removed widget with an exiting animation stays drawn after its former siblings until the
    /// animation ends; one put back in the same transaction does not animate out.
    @Test func exitingWidgetsStayUntilTheirAnimationEnds() throws {
        let window = NSWindow(contentRect: CGRect(x: 0, y: 0, width: 200, height: 200), styleMask: [],
                              backing: .buffered, defer: true)
        let view = try #require(window.contentView)
        let renderer = MacWidgetRenderer(view: view, devtools: nil)
        let exiting = #"{"exiting":{"visual":{"opacity":0},"curve":\#(Self.linear)}}"#
        try renderer.render(decode("""
        {"version":1,"root":1,"mutations":[
          {"Create":{"id":1,"kind":"view","props":{}}},
          {"Create":{"id":2,"kind":"view","props":{"animation":\(exiting)}}},
          {"Create":{"id":3,"kind":"view","props":{}}},
          {"InsertChild":{"parent":1,"child":2}},
          {"InsertChild":{"parent":1,"child":3}}
        ]}
        """))
        let root = try #require(view.subviews.first)
        let leaving = try #require(renderer.widgets[2])

        try renderer.render(decode("""
        {"version":1,"mutations":[
          {"RemoveChild":{"parent":1,"child":2}},
          {"InsertChild":{"parent":1,"child":2,"index":1}}
        ]}
        """))
        #expect(root.subviews.map { $0.identifier?.rawValue } == ["3", "2"])
        #expect(leaving.layer?.animation(forKey: "lk.exit") == nil)

        try renderer.render(decode("""
        {"version":1,"mutations":[
          {"RemoveChild":{"parent":1,"child":2}},
          {"Delete":{"id":2}},
          {"Create":{"id":4,"kind":"view","props":{}}},
          {"InsertChild":{"parent":1,"child":4,"index":1}}
        ]}
        """))
        #expect(renderer.widgets[2] == nil)
        #expect(root.subviews.map { $0.identifier?.rawValue } == ["3", "4", "2"])
        #expect(leaving.layer?.animation(forKey: "lk.exit") != nil)
    }

    private func apply(_ transform: CATransform3D, _ point: CGPoint) -> CGPoint {
        CGPoint(
            x: point.x * transform.m11 + point.y * transform.m21 + transform.m41,
            y: point.x * transform.m12 + point.y * transform.m22 + transform.m42)
    }

    private func decode(_ source: String) throws -> Transaction {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase

        return try decoder.decode(Transaction.self, from: Data(source.utf8))
    }
}
