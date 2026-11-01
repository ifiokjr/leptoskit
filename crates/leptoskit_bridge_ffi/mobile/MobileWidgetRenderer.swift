import LeptosKitBridge
import UIKit

/// A tappable container reporting holds from its touch's start to its end or
/// cancellation; a recognized tap cancels the touch.
final class PressView: UIView {
    var onHold: ((Bool) -> Void)?

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        onHold?(true)
        super.touchesBegan(touches, with: event)
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        super.touchesEnded(touches, with: event)
        onHold?(false)
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        super.touchesCancelled(touches, with: event)
        onHold?(false)
    }
}

/// Owns the UIKit tree, mutation suppression and readiness after a visible UI turn.
@MainActor
final class MobileWidgetRenderer: NSObject, UITextFieldDelegate, UIScrollViewDelegate, NativeRiveSink {
    private static let pressIn: UInt32 = 8
    private static let pressOut: UInt32 = 9
    private let view: UIView
    private var widgets: [Int: UIView] = [:]
    /// Requested corner radii, fitted to each frame as layout changes it.
    private var radii: [Int: CGFloat] = [:]
    /// How many children the renderer has inserted into each widget. Reading
    /// `UIView.subviews` copies the whole array, so counting through it made
    /// inserting n children cost n copies of a growing list: over 200 ms for
    /// 1,000 cards.
    private var childCounts: [Int: Int] = [:]
    /// Deleted containers and labels, kept for the next screen to reuse, as
    /// React Native's Fabric does: creating a `UILabel` costs far more than
    /// restyling one, and `WidgetStyle.apply` sets every property it owns.
    private var recycled = RecycledViews()
    private let scrolls = ScrollTracker()
    private var applying = false
    private var readinessScheduled = false
    private var readinessDelivered = false
    private var open = true
    var onReady: (() -> Void)?
    var onEvent: ((Int, UInt32, String) -> Void)?
    var onRiveViewportChange: (() -> Void)?
    /// The root widget's opaque background. Like a browser painting the whole
    /// canvas with the root element's background, the host paints it behind
    /// the safe areas; nil leaves the host's own background.
    private(set) var canvas: UIColor?
    var onCanvasChange: (() -> Void)?
    /// A transaction names its root only when the root changes.
    private var root: Int?
    /// The DevTools highlight and picking, in apps `leptoskit dev` reports to a hub.
    private(set) var devtools: DevtoolsOverlay?
    init(view: UIView, devtools link: DevtoolsLink? = DevtoolsLink.live()) {
        self.view = view
        super.init()
        guard let link else { return }
        let overlay = DevtoolsOverlay(
            link: link, widget: { [weak self] in self?.widgets[$0] },
            node: { [weak self] view in self?.widgets[view.tag] === view ? view.tag : nil })
        overlay.attach(to: view)
        devtools = overlay
    }
    func close() {
        open = false
        devtools?.close()
        devtools = nil
        onRiveViewportChange = nil
        for widget in widgets.values { (widget as? UIImageView)?.image = nil }
        widgets.removeAll()
        radii.removeAll()
        childCounts.removeAll()
        scrolls.removeAll()
        recycled = RecycledViews()
    }

    /// The Rive views on screen now, for the guest thread to advance.
    func visibleRiveNodes() -> [Int] {
        widgets.compactMap { node, widget in widget is NativeRiveImageView && riveVisible(node) ? node : nil }
    }

    func riveVisible(_ node: Int) -> Bool {
        guard let widget = widgets[node], let window = widget.window, !widget.isHidden,
            widget.alpha > 0 else { return false }
        var visible = widget.convert(widget.bounds, to: window)
        var ancestor = widget.superview
        while let parent = ancestor {
            if parent.isHidden || parent.alpha == 0 { return false }
            if parent.clipsToBounds { visible = visible.intersection(parent.convert(parent.bounds, to: window)) }
            ancestor = parent.superview
        }
        return !visible.isEmpty && visible.intersects(window.bounds)
    }

    func presentRive(_ node: Int, width: Int, height: Int, pixels: UnsafeBufferPointer<UInt8>) throws {
        guard let image = widgets[node] as? UIImageView else { throw PluginError.native }
        image.image = UIImage(cgImage: try nativeRiveImage(width: width, height: height, pixels: pixels))
    }

    func scrollViewDidScroll(_ scrollView: UIScrollView) {
        onRiveViewportChange?()
        guard widgets[scrollView.tag] === scrollView else { return }

        if applying { scrolls.movedWhileApplying(scrollView.tag) } else { reportScroll(scrollView.tag) }
    }

