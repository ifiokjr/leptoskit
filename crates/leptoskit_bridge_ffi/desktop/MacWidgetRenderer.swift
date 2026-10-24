import AppKit
import LeptosKitBridge

final class FlippedView: NSView {
    override var isFlipped: Bool { true }
}

@MainActor
final class MacWidgetRenderer: NSObject, NSTextFieldDelegate {
    private let view: NSView
    private var widgets: [Int: NSView] = [:]
    private var applying = false
    var onEvent: ((Int, UInt32, String) -> Void)?
    init(view: NSView) { self.view = view }
    func close() { widgets.removeAll() }

    /// Mutations precede root attachment and scroll sizing while native events are suppressed.
    func render(_ transaction: Transaction) throws {
        guard transaction.version == 1 else { throw PluginError.unavailable }
        applying = true
        defer { applying = false }

        for change in transaction.mutations { try apply(change) }

        if let root = transaction.root, let widget = widgets[root], widget.superview !== view {
            view.addSubview(widget)
        }

        updateScrollSizes()
    }

    private func updateScrollSizes() {
        for scroll in widgets.values.compactMap({ $0 as? NSScrollView }) {
            guard let document = scroll.documentView else { continue }

            document.setFrameSize(document.subviews.reduce(scroll.contentSize) { size, child in
                CGSize(
                    width: max(size.width, child.frame.maxX),
                    height: max(size.height, child.frame.maxY)
                )
            })
        }
    }

    private func create(_ item: Change.Widget) throws {
        let widget: NSView
        switch item.kind {
        case "view", "View": widget = FlippedView()
        case "text", "text_run", "Text", "TextRun":
            widget = NSTextField(wrappingLabelWithString: "")
        case "button", "Button":
            let button = NSButton(title: "", target: self, action: #selector(pressed(_:)))
            button.bezelStyle = .rounded
            widget = button
        case "text_input", "TextInput":
            let field = NSTextField()
            field.delegate = self
            field.target = self
            field.action = #selector(submitted(_:))
            widget = field
        case "scroll_view", "ScrollView":
            let scroll = NSScrollView()
            scroll.documentView = FlippedView()
            scroll.hasVerticalScroller = true
            scroll.hasHorizontalScroller = true
            scroll.drawsBackground = false
            widget = scroll
        case "image", "Image": widget = NSImageView()
        default:
            throw NativeAppError(message: "Unsupported native widget: \(item.kind ?? "missing kind")")
        }
        widget.identifier = NSUserInterfaceItemIdentifier(String(item.id))
        widgets[item.id] = widget
        try MacWidgetStyle.apply(item.props, to: widget)
    }

    private func apply(_ change: Change) throws {
        if let item = change.create { try create(item) }

        if let item = change.updateProps, let widget = widgets[item.id] {
            try MacWidgetStyle.apply(item.props, to: widget)
        }

        if let item = change.updateLayout, let widget = widgets[item.id] {
            widget.frame = CGRect(
                x: item.offsetX, y: item.offsetY, width: item.width, height: item.height
            )
        }

        applyTree(change)
    }

    private func applyTree(_ change: Change) {
        if let edge = change.insertChild { insertChild(edge) }

        if let edge = change.removeChild { widgets[edge.child]?.removeFromSuperview() }

        if let item = change.delete { widgets.removeValue(forKey: item.id)?.removeFromSuperview() }
    }

    private func insertChild(_ edge: Change.Edge) {
        if let parent = widgets[edge.parent], let child = widgets[edge.child] {
            let container = (parent as? NSScrollView)?.documentView ?? parent
            child.removeFromSuperview()
            let index = min(edge.index ?? container.subviews.count, container.subviews.count)

            let sibling = index < container.subviews.count ? container.subviews[index] : nil
            container.addSubview(
                child, positioned: sibling == nil ? .above : .below, relativeTo: sibling
            )
        }
    }

    private func node(_ widget: NSView) -> Int? { widget.identifier.flatMap { Int($0.rawValue) } }

    @objc private func pressed(_ sender: NSButton) {
        if let id = node(sender) { event(id, 0) }
    }

    @objc private func submitted(_ sender: NSTextField) {
        if let id = node(sender) { event(id, 2, sender.stringValue) }
    }

    func controlTextDidChange(_ notification: Notification) {
        if let field = notification.object as? NSTextField, let id = node(field) {
            event(id, 1, field.stringValue)
        }
    }

    func controlTextDidBeginEditing(_ notification: Notification) {
        if let field = notification.object as? NSTextField, let id = node(field) { event(id, 3) }
    }

    func controlTextDidEndEditing(_ notification: Notification) {
        if let field = notification.object as? NSTextField, let id = node(field) { event(id, 4) }
    }

    private func event(_ node: Int, _ kind: UInt32, _ text: String = "") {
        guard !applying else { return }
        onEvent?(node, kind, text)
    }
}

/// Resolves AppKit properties at the rendering boundary.
@MainActor
enum MacWidgetStyle {
    private static let defaultFontSize: CGFloat = 14
    private static let greenShift: UInt32 = 16
    private static let blueShift: UInt32 = 8

