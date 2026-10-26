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
///
/// Every field but the view box is optional on the wire and decodes to its default; an unknown keyword or an
/// invalid number fails the transaction decode. Drawings compare by value, so an unchanged drawing sent again
/// repaints nothing.
struct VectorDrawing: Decodable, Equatable {
    struct ViewBox: Decodable, Equatable {
        let minX: CGFloat
        let minY: CGFloat
        let width: CGFloat
        let height: CGFloat
    }

    /// How the view box maps onto the frame: `fit` and `fill` scale uniformly to show all of it or to cover the
    /// frame, as SVG's `meet` and `slice`; `stretch` scales each axis on its own and ignores alignment.
    enum Scaling: String, Decodable { case fit, fill, stretch }

    enum CodingKeys: String, CodingKey { case viewBox, scaling, align, paths }
    let viewBox: ViewBox
    let scaling: Scaling
    let align: VectorAlignment
    let paths: [VectorPath]

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
        scaling = try container.decodeIfPresent(Scaling.self, forKey: .scaling) ?? .fit
        align = try container.decodeIfPresent(VectorAlignment.self, forKey: .align) ?? .center
        paths = try container.decodeIfPresent([VectorPath].self, forKey: .paths) ?? []
    }

    /// Maps view-box coordinates onto a frame whose origin is its top-left corner.
    func transform(for size: CGSize) -> CGAffineTransform {
        let width = max(size.width, 0)
        let height = max(size.height, 0)
        let scaleX = width / viewBox.width
        let scaleY = height / viewBox.height

        if scaling == .stretch {
            return CGAffineTransform(
                a: scaleX, b: 0, c: 0, d: scaleY, tx: -viewBox.minX * scaleX, ty: -viewBox.minY * scaleY
            )
        }

        let scale = scaling == .fit ? min(scaleX, scaleY) : max(scaleX, scaleY)

        return CGAffineTransform(
            a: scale, b: 0, c: 0, d: scale,
            tx: (width - viewBox.width * scale) * align.horizontal.factor - viewBox.minX * scale,
            ty: (height - viewBox.height * scale) * align.vertical.factor - viewBox.minY * scale
        )
    }
}

/// Where a fitted or filled drawing sits in its frame; the wire omits it when the drawing is centred.
struct VectorAlignment: Decodable, Equatable {
    /// Where the drawing sits along one axis.
    enum Align: String, Decodable {
        case start, center, end

        /// The share of the spare space placed before the drawing.
        var factor: CGFloat {
            switch self {
            case .start: 0
            case .center: 0.5
            case .end: 1
            }
        }
    }

    static let center = Self(horizontal: .center, vertical: .center)
    enum CodingKeys: String, CodingKey {
        case horizontal = "x"
        case vertical = "y"
    }

    let horizontal: Align
    let vertical: Align
}

/// One outline painted with an optional fill, then an optional stroke.
///
/// Path data arrives canonical (absolute `M`, `L`, `Q`, `C` and `Z`, one space between tokens),
/// so it splits on spaces instead of parsing SVG. Malformed data fails the transaction decode.
struct VectorPath: Decodable, Equatable {
    /// Which regions of a self-intersecting or nested outline the fill covers: `nonzero`, SVG's default, or
    /// `evenodd`, under which nested outlines punch holes whatever their direction.
    enum FillRule: String, Decodable { case nonzero, evenodd }

    private static let pointCounts: [Substring: Int] = ["M": 1, "L": 1, "Q": 2, "C": 3, "Z": 0]
    enum CodingKeys: String, CodingKey { case data, fill, fillRule, stroke }
    /// The outline in view-box coordinates.
    let outline: CGPath
    let fill: UInt32?
    let fillRule: FillRule
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
        fillRule = try container.decodeIfPresent(FillRule.self, forKey: .fillRule) ?? .nonzero
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

/// How an outline is stroked: in layout units when fixed, in view-box units when it scales with the drawing.
struct VectorStroke: Decodable, Equatable {
    enum Cap: String, Decodable { case butt, round, square }
    enum Join: String, Decodable { case miter, round, bevel }
    /// Whether the width and dashes follow the drawing's scale.
    enum Scaling: String, Decodable { case fixed, drawing }
    enum CodingKeys: String, CodingKey { case color, width, cap, join, miterLimit, dashes, dashOffset, scaling }
    let color: UInt32
    let width: CGFloat
    let cap: Cap
    let join: Join
    /// The ratio of miter length to line width beyond which a miter join is beveled; SVG's default is 4.
    let miterLimit: CGFloat
    /// Alternating dash and gap lengths: empty for a solid line, else an even count.
    let dashes: [CGFloat]
    /// How far into the dash pattern the stroke starts.
    let dashOffset: CGFloat
    let scaling: Scaling

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        color = try container.decode(UInt32.self, forKey: .color)
        width = try container.decode(CGFloat.self, forKey: .width)
        cap = try container.decodeIfPresent(Cap.self, forKey: .cap) ?? .butt
        join = try container.decodeIfPresent(Join.self, forKey: .join) ?? .miter
        miterLimit = try container.decodeIfPresent(CGFloat.self, forKey: .miterLimit) ?? 4
        scaling = try container.decodeIfPresent(Scaling.self, forKey: .scaling) ?? .fixed
        let dashes = try container.decodeIfPresent([CGFloat].self, forKey: .dashes) ?? []
        let dashOffset = try container.decodeIfPresent(CGFloat.self, forKey: .dashOffset) ?? 0