    private func reportScroll(_ id: Int) {
        guard let scroll = widgets[id] as? UIScrollView else { return }
        event(id, ScrollTracker.event, ScrollTracker.payload(scroll.contentOffset))
    }

    func render(_ transaction: Transaction) throws {
        guard transaction.version == 1 else { throw PluginError.unavailable }
        applying = true
        defer {
            applying = false
            scrolls.transactionApplied { [weak self] id in self?.reportScroll(id) }
        }

        for change in transaction.mutations {
            try apply(change)
        }

        if let root = transaction.root, let widget = widgets[root], widget.superview !== view {
            if let devtools {
                view.insertSubview(widget, belowSubview: devtools)
            } else {
                view.addSubview(widget)
            }
        }

        scheduleReadiness(transaction.root)

        updateScrollSizes()
        scrollToTargets()
        updateCanvas(transaction.root)
    }

    private func updateCanvas(_ named: Int?) {
        if let named { root = named }
        let color = root.flatMap { widgets[$0]?.backgroundColor }.flatMap { $0.cgColor.alpha == 1 ? $0 : nil }
        guard color != canvas else { return }
        canvas = color
        onCanvasChange?()
    }

    private func scheduleReadiness(_ root: Int?) {
        guard let root, let widget = widgets[root], widget.superview === view,

            widget.bounds.width > 0, widget.bounds.height > 0, !readinessScheduled
        else { return }
        readinessScheduled = true
        Task { [weak self, weak widget] in self?.deliverReadiness(widget) }
    }

    private func deliverReadiness(_ widget: UIView?) {
        guard let widget, open, !readinessDelivered, widget.superview === view, view.window != nil
        else {
            readinessScheduled = false

            return
        }

        readinessDelivered = true
        onReady?()
    }
    private func updateScrollSizes() {
        for scroll in widgets.values.compactMap({ $0 as? UIScrollView }) {
            let size = scroll.subviews.reduce(CGSize.zero) { size, child in
                CGSize(
                    width: max(size.width, child.frame.maxX),
                    height: max(size.height, child.frame.maxY))
            }

            if scroll.contentSize != size { scroll.contentSize = size }
        }
    }

    /// Scrolls each view whose target changed, once its frame and content size are current.
    private func scrollToTargets() {
        for (id, target) in scrolls.takeChanged() {
            guard let scroll = widgets[id] as? UIScrollView else { continue }
            let inset = scroll.adjustedContentInset
            let offset = ScrollTracker.clamp(
                target, least: CGPoint(x: -inset.left, y: -inset.top),
                most: CGPoint(
                    x: scroll.contentSize.width + inset.right - scroll.bounds.width,
                    y: scroll.contentSize.height + inset.bottom - scroll.bounds.height))
            scroll.setContentOffset(offset, animated: target.animated == true)
        }
    }

    private func create(_ item: Change.Widget) throws {
        let widget: UIView
        switch item.kind {
        case "view", "View": widget = recycled.views.popLast() ?? UIView()
        case "text", "text_run", "Text", "TextRun": widget = makeLabel()
        case "button", "Button": widget = makeButton(item.id)
        case "pressable", "Pressable": widget = makePressable(item.id)
        case "text_input", "TextInput": widget = makeTextField(item.id)
        case "scroll_view", "ScrollView":
            let scroll = UIScrollView()
            scroll.delegate = self
            widget = scroll
        case "image", "Image": widget = UIImageView()
        case "vector", "Vector": widget = VectorView()
        case "rive", "Rive": widget = NativeRiveImageView()
        default:
            throw NativeAppError(
                message: "Unsupported native widget: \(item.kind ?? "missing kind")")
        }
        widget.tag = item.id
        widgets[item.id] = widget
        radii[item.id] = item.props.cornerRadius
        try WidgetStyle.apply(item.props, to: widget)
        scrolls.receive(item.props.scrollOffset, for: item.id)
    }

    private func makeLabel() -> UILabel {
        if let label = recycled.labels.popLast() { return label }

        let label = UILabel()
        label.numberOfLines = 0

        return label
    }

    /// Press in and out follow the events on which UIKit sets and clears the
    /// button's highlight, so `active:` styles track it.
    private func makeButton(_ id: Int) -> UIButton {
        let button = UIButton(type: .system)
        button.addAction(UIAction { [weak self] _ in self?.event(id, 0) }, for: .touchUpInside)
        button.addAction(
            UIAction { [weak self] _ in self?.event(id, Self.pressIn) }, for: [.touchDown, .touchDragEnter])
        button.addAction(
            UIAction { [weak self] _ in self?.event(id, Self.pressOut) },
            for: [.touchDragExit, .touchUpInside, .touchUpOutside, .touchCancel])

        return button
    }