    static func apply(_ props: WidgetProps, to widget: NSView) throws {
        widget.setAccessibilityLabel(props.accessibilityLabel)
        widget.setAccessibilityIdentifier(props.accessibilityLabel)
        widget.wantsLayer = true
        applyLayer(props, to: widget)
        widget.isHidden = props.hidden ?? false
        widget.alphaValue = props.opacity ?? 1
        applyText(props, to: widget)

        if let image = widget as? NSImageView { try applyImage(props.source, to: image) }
    }

    private static func applyLayer(_ props: WidgetProps, to widget: NSView) {
        let layer = widget.layer
        layer?.backgroundColor = props.background.map(color)?.cgColor
        layer?.cornerRadius = props.cornerRadius ?? 0
        applyBorder(props, to: widget)
        layer?.masksToBounds = (props.clip ?? false) || (props.cornerRadius ?? 0) > 0
    }

    private static func applyBorder(_ props: WidgetProps, to widget: NSView) {
        widget.layer?.borderWidth = props.borderWidth ?? 0
        widget.layer?.borderColor = props.borderColor.map(color)?.cgColor
    }

    private static func applyImage(_ source: String?, to image: NSImageView) throws {
        image.imageScaling = .scaleProportionallyUpOrDown

        guard let source else { image.image = nil; return }

        guard let url = URL(string: source), url.isFileURL,
              let loaded = NSImage(contentsOf: url) else {
            throw NativeAppError(message: "Image requires a readable file URL: \(source)")
        }

        image.image = loaded
    }

    private static func font(_ props: WidgetProps) -> NSFont {
        let weight: NSFont.Weight = switch props.fontWeight {
        case "bold": .bold
        case "semibold": .semibold
        case "medium": .medium
        default: .regular
        }

        return NSFont.systemFont(ofSize: props.fontSize ?? defaultFontSize, weight: weight)
    }

    private static func alignment(_ value: String?) -> NSTextAlignment {
        switch value {
        case "center": .center
        case "end": .right
        default: .natural
        }
    }

    private static func applyText(_ props: WidgetProps, to widget: NSView) {
        if let button = widget as? NSButton { applyButton(props, to: button) }

        if let field = widget as? NSTextField { applyField(props, to: field) }
    }

    private static func applyButton(_ props: WidgetProps, to button: NSButton) {
        button.title = props.text ?? ""
        button.font = font(props)
        button.contentTintColor = props.color.map(color)
        button.isEnabled = props.enabled ?? true
    }

    private static func applyField(_ props: WidgetProps, to field: NSTextField) {
        let value = text(props, editable: field.isEditable)

        if field.stringValue != value { field.stringValue = value }
        field.placeholderString = props.placeholder
        field.font = font(props)
        field.textColor = props.color.map(color) ?? .labelColor
        field.alignment = alignment(props.textAlign)
        field.isEnabled = props.enabled ?? true
    }

    private static func text(_ props: WidgetProps, editable: Bool) -> String {
        guard editable else { return props.text ?? "" }

        return props.value ?? props.text ?? ""
    }

    private static func color(_ rgba: UInt32) -> NSColor {
        NSColor(
            red: CGFloat((rgba >> 24) & 255) / 255,
            green: CGFloat((rgba >> greenShift) & 255) / 255,
            blue: CGFloat((rgba >> blueShift) & 255) / 255,
            alpha: CGFloat(rgba & 255) / 255
        )
    }
}
