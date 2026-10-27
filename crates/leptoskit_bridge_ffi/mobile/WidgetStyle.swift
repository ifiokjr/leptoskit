import UIKit

/// Applies renderer-owned widget props in view, text and image order.
enum WidgetStyle {
    private static let redShift: UInt32 = 24
    private static let greenShift: UInt32 = 16
    private static let blueShift: UInt32 = 8
    private static let channelMask: UInt32 = 255
    private static let channelMaximum: CGFloat = 255

    static func apply(_ props: WidgetProps, to widget: UIView) throws {
        widget.accessibilityLabel = props.accessibilityLabel
        widget.accessibilityIdentifier = props.accessibilityLabel
        widget.backgroundColor = props.background.map(color) ?? .clear
        widget.isOpaque = false
        widget.isUserInteractionEnabled = props.enabled ?? true
        widget.isHidden = props.hidden ?? false
        widget.alpha = props.opacity ?? 1
        widget.layer.cornerRadius = fitRadius(props.cornerRadius ?? 0, to: widget.bounds.size)
        widget.layer.borderWidth = props.borderWidth ?? 0
        widget.layer.borderColor = props.borderColor.map(color)?.cgColor

        applyClipping(props, to: widget)
        WidgetTextStyle.apply(props, to: widget)
        try WidgetImageStyle.apply(props, to: widget)
    }

    private static func applyClipping(_ props: WidgetProps, to widget: UIView) {
        widget.clipsToBounds = (props.clip ?? false) || (props.cornerRadius ?? 0) > 0
    }

    /// Scales a radius down to fit the view, as CSS does: `rounded-full`
    /// asks for 9999 points, and a layer whose radius exceeds half its size
    /// draws nothing once it clips.
    static func fitRadius(_ requested: CGFloat, to size: CGSize) -> CGFloat {
        max(0, min(requested, size.width / 2, size.height / 2))
    }

    static func color(_ rgba: UInt32) -> UIColor {
        return UIColor(
            red: CGFloat((rgba >> redShift) & channelMask) / channelMaximum,
            green: CGFloat((rgba >> greenShift) & channelMask) / channelMaximum,
            blue: CGFloat((rgba >> blueShift) & channelMask) / channelMaximum,
            alpha: CGFloat(rgba & channelMask) / channelMaximum
        )
    }
}
