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
    let scrollOffset: ScrollOffset?
}

struct WidgetAria: Decodable {
    let ariaHidden: Bool?
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
