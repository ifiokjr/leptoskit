import Foundation
import QuartzCore

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
}

struct WidgetLayout: Decodable {
    let id: Int
    let offsetX: CGFloat
    let offsetY: CGFloat
    let width: CGFloat
    let height: CGFloat
    enum CodingKeys: String, CodingKey {
        case id, width, height
        case offsetX = "x"
        case offsetY = "y"
    }
}

struct Change: Decodable {
    struct Widget: Decodable { let id: Int; let kind: String?; let props: WidgetProps }
    struct Edge: Decodable { let parent: Int; let child: Int; let index: Int? }
    struct Removed: Decodable { let id: Int }
    let create: Widget?
    let updateProps: Widget?
    let updateLayout: WidgetLayout?
    let insertChild: Edge?
    let removeChild: Edge?
    let delete: Removed?
    enum CodingKeys: String, CodingKey {
        case create = "Create"
        case updateProps = "UpdateProps"
        case updateLayout = "UpdateLayout"
        case insertChild = "InsertChild"
        case removeChild = "RemoveChild"
        case delete = "Delete"
    }
}

struct Transaction: Decodable {
    let version: Int
    let root: Int?
    let mutations: [Change]
}

/// A `Vector`'s `drawing` prop: paths in view-box coordinates, mapped onto the frame at paint time.
struct VectorDrawing: Decodable {
    struct ViewBox: Decodable {
        let minX: CGFloat
        let minY: CGFloat
        let width: CGFloat
        let height: CGFloat
    }

    enum Scaling: String, Decodable { case fit, stretch }
    enum CodingKeys: String, CodingKey { case viewBox, scaling, paths }
    let viewBox: ViewBox
    let scaling: Scaling?
    let paths: [VectorPath]?

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let viewBox = try container.decode(ViewBox.self, forKey: .viewBox)

        guard [viewBox.minX, viewBox.minY, viewBox.width, viewBox.height].allSatisfy(\.isFinite),
              viewBox.width > 0, viewBox.height > 0 else {
            throw DecodingError.dataCorruptedError(
                forKey: .viewBox, in: container, debugDescription: "A view box needs a positive, finite size"
            )
        }

        self.viewBox = viewBox
        scaling = try container.decodeIfPresent(Scaling.self, forKey: .scaling)
        paths = try container.decodeIfPresent([VectorPath].self, forKey: .paths)
    }

    /// Maps view-box coordinates onto a frame whose origin is its top-left corner.
    func transform(for size: CGSize) -> CGAffineTransform {
        let width = max(size.width, 0)
        let height = max(size.height, 0)
        let scaleX = width / viewBox.width
        let scaleY = height / viewBox.height

        switch scaling ?? .fit {
        case .stretch:
            return CGAffineTransform(
                a: scaleX, b: 0, c: 0, d: scaleY, tx: -viewBox.minX * scaleX, ty: -viewBox.minY * scaleY
            )
        case .fit:
            let scale = min(scaleX, scaleY)

            return CGAffineTransform(
                a: scale, b: 0, c: 0, d: scale,
                tx: (width - viewBox.width * scale) / 2 - viewBox.minX * scale,
                ty: (height - viewBox.height * scale) / 2 - viewBox.minY * scale
            )
        }
    }
}

/// One outline painted with an optional fill, then an optional stroke.
///
/// Path data arrives canonical (absolute `M`, `L`, `Q`, `C` and `Z`, one space between tokens),
/// so it splits on spaces instead of parsing SVG. Malformed data fails the transaction decode.
struct VectorPath: Decodable {
    private static let pointCounts: [Substring: Int] = ["M": 1, "L": 1, "Q": 2, "C": 3, "Z": 0]
    enum CodingKeys: String, CodingKey { case data, fill, stroke }
    /// The outline in view-box coordinates.
    let outline: CGPath
    let fill: UInt32?
    let stroke: VectorStroke?

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let data = try container.decode(String.self, forKey: .data)

        guard let outline = Self.outline(data) else {
            throw DecodingError.dataCorruptedError(
                forKey: .data, in: container, debugDescription: "Invalid vector path data: \(data)"
            )
        }

