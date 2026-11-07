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

    /// The most colors kept: an animated arbitrary color makes a new one
    /// every frame, and the cache outlives every app the process runs.
    private static let colorLimit = 256

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
        applyControl(props, to: widget)
        try WidgetImageStyle.apply(props, to: widget)
        (widget as? VectorView)?.apply(props)
    }

    private static func applyControl(_ props: WidgetProps, to widget: UIView) {
        let tint = props.color.map(color) ?? .systemBlue
        (widget as? UIControl)?.isEnabled = props.enabled ?? true
        if let control = widget as? UISwitch {
            control.isOn = props.checked ?? false
            control.onTintColor = tint
        }
        if let button = widget as? ChoiceButton {
            button.isSelected = props.checked ?? false
            button.tintColor = tint
            button.setImage(UIImage(systemName: button.radio ? "circle" : "square"), for: .normal)
            button.setImage(UIImage(systemName: button.radio ? "largecircle.fill.circle" : "checkmark.square.fill"),
                for: .selected)
            button.accessibilityValue = button.isSelected ? "1" : "0"
            button.accessibilityTraits = [.button]
            if button.isSelected { button.accessibilityTraits.insert(.selected) }
            if !button.isEnabled { button.accessibilityTraits.insert(.notEnabled) }
        }
        if let slider = widget as? UISlider {
            let state = props.slider ?? .initial
            slider.value = state.coordinate
            slider.minimumTrackTintColor = tint
            slider.accessibilityValue = props.aria?.valueText ?? String(state.value)
        }
        if let progress = widget as? LinearProgressView { progress.apply(props) }
        if let spinner = widget as? UIActivityIndicatorView {
            spinner.color = tint
            spinner.hidesWhenStopped = false
            if props.running != false && props.hidden != true {
                spinner.startAnimating()
            } else { spinner.stopAnimating() }
            spinner.isAccessibilityElement = props.accessibilityLabel != nil
        }
        if widget is UISwitch || widget is ChoiceButton || widget is UISlider
            || widget is LinearProgressView || widget is UIActivityIndicatorView {
            widget.accessibilityElementsHidden = props.aria?.ariaHidden ?? false
        }
    }

    @MainActor
    private static func applyClipping(_ props: WidgetProps, to widget: UIView) {
        let clips = (props.clip ?? false) || (props.cornerRadius ?? 0) > 0

        if widget.clipsToBounds != clips { widget.clipsToBounds = clips }
    }

    /// Places a widget at its layout frame. A transformed view's `frame` is its transformed outline, so
    /// one with a transform is placed by its untransformed box.
    static func place(_ widget: UIView, _ frame: CGRect) {
        if CATransform3DIsIdentity(widget.layer.transform) {
            widget.frame = frame
        } else {
            widget.bounds.size = frame.size
            widget.center = CGPoint(x: frame.midX, y: frame.midY)
        }
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

        if colors.count >= colorLimit { colors.removeAll(keepingCapacity: true) }

        colors[rgba] = created

        return created
    }
}

/// UIKit has no checkbox/radio control; system-symbol buttons match the Rust backend.
final class ChoiceButton: UIButton {
    var radio = false
}

/// A determinate native bar or an indeterminate sweep, within the guest's frame.
final class LinearProgressView: UIView {
    private let bar = UIProgressView(progressViewStyle: .bar)
    private let sweep = CALayer()
    private var indeterminate = true

    override init(frame: CGRect) {
        super.init(frame: frame)
        addSubview(bar)
        layer.addSublayer(sweep)
        isAccessibilityElement = true
        bar.isAccessibilityElement = false
        clipsToBounds = true
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func apply(_ props: WidgetProps) {
        let tint = props.color.map(WidgetStyle.color) ?? .systemBlue
        bar.progressTintColor = tint
        bar.trackTintColor = props.background.map(WidgetStyle.color)
        sweep.backgroundColor = tint.cgColor
        switch props.progress ?? .indeterminate {
        case .indeterminate:
            indeterminate = true
            accessibilityValue = props.aria?.valueText ?? "Loading"
        case .determinate(let value):
            indeterminate = false
            bar.progress = value
            accessibilityValue = props.aria?.valueText ?? "\(Int((value * 100).rounded()))%"
        }
        bar.isHidden = indeterminate
        sweep.isHidden = !indeterminate
        setNeedsLayout()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        bar.transform = .identity
        bar.frame = bounds
        let height = bar.bounds.height
        if height > 0 { bar.transform = CGAffineTransform(scaleX: 1, y: bounds.height / height) }
        bar.center = CGPoint(x: bounds.midX, y: bounds.midY)
        sweep.frame = CGRect(x: -bounds.width / 3, y: 0, width: bounds.width / 3, height: bounds.height)
        sweep.removeAnimation(forKey: "progress")
        if indeterminate && window != nil && !isHidden && bounds.width > 0 {
            let animation = CABasicAnimation(keyPath: "transform.translation.x")
            animation.fromValue = 0
            animation.toValue = bounds.width * 4 / 3
            animation.duration = 1
            animation.repeatCount = .infinity
            sweep.add(animation, forKey: "progress")
        }
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        setNeedsLayout()
        if window == nil { sweep.removeAllAnimations() }
    }
}

/// VoiceOver adjusts by the guest's step, rather than UIKit's normalized 10%.
final class ValueSlider: UISlider {
    var onStep: ((Bool) -> Void)?
    override func accessibilityIncrement() { onStep?(true) }
    override func accessibilityDecrement() { onStep?(false) }
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