        guard width.isFinite, width > 0, miterLimit.isFinite, miterLimit >= 1, dashOffset.isFinite,
              dashes.allSatisfy({ $0.isFinite && $0 >= 0 }) else {
            throw DecodingError.dataCorrupted(DecodingError.Context(
                codingPath: container.codingPath,
                debugDescription: "A stroke needs a positive width, a miter limit of at least 1, "
                    + "and finite, non-negative dashes"
            ))
        }

        // As the core's `Stroke::dashes`: an all-zero list is solid, and an odd one repeats to make it even.
        self.dashes = if dashes.allSatisfy({ $0 == 0 }) {
            []
        } else if dashes.count.isMultiple(of: 2) {
            dashes
        } else {
            dashes + dashes
        }
        self.dashOffset = self.dashes.isEmpty ? 0 : dashOffset
    }

    /// The factor from this stroke's units to frame units under `transform`: one for a fixed stroke, and the
    /// square root of the area scale for one that scales, which is the scale itself whenever it is uniform.
    func frameScale(_ transform: CGAffineTransform) -> CGFloat {
        switch scaling {
        case .fixed: 1
        case .drawing: abs(transform.a * transform.d - transform.b * transform.c).squareRoot()
        }
    }
}

/// Paints a drawing with one shape layer per paint: each path's fill, then its stroke, in order.
///
/// Outline points are mapped onto the frame before stroking, so stroke widths are set in frame units: as given
/// for a fixed stroke, and multiplied by the drawing's scale for one that scales. A changed drawing rebuilds
/// the layers; a new size or scale remaps the same layers; an unchanged drawing at an unchanged size does
/// nothing.
@MainActor
struct VectorLayers {
    private struct Geometry: Equatable {
        let size: CGSize
        let scale: CGFloat
    }

    /// One path and the layers that paint it.
    private struct Painted {
        let path: VectorPath
        let fill: CAShapeLayer?
        let stroke: CAShapeLayer?
    }

    /// The drawing the layers show.
    private(set) var drawing: VectorDrawing?
    private var painted: [Painted] = []
    private var geometry: Geometry?

    /// Shows `drawing` under `host` at `host`'s size and scale, redoing only the work that changed.
    mutating func paint(_ drawing: VectorDrawing?, in host: CALayer) {
        let geometry = Geometry(size: host.bounds.size, scale: host.contentsScale)
        let rebuild = drawing != self.drawing
        guard rebuild || geometry != self.geometry else { return }

        CATransaction.begin()
        CATransaction.setDisableActions(true)
        defer { CATransaction.commit() }

        if rebuild {
            for item in painted {
                item.fill?.removeFromSuperlayer()
                item.stroke?.removeFromSuperlayer()
            }
            painted = (drawing?.paths ?? []).map { path in
                Painted(
                    path: path,
                    fill: path.fill.map { Self.fill($0, path.fillRule) },
                    stroke: path.stroke.map(Self.stroke)
                )
            }
            for layer in painted.flatMap({ [$0.fill, $0.stroke] }).compactMap(\.self) {
                host.addSublayer(layer)
            }
            self.drawing = drawing
        }

        self.geometry = geometry
        guard let drawing else { return }

        var transform = drawing.transform(for: geometry.size)

        for item in painted {
            let outline = item.path.outline.copy(using: &transform)

            for layer in [item.fill, item.stroke].compactMap(\.self) {
                layer.frame = host.bounds
                layer.contentsScale = geometry.scale
                layer.path = outline
            }

            if let stroke = item.path.stroke, let layer = item.stroke {
                Self.size(layer, for: stroke, by: stroke.frameScale(transform))
            }
        }
    }

    private static func fill(_ rgba: UInt32, _ rule: VectorPath.FillRule) -> CAShapeLayer {
        let layer = CAShapeLayer()
        layer.fillRule = switch rule {
        case .nonzero: .nonZero
        case .evenodd: .evenOdd
        }
        layer.fillColor = color(rgba)
        layer.strokeColor = nil

        return layer
    }

    private static func stroke(_ stroke: VectorStroke) -> CAShapeLayer {
        let layer = CAShapeLayer()
        layer.fillColor = nil
        layer.strokeColor = color(stroke.color)
        layer.miterLimit = stroke.miterLimit
        layer.lineCap = switch stroke.cap {
        case .butt: .butt
        case .round: .round
        case .square: .square
        }
        layer.lineJoin = switch stroke.join {
        case .miter: .miter
        case .round: .round
        case .bevel: .bevel
        }

        return layer
    }

    /// Sets the width and dashes in frame units, `factor` times the stroke's own.
    private static func size(_ layer: CAShapeLayer, for stroke: VectorStroke, by factor: CGFloat) {
        let dashes = stroke.dashes.map { $0 * factor }
        layer.lineWidth = stroke.width * factor
        // A collapsed frame scales every dash to zero, which is no pattern at all.
        layer.lineDashPattern = dashes.contains { $0 > 0 } ? dashes.map { NSNumber(value: Double($0)) } : nil
        layer.lineDashPhase = stroke.dashOffset * factor
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