    private func makePressable(_ id: Int) -> UIView {
        let press = PressView()
        press.onHold = { [weak self] held in self?.event(id, held ? Self.pressIn : Self.pressOut) }
        press.isUserInteractionEnabled = true
        press.addGestureRecognizer(
            UITapGestureRecognizer(
                target: self, action: #selector(pressedWidget(_:))))

        return press
    }

    private func makeTextField(_ id: Int) -> UITextField {
        let field = UITextField()
        field.delegate = self
        field.addAction(
            UIAction { [weak self, weak field] _ in
                self?.event(id, 1, field?.text ?? "")
            }, for: .editingChanged)

        return field
    }
    private func apply(_ change: Change) throws {
        if let item = change.create { try create(item) }

        if let item = change.updateProps, let widget = widgets[item.id] {
            radii[item.id] = item.props.cornerRadius
            try WidgetStyle.apply(item.props, to: widget)
            scrolls.receive(item.props.scrollOffset, for: item.id)
        }

        if let item = change.updateLayout, let widget = widgets[item.id] {
            widget.frame = CGRect(
                x: item.offsetX, y: item.offsetY, width: item.width, height: item.height)
            widget.layer.cornerRadius = WidgetStyle.fitRadius(radii[item.id] ?? 0, to: widget.bounds.size)
        }

        applyTree(change)
    }

    private func applyTree(_ change: Change) {
        if let edge = change.insertChild, let parent = widgets[edge.parent],
           let child = widgets[edge.child] {
            detach(child)
            let count = childCounts[edge.parent, default: 0]
            parent.insertSubview(child, at: min(edge.index ?? count, count))
            childCounts[edge.parent] = count + 1
        }

        if let edge = change.removeChild, let child = widgets[edge.child] { detach(child) }

        if let item = change.delete {
            radii[item.id] = nil
            scrolls.forget(item.id)
            // A view still holding children cannot be handed out again.
            let empty = childCounts.removeValue(forKey: item.id) ?? 0 == 0

            if let widget = widgets.removeValue(forKey: item.id) {
                detach(widget)
                if empty { recycled.keep(widget) }
            }
        }
    }

    /// Removes a widget from its parent, keeping the parent's child count.
    private func detach(_ child: UIView) {
        guard let parent = child.superview else { return }

        if widgets[parent.tag] === parent, let count = childCounts[parent.tag] {
            childCounts[parent.tag] = count - 1
        }

        child.removeFromSuperview()
    }

    private func event(_ node: Int, _ kind: UInt32, _ text: String = "") {
        guard open, !applying else { return }
        onEvent?(node, kind, text)
    }

    @objc private func pressedWidget(_ sender: UITapGestureRecognizer) {
        if let view = sender.view { event(view.tag, 0) }
    }

    func textFieldShouldReturn(_ textField: UITextField) -> Bool {
        event(textField.tag, 2, textField.text ?? "")

        return true
    }

    func textFieldDidBeginEditing(_ textField: UITextField) { event(textField.tag, 3) }
    func textFieldDidEndEditing(_ textField: UITextField) { event(textField.tag, 4) }
}

/// Draws a `Vector` in shape layers, repainted only when its drawing, size or screen scale changes.
final class VectorView: UIView {
    private var shapes = VectorLayers()
    var drawing: VectorDrawing? { didSet { paint() } }

    override func layoutSubviews() {
        super.layoutSubviews()
        paint()
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        paint()
    }

    /// A labelled drawing is an image to assistive technology; an unlabelled one is decorative.
    func apply(_ props: WidgetProps) {
        isAccessibilityElement = !(props.accessibilityLabel ?? "").isEmpty
        accessibilityTraits = .image
        drawing = props.drawing
    }

    private func paint() {
        shapes.paint(drawing, in: layer)
    }
}

/// Plain containers and labels waiting to be reused, by exact class so a
/// subclass with its own behaviour is never handed out as a plain view.
struct RecycledViews {
    /// Enough for a screen of a few thousand views; beyond it, views go.
    private static let limit = 4096
    var views: [UIView] = []
    var labels: [UILabel] = []

    @MainActor
    mutating func keep(_ widget: UIView) {
        if type(of: widget) == UILabel.self, labels.count < Self.limit, let label = widget as? UILabel {
            label.frame = .zero
            labels.append(label)
        } else if type(of: widget) == UIView.self, views.count < Self.limit {
            widget.frame = .zero
            views.append(widget)
        }
    }
}
