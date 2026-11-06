import CoreGraphics

/// Shared widget properties decoded from native render transactions.
struct WidgetProps: Decodable {
    let text: String?
    let source: String?
    let placeholder: String?
    /// What the field holds, as HTML's `autocomplete`; read it through `textContent`.
    let autocomplete: String?
    let value: String?
    let checked: Bool?
    let slider: WidgetSlider?
    let progress: WidgetProgress?
    let running: Bool?
    let focused: Bool?
    let accessibilityLabel: String?
    let color: UInt32?
    let background: UInt32?
    let enabled: Bool?
    let hidden: Bool?
    let clip: Bool?
    let cornerRadius: CGFloat?
    let borderWidth: CGFloat?
    let borderColor: UInt32?
    let opacity: CGFloat?
    let fontSize: CGFloat?
    let fontWeight: String?
    let textAlign: String?
    let drawing: VectorDrawing?
    let aria: WidgetAria?
    let scrollOffset: ScrollOffset?
    let transform: WidgetTransform?
    let animation: WidgetAnimation?
}

extension WidgetProps {
    /// The first `autocomplete` token password managers read on every platform.
    var textContent: TextContent? {
        autocomplete?.lowercased().split(whereSeparator: \.isWhitespace)
            .lazy.compactMap { TextContent(rawValue: String($0)) }.first
    }
}

/// What a text field holds, which the platform's content type tells AutoFill and password managers.
enum TextContent: String {
    case username, email
    case currentPassword = "current-password"
    case newPassword = "new-password"
}

struct WidgetAria: Decodable {
    let ariaHidden: Bool?
    let valueText: String?
}

/// Where a scroll view's content is scrolled to, in points, as `Element::scroll_offset` sets it.
struct ScrollOffset: Decodable, Equatable {
    let offsetX: CGFloat
    let offsetY: CGFloat
    /// Unset means the content jumps there.
    let animated: Bool?

    private enum CodingKeys: String, CodingKey {
        case offsetX = "x", offsetY = "y", animated
    }
}

/// The guest validates the range and step grid. UIKit uses a normalized Float
/// coordinate so large Double ranges do not overflow UISlider.
struct WidgetSlider: Decodable {
    let min: Double
    let max: Double
    let value: Double
    let step: Double?

    static let initial = Self(min: 0, max: 1, value: 0, step: nil)
    var coordinate: Float { Float((value - min) / (max - min)) }

    func proposal(_ coordinate: Float) -> Double {
        let value = min + Double(Swift.max(0, Swift.min(1, coordinate))) * (max - min)
        return snapped(value)
    }

    func snapped(_ proposal: Double) -> Double {
        let value = Swift.max(min, Swift.min(max, proposal))
        guard let step else { return value }
        let candidate = ((max - min) / step).rounded()
        let boundary = min + candidate * step
        let tolerance = Swift.max(boundary.ulp, max.ulp)
        let last = boundary <= max || abs(boundary - max) <= tolerance
            ? candidate : Swift.max(candidate - 1, 0)
        let index = Swift.max(0, Swift.min(last, ((value - min) / step).rounded()))
        return Swift.max(min, Swift.min(max, min + index * step))
    }
}

/// Rust's externally tagged Progress enum: "indeterminate" or {"determinate": 0.5}.
enum WidgetProgress: Decodable {
    case indeterminate
    case determinate(Float)

    private enum CodingKeys: String, CodingKey { case determinate }

    init(from decoder: Decoder) throws {
        if let text = try? decoder.singleValueContainer().decode(String.self), text == "indeterminate" {
            self = .indeterminate
        } else {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            self = .determinate(try container.decode(Float.self, forKey: .determinate))
        }
    }
}
