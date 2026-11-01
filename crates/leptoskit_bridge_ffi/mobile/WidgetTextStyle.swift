import UIKit

/// Applies each text-capable view's existing UIKit defaults.
@MainActor
enum WidgetTextStyle {
    private static let defaultSize: CGFloat = 14

    static func apply(_ props: WidgetProps, to widget: UIView) {
        let font = font(props)
        let alignment = alignment(props)

        if let label = widget as? UILabel {
            // Each setter re-lays out the label's text, so only changes are set.
            if label.text != props.text { label.text = props.text }
            let accessibilityLabel = props.accessibilityLabel ?? props.text

            if label.accessibilityLabel != accessibilityLabel { label.accessibilityLabel = accessibilityLabel }
            let accessibilityValue = props.accessibilityLabel == nil ? nil : props.text

            if label.accessibilityValue != accessibilityValue { label.accessibilityValue = accessibilityValue }

            if label.font !== font { label.font = font }
            let color = textColor(props)

            if label.textColor != color { label.textColor = color }

            if label.textAlignment != alignment { label.textAlignment = alignment }
        }

        if let button = widget as? UIButton {
            button.setTitle(props.text, for: .normal)
            button.titleLabel?.font = font
            button.setTitleColor(
                props.color.map(WidgetStyle.color) ?? .systemBlue,
                for: .normal
            )
            button.isEnabled = props.enabled ?? true
        }

        if let field = widget as? UITextField {
            applyField(props, to: field, font: font, alignment: alignment)
        }
    }

    private static func applyField(
        _ props: WidgetProps, to field: UITextField, font: UIFont, alignment: NSTextAlignment
    ) {
        let value = props.value ?? props.text ?? ""

        if field.text != value { field.text = value }

        field.placeholder = props.placeholder
        field.font = font
        field.textColor = textColor(props)
        field.textAlignment = alignment
        field.isEnabled = props.enabled ?? true
    }

    private static func textColor(_ props: WidgetProps) -> UIColor {
        props.color.map(WidgetStyle.color) ?? .label
    }

    private static func alignment(_ props: WidgetProps) -> NSTextAlignment {
        switch props.textAlign {
        case "center": .center
        case "end": .right
        default: .natural
        }
    }

    /// System fonts by size and weight: every label asks for one, and a
    /// screen uses only a few.
    private static var fonts: [FontKey: UIFont] = [:]

    private struct FontKey: Hashable {
        let size: CGFloat
        let weight: CGFloat
    }

    private static func font(_ props: WidgetProps) -> UIFont {
        let weight: UIFont.Weight =
            switch props.fontWeight {
            case "bold": .bold
            case "semibold": .semibold
            case "medium": .medium
            default: .regular
            }
        let size = props.fontSize ?? defaultSize
        let key = FontKey(size: size, weight: weight.rawValue)

        if let cached = fonts[key] { return cached }

        let font = UIFont.systemFont(ofSize: size, weight: weight)
        fonts[key] = font

        return font
    }
}
