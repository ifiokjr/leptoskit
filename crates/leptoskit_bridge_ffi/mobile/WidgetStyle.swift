import UIKit

/// Applies renderer-owned widget props to UIKit views. Stateless: the view
/// controller owns the widget tree and calls this for each create and update.
enum WidgetStyle {
    static func apply(_ props: WidgetProps, to widget: UIView) throws {
        widget.accessibilityLabel = props.accessibilityLabel
        widget.accessibilityIdentifier = props.accessibilityLabel
        widget.backgroundColor = props.background.map(color) ?? .clear
        widget.isOpaque = false
        widget.isUserInteractionEnabled = props.enabled ?? true
        widget.isHidden = props.hidden ?? false
        widget.alpha = props.opacity ?? 1
        widget.layer.cornerRadius = props.cornerRadius ?? 0
        widget.layer.borderWidth = props.borderWidth ?? 0
        widget.layer.borderColor = props.borderColor.map(color)?.cgColor
        widget.clipsToBounds = (props.clip ?? false) || (props.cornerRadius ?? 0) > 0
        applyText(props, to: widget)
        try applyImage(props, to: widget)
    }

    private static func font(_ props: WidgetProps) -> UIFont {
        let weight: UIFont.Weight = switch props.fontWeight {
        case "bold": .bold
        case "semibold": .semibold
        case "medium": .medium
        default: .regular
        }
        return UIFont.systemFont(ofSize: props.fontSize ?? 14, weight: weight)
    }

    private static func applyText(_ props: WidgetProps, to widget: UIView) {
        let font = font(props)
        let alignment: NSTextAlignment = switch props.textAlign {
        case "center": .center
        case "end": .right
        default: .natural
        }
        if let label = widget as? UILabel {
            label.text = props.text
            label.font = font
            label.textColor = props.color.map(color) ?? .label
            label.textAlignment = alignment
        }
        if let button = widget as? UIButton {
            button.setTitle(props.text, for: .normal)
            button.titleLabel?.font = font
            button.setTitleColor(props.color.map(color) ?? .systemBlue, for: .normal)
            button.isEnabled = props.enabled ?? true
        }
        if let field = widget as? UITextField {
            let value = props.value ?? props.text ?? ""
            if field.text != value { field.text = value }
            field.placeholder = props.placeholder
            field.font = font
            field.textColor = props.color.map(color) ?? .label
            field.textAlignment = alignment
            field.isEnabled = props.enabled ?? true
        }
    }

    private static func applyImage(_ props: WidgetProps, to widget: UIView) throws {
        if let image = widget as? UIImageView {
            image.contentMode = .scaleAspectFit
            if let source = props.source {
                guard let url = URL(string: source), url.isFileURL,
                      let loaded = UIImage(contentsOfFile: url.path) else {
                    throw RuntimeFailure(message: "Image requires a readable file URL: \(source)")
                }
                image.image = loaded
            } else { image.image = nil }
        }
    }

    private static func color(_ rgba: UInt32) -> UIColor {
        UIColor(red: CGFloat((rgba >> 24) & 255) / 255,
                green: CGFloat((rgba >> 16) & 255) / 255,
                blue: CGFloat((rgba >> 8) & 255) / 255,
                alpha: CGFloat(rgba & 255) / 255)
    }
}
