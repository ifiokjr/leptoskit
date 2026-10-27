import UIKit

/// Applies each text-capable view's existing UIKit defaults.
@MainActor
enum WidgetTextStyle {
    private static let defaultSize: CGFloat = 14

    static func apply(_ props: WidgetProps, to widget: UIView) {
        let font = font(props)
        let alignment = alignment(props)

        if let label = widget as? UILabel {
            label.text = props.text
            label.accessibilityValue = props.accessibilityLabel == nil ? nil : props.text
            label.font = font
            label.textColor = textColor(props)
            label.textAlignment = alignment
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

    private static func font(_ props: WidgetProps) -> UIFont {
        let weight: UIFont.Weight =
            switch props.fontWeight {
            case "bold": .bold
            case "semibold": .semibold
            case "medium": .medium
            default: .regular
            }

        return UIFont.systemFont(ofSize: props.fontSize ?? defaultSize, weight: weight)
    }
}
