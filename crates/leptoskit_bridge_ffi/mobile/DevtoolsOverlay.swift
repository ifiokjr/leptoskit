import CLeptosKitBridge
import Foundation
#if os(macOS)
import AppKit
typealias DevtoolsView = NSView
#else
import UIKit
typealias DevtoolsView = UIView
#endif

/// A node the DevTools hub asked to highlight, as the runtime places it.
struct DevtoolsHighlight: Equatable {
    /// The highlighted node.
    let node: Int
    /// The drawn view the outline goes in: the node's own, or the nearest
    /// ancestor's when the node is a layout-only container with no view.
    let anchor: Int
    /// The node's frame in the coordinates `anchor` lays its children out
    /// in; unused when `anchor` is `node`, whose own bounds are the outline.
    let frame: CGRect
}

/// The runtime's DevTools calls, apart so tests can stand in for a hub. The
/// runtime answers them on any thread.
struct DevtoolsLink {
    var highlight: () -> DevtoolsHighlight?
    var picking: () -> Bool
    /// Picks what a tap at a point in a drawn view's child coordinates lands
    /// on; false when the hub is not picking, so the tap is the app's.
    var pick: (Int, CGPoint) -> Bool

    /// The process's link while it reports to the hub `leptoskit dev` runs,
    /// and nil otherwise: without DevTools, renderers add no overlay at all.
    static func live() -> DevtoolsLink? {
        guard lk_devtools_active() == 1 else { return nil }

        return DevtoolsLink(
            highlight: {
                var frame = lk_devtools_frame()

                guard lk_devtools_highlight(&frame) == 1 else { return nil }

                return DevtoolsHighlight(
                    node: frame.node, anchor: frame.anchor,
                    frame: CGRect(
                        x: CGFloat(frame.anchor_x), y: CGFloat(frame.anchor_y),
                        width: CGFloat(frame.width), height: CGFloat(frame.height)))
            },
            picking: { lk_devtools_picking() == 1 },
            pick: { view, point in lk_devtools_pick(view, Float(point.x), Float(point.y)) == 1 })
    }
}

/// Lies over everything a renderer draws: it outlines the node the hub
/// highlights, as the web agent does, and while the hub picks a node it takes
/// the next tap or click instead of the app. Otherwise no event reaches it.
///
/// The highlight changes on the runtime's reporting thread, so the overlay
/// reads it ten times a second, the rate the runtime reports layout at,
/// rather than having that thread call into the main thread.
@MainActor
final class DevtoolsOverlay: DevtoolsView {
    static let color = (red: 0xe5 / 255.0, green: 0.0, blue: 1.0)
    private let link: DevtoolsLink
    private let widget: (Int) -> DevtoolsView?
    private let node: (DevtoolsView) -> Int?
    /// The outline, 2 points outside the node's frame with a translucent fill.
    let outline = DevtoolsView()
    private var timer: Timer?
    /// Set while finding the view under a tap, so the search looks past this overlay.
    private var searching = false

    /// `widget` finds a rendered node's view and `node` a view's node.
    init(link: DevtoolsLink, widget: @escaping (Int) -> DevtoolsView?, node: @escaping (DevtoolsView) -> Int?) {
        self.link = link
        self.widget = widget
        self.node = node
        super.init(frame: .zero)
        let (red, green, blue) = Self.color
        #if os(macOS)
        autoresizingMask = [.width, .height]
        setAccessibilityElement(false)
        outline.wantsLayer = true
        outline.layer?.borderWidth = 2
        outline.layer?.borderColor = NSColor(srgbRed: red, green: green, blue: blue, alpha: 1).cgColor
        outline.layer?.backgroundColor = NSColor(srgbRed: red, green: green, blue: blue, alpha: 0x22 / 255.0).cgColor
        #else
        autoresizingMask = [.flexibleWidth, .flexibleHeight]
        backgroundColor = .clear
        accessibilityElementsHidden = true
        outline.isUserInteractionEnabled = false
        outline.layer.borderWidth = 2
        outline.layer.borderColor = UIColor(red: red, green: green, blue: blue, alpha: 1).cgColor
        outline.backgroundColor = UIColor(red: red, green: green, blue: blue, alpha: 0x22 / 255.0)
        #endif
        outline.isHidden = true
        addSubview(outline)
    }

    required init?(coder: NSCoder) { nil }

    #if os(macOS)
    override var isFlipped: Bool { true }
    #endif

    /// Covers `host` and follows the hub until closed.
    func attach(to host: DevtoolsView) {
        frame = host.bounds
        host.addSubview(self)
        let timer = Timer(timeInterval: 0.1, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.refresh() }
        }
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
    }

    func close() {
        timer?.invalidate()
        timer = nil
        removeFromSuperview()
    }

    /// Moves the outline over the highlighted node, wherever its views have
    /// scrolled to, or hides it.
    func refresh() {
        guard let highlight = link.highlight(), let anchor = widget(highlight.anchor),
            let rect = place(highlight, in: anchor)
        else {
            outline.isHidden = true
            return
        }

        outline.frame = rect.insetBy(dx: -2, dy: -2)
        outline.isHidden = false
    }

    private func place(_ highlight: DevtoolsHighlight, in anchor: DevtoolsView) -> CGRect? {
        guard anchor.window != nil, anchor.window === window else { return nil }

        if highlight.anchor == highlight.node { return convert(anchor.bounds, from: anchor) }

        return convert(highlight.frame, from: Self.content(of: anchor))
    }

    /// The view a drawn widget lays its children out in.
    private static func content(of view: DevtoolsView) -> DevtoolsView {
        #if os(macOS)
        return (view as? NSScrollView)?.documentView ?? view
        #else
        return view
        #endif
    }

    /// Picks the node under `point`, in this overlay's coordinates: the
    /// deepest drawn view there tells the runtime where, and the runtime finds
    /// the node, layout-only containers included. Returns whether it did.
    @discardableResult
    func pick(at point: CGPoint) -> Bool {
        guard let host = superview else { return false }
        searching = true
        #if os(macOS)
        // AppKit hit-tests a view with a point in its superview's coordinates.
        let hit = host.hitTest(host.superview.map { $0.convert(point, from: self) } ?? host.convert(point, from: self))
        #else
        let hit = host.hitTest(host.convert(point, from: self), with: nil)
        #endif
        searching = false
        var candidate = hit

        while let view = candidate, view !== host {
            if let id = node(view) {
                let content = Self.content(of: view)
                let found = link.pick(id, content.convert(point, from: self))
                refresh()
                return found
            }

            candidate = view.superview
        }

        return false
    }

    #if os(macOS)
    override func hitTest(_ point: NSPoint) -> NSView? {
        guard !searching, link.picking(), frame.contains(point) else { return nil }

        return self
    }

    /// The developer is usually in the browser when they pick, so the first
    /// click on the window picks rather than only activating it.
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func mouseDown(with event: NSEvent) {
        pick(at: convert(event.locationInWindow, from: nil))
    }
    #else
    override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
        !searching && link.picking() && super.point(inside: point, with: event)
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard let touch = touches.first else { return }
        pick(at: touch.location(in: self))
    }
    #endif
}
