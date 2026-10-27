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
final class MobileWidgetRenderer: NSObject, UITextFieldDelegate {
    private static let pressIn: UInt32 = 8
    private static let pressOut: UInt32 = 9
    private let view: UIView
    private var widgets: [Int: UIView] = [:]
    /// Requested corner radii, fitted to each frame as layout changes it.
    private var radii: [Int: CGFloat] = [:]
    private var applying = false
    private var readinessScheduled = false
    private var readinessDelivered = false
    private var open = true
    var onReady: (() -> Void)?
    var onEvent: ((Int, UInt32, String) -> Void)?
    /// The root widget's opaque background. Like a browser painting the whole
    /// canvas with the root element's background, the host paints it behind
    /// the safe areas; nil leaves the host's own background.
    private(set) var canvas: UIColor?
    var onCanvasChange: (() -> Void)?
    /// A transaction names its root only when the root changes.
    private var root: Int?
    init(view: UIView) { self.view = view }
    func close() {
        open = false
        widgets.removeAll()
        radii.removeAll()
    }

    func render(_ transaction: Transaction) throws {
        guard transaction.version == 1 else { throw PluginError.unavailable }
        applying = true
        defer { applying = false }

        for change in transaction.mutations {
            try apply(change)
        }

        if let root = transaction.root, let widget = widgets[root], widget.superview !== view {
            view.addSubview(widget)
        }

        scheduleReadiness(transaction.root)

        updateScrollSizes()
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
            scroll.contentSize = scroll.subviews.reduce(CGSize.zero) { size, child in
                CGSize(
                    width: max(size.width, child.frame.maxX),
                    height: max(size.height, child.frame.maxY))
            }
        }
    }

    private func create(_ item: Change.Widget) throws {
        let widget: UIView
        switch item.kind {
        case "view", "View": widget = UIView()
        case "text", "text_run", "Text", "TextRun":
            let label = UILabel()
            label.numberOfLines = 0
            widget = label
        case "button", "Button": widget = makeButton(item.id)
        case "pressable", "Pressable": widget = makePressable(item.id)
        case "text_input", "TextInput": widget = makeTextField(item.id)
        case "scroll_view", "ScrollView": widget = UIScrollView()
        case "image", "Image": widget = UIImageView()
        default:
            throw NativeAppError(
                message: "Unsupported native widget: \(item.kind ?? "missing kind")")
        }
        widget.tag = item.id
        widgets[item.id] = widget
        radii[item.id] = item.props.cornerRadius
        try WidgetStyle.apply(item.props, to: widget)
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
            child.removeFromSuperview()
            parent.insertSubview(
                child, at: min(edge.index ?? parent.subviews.count, parent.subviews.count))
        }

        if let edge = change.removeChild { widgets[edge.child]?.removeFromSuperview() }

        if let item = change.delete {
            radii[item.id] = nil
            widgets.removeValue(forKey: item.id)?.removeFromSuperview()
        }
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