        self.outline = outline
        fill = try container.decodeIfPresent(UInt32.self, forKey: .fill)
        stroke = try container.decodeIfPresent(VectorStroke.self, forKey: .stroke)
    }

    /// Builds canonical path data, or nil when a command or coordinate is malformed.
    static func outline(_ data: String) -> CGPath? {
        var tokens = data.split(separator: " ")[...]
        let path = CGMutablePath()

        while let command = tokens.popFirst() {
            guard let count = pointCounts[command], command == "M" || !path.isEmpty,
                  let points = points(count, from: &tokens)
            else { return nil }

            switch command {
            case "M": path.move(to: points[0])
            case "L": path.addLine(to: points[0])
            case "Q": path.addQuadCurve(to: points[1], control: points[0])
            case "C": path.addCurve(to: points[2], control1: points[0], control2: points[1])
            default: path.closeSubpath()
            }
        }

        return path.copy()
    }

    private static func points(_ count: Int, from tokens: inout ArraySlice<Substring>) -> [CGPoint]? {
        guard tokens.count >= count * 2 else { return nil }
        let values = tokens.prefix(count * 2).compactMap { Double($0) }.filter(\.isFinite)
        tokens.removeFirst(count * 2)

        guard values.count == count * 2 else { return nil }

        return stride(from: 0, to: values.count, by: 2).map { CGPoint(x: values[$0], y: values[$0 + 1]) }
    }
}

/// A stroke in layout units: the drawing's scale never changes its width.
struct VectorStroke: Decodable {
    enum Cap: String, Decodable { case butt, round, square }
    enum Join: String, Decodable { case miter, round, bevel }
    /// Strokes miter until the miter length reaches four times the line width.
    static let miterLimit: CGFloat = 4
    let color: UInt32
    let width: CGFloat
    let cap: Cap?
    let join: Join?
}

/// Paints a drawing with one shape layer per paint: each path's fill, then its stroke, in order.
///
/// Outline points are mapped onto the frame before stroking, so line widths stay in layout units.
@MainActor
enum VectorLayers {
    /// Replaces `current` under `host` with layers for `drawing` at `host`'s size and scale.
    static func replace(_ current: [CAShapeLayer], in host: CALayer, drawing: VectorDrawing?) -> [CAShapeLayer] {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        defer { CATransaction.commit() }

        current.forEach { $0.removeFromSuperlayer() }
        guard let drawing else { return [] }

        var transform = drawing.transform(for: host.bounds.size)
        let layers = (drawing.paths ?? []).flatMap { path -> [CAShapeLayer] in
            guard let outline = path.outline.copy(using: &transform) else { return [] }

            return [path.fill.map { fill($0, outline) }, path.stroke.map { stroke($0, outline) }].compactMap(\.self)
        }

        for layer in layers {
            layer.frame = host.bounds
            layer.contentsScale = host.contentsScale
            host.addSublayer(layer)
        }

        return layers
    }

    private static func fill(_ rgba: UInt32, _ outline: CGPath) -> CAShapeLayer {
        let layer = CAShapeLayer()
        layer.path = outline
        layer.fillRule = .nonZero
        layer.fillColor = color(rgba)
        layer.strokeColor = nil

        return layer
    }

    private static func stroke(_ stroke: VectorStroke, _ outline: CGPath) -> CAShapeLayer {
        let layer = CAShapeLayer()
        layer.path = outline
        layer.fillColor = nil
        layer.strokeColor = color(stroke.color)
        layer.lineWidth = stroke.width
        layer.miterLimit = VectorStroke.miterLimit
        layer.lineCap = switch stroke.cap ?? .butt {
        case .butt: .butt
        case .round: .round
        case .square: .square
        }
        layer.lineJoin = switch stroke.join ?? .miter {
        case .miter: .miter
        case .round: .round
        case .bevel: .bevel
        }

        return layer
    }

    private static func color(_ rgba: UInt32) -> CGColor {
        CGColor(
            srgbRed: CGFloat((rgba >> 24) & 255) / 255,
            green: CGFloat((rgba >> 16) & 255) / 255,
            blue: CGFloat((rgba >> 8) & 255) / 255,
            alpha: CGFloat(rgba & 255) / 255
        )
    }
}
