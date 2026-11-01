import UIKit

/// Applies renderer-owned widget props in view, text and image order.
@MainActor
enum WidgetStyle {
    private static let redShift: UInt32 = 24
    private static let greenShift: UInt32 = 16
    private static let blueShift: UInt32 = 8
    private static let channelMask: UInt32 = 255
    private static let channelMaximum: CGFloat = 255

    /// Colors by their packed RGBA value: a screen reuses a few colors on
    /// many views, and creating each again costs an allocation.
    private static var colors: [UInt32: UIColor] = [:]

    /// Sets each property only when it differs from the view's own value: a
    /// new view already holds the defaults, and UIKit setters do work even
    /// for an unchanged value, which added up across a screen of views.
    @MainActor
    static func apply(_ props: WidgetProps, to widget: UIView) throws {
        if widget.accessibilityLabel != props.accessibilityLabel {
            widget.accessibilityLabel = props.accessibilityLabel
        }

        if widget.accessibilityIdentifier != props.accessibilityLabel {
            widget.accessibilityIdentifier = props.accessibilityLabel
        }

        let background = props.background.map(color) ?? .clear

        if widget.backgroundColor != background { widget.backgroundColor = background }

        if widget.isOpaque { widget.isOpaque = false }
        let interactive = props.enabled ?? true

        if widget.isUserInteractionEnabled != interactive { widget.isUserInteractionEnabled = interactive }
        let hidden = props.hidden ?? false

        if widget.isHidden != hidden { widget.isHidden = hidden }
        let alpha = props.opacity ?? 1

        if widget.alpha != alpha { widget.alpha = alpha }
        let radius = fitRadius(props.cornerRadius ?? 0, to: widget.bounds.size)

        if widget.layer.cornerRadius != radius { widget.layer.cornerRadius = radius }
        let borderWidth = props.borderWidth ?? 0

        if widget.layer.borderWidth != borderWidth { widget.layer.borderWidth = borderWidth }
        let borderColor = props.borderColor.map(color)?.cgColor

        if widget.layer.borderColor != borderColor { widget.layer.borderColor = borderColor }

        applyClipping(props, to: widget)
        WidgetTextStyle.apply(props, to: widget)
        try WidgetImageStyle.apply(props, to: widget)
        (widget as? VectorView)?.apply(props)
    }

    @MainActor
    private static func applyClipping(_ props: WidgetProps, to widget: UIView) {
        let clips = (props.clip ?? false) || (props.cornerRadius ?? 0) > 0

        if widget.clipsToBounds != clips { widget.clipsToBounds = clips }
    }

    /// Scales a radius down to fit the view, as CSS does: `rounded-full`
    /// asks for 9999 points, and a layer whose radius exceeds half its size
    /// draws nothing once it clips.
    static func fitRadius(_ requested: CGFloat, to size: CGSize) -> CGFloat {
        max(0, min(requested, size.width / 2, size.height / 2))
    }

    static func color(_ rgba: UInt32) -> UIColor {
        if let cached = colors[rgba] { return cached }

        let created = UIColor(
            red: CGFloat((rgba >> redShift) & channelMask) / channelMaximum,
            green: CGFloat((rgba >> greenShift) & channelMask) / channelMaximum,
            blue: CGFloat((rgba >> blueShift) & channelMask) / channelMaximum,
            alpha: CGFloat(rgba & channelMask) / channelMaximum
        )
        colors[rgba] = created

        return created
    }
}
