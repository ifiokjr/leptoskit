import CoreGraphics

/// Shared widget properties decoded from native render transactions.
struct WidgetProps: Decodable {
    let text: String?
    let source: String?
    let placeholder: String?
    let value: String?
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
    let scroll: Scroll?

    /// Where a scroll container's content sits, and whether to animate there.
    struct Scroll: Decodable, Equatable {
        let x: CGFloat
        let y: CGFloat
        let animated: Bool

        private enum CodingKeys: String, CodingKey { case x, y, animated }

        init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            x = try container.decode(CGFloat.self, forKey: .x)
            y = try container.decode(CGFloat.self, forKey: .y)
            animated = try container.decodeIfPresent(Bool.self, forKey: .animated) ?? false
        }
    }
}

struct WidgetAria: Decodable {
    let ariaHidden: Bool?
}
