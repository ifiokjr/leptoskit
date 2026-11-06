import CoreGraphics
import Foundation
import LeptosKitBridge

/// Reads `lk_app_render_binary`'s transaction into the `Transaction` the JSON form decodes to.
///
/// `JSONDecoder` took 30 ms to decode the 6,000 mutations that create 1,000 cards; this reads them with
/// fixed-width loads. The layout is documented in the Rust bridge's `binary` module: typed fields for the
/// properties every view uses, and the JSON form for any other property that is set.
extension Transaction {
    init(binary bytes: UnsafeRawBufferPointer) throws {
        var reader = BinaryReader(bytes)
        guard try reader.take(4).elementsEqual("LKB1".utf8) else { throw reader.malformed }
        let root = try reader.int64()
        let count = try reader.uint32()
        var mutations: [Change] = []
        mutations.reserveCapacity(Int(count))

        for _ in 0..<count {
            mutations.append(try reader.change())
        }

        guard reader.atEnd else { throw reader.malformed }
        self.init(version: 1, root: root < 0 ? nil : Int(root), mutations: mutations)
    }
}

private extension Change {
    /// A change carrying one mutation.
    static func with(
        create: Widget? = nil, updateProps: Widget? = nil, updateLayout: WidgetLayout? = nil,
        insertChild: Edge? = nil, removeChild: Edge? = nil, delete: Removed? = nil
    ) -> Change {
        Change(
            create: create, updateProps: updateProps, updateLayout: updateLayout,
            insertChild: insertChild, removeChild: removeChild, delete: delete)
    }
}

/// The properties sent in the JSON form, decoded as the JSON transaction decodes them.
private struct WidgetExtras: Decodable {
    let autocomplete: String?
    let checked: Bool?
    let slider: WidgetSlider?
    let progress: WidgetProgress?
    let running: Bool?
    let focused: Bool?
    let drawing: VectorDrawing?
    let aria: WidgetAria?
    let scrollOffset: ScrollOffset?
}

/// A cursor over one binary transaction; every read checks it stays in bounds.
private struct BinaryReader {
    private static let fontWeights = ["regular", "medium", "semibold", "bold"]
    private static let textAligns = ["start", "center", "end"]
    private let bytes: UnsafeRawBufferPointer
    private var offset = 0
    var atEnd: Bool { offset == bytes.count }
    var malformed: NativeAppError { NativeAppError(message: "Malformed render transaction at byte \(offset)") }

    init(_ bytes: UnsafeRawBufferPointer) { self.bytes = bytes }

    mutating func take(_ count: Int) throws -> UnsafeRawBufferPointer {
        guard count >= 0, count <= bytes.count - offset else { throw malformed }
        defer { offset += count }

        return UnsafeRawBufferPointer(rebasing: bytes[offset..<offset + count])
    }

    mutating func uint8() throws -> UInt8 { try take(1)[0] }

    mutating func uint32() throws -> UInt32 {
        UInt32(littleEndian: try take(4).loadUnaligned(as: UInt32.self))
    }

    mutating func int64() throws -> Int64 {
        Int64(littleEndian: try take(8).loadUnaligned(as: Int64.self))
    }

    mutating func id() throws -> Int { Int(try uint32()) }

    mutating func float() throws -> CGFloat { CGFloat(Float(bitPattern: try uint32())) }

    mutating func string() throws -> String { try utf8(take(Int(try uint32()))) }

    private func utf8(_ bytes: UnsafeRawBufferPointer) throws -> String {
        guard let text = String(bytes: bytes, encoding: .utf8) else { throw malformed }

        return text
    }

    mutating func change() throws -> Change {
        switch try uint8() {
        case 1:
            let id = try id()
            let kind = try utf8(take(Int(try uint8())))
            let widget = Change.Widget(id: id, kind: kind, props: try props())

            return .with(create: widget)
        case 2:
            let widget = Change.Widget(id: try id(), kind: nil, props: try props())

            return .with(updateProps: widget)
        case 3:
            let layout = WidgetLayout(
                id: try id(), offsetX: try float(), offsetY: try float(), width: try float(), height: try float())

            return .with(updateLayout: layout)
        case 4:
            let edge = Change.Edge(parent: try id(), child: try id(), index: try id())

            return .with(insertChild: edge)
        case 5:
            let edge = Change.Edge(parent: try id(), child: try id(), index: nil)

            return .with(removeChild: edge)
        case 6:
            let removed = Change.Removed(id: try id())

            return .with(delete: removed)
        default:
            throw malformed
        }
    }

    mutating func props() throws -> WidgetProps {
        let present = try uint32()
        func has(_ bit: UInt32) -> Bool { present & (1 << bit) != 0 }

        let text = has(0) ? try string() : nil
        let source = has(1) ? try string() : nil
        let placeholder = has(2) ? try string() : nil
        let value = has(3) ? try string() : nil
        let accessibilityLabel = has(4) ? try string() : nil
        let color = has(5) ? try uint32() : nil
        let background = has(6) ? try uint32() : nil
        let borderColor = has(7) ? try uint32() : nil

        let enabled = has(8) ? try uint8() != 0 : nil
        let hidden = has(9) ? try uint8() != 0 : nil
        let clip = has(10) ? try uint8() != 0 : nil
        let cornerRadius = has(11) ? try float() : nil
        let borderWidth = has(12) ? try float() : nil
        let opacity = has(13) ? try float() : nil
        let fontSize = has(14) ? try float() : nil
        let fontWeight = has(15) ? try choice(Self.fontWeights) : nil
        let textAlign = has(16) ? try choice(Self.textAligns) : nil
        let transform = has(17) ? try self.transform() : nil
        let animation = has(18) ? try self.animation() : nil
        let extras = has(31) ? try extras() : nil

        return WidgetProps(
            text: text, source: source, placeholder: placeholder, autocomplete: extras?.autocomplete, value: value,
            checked: extras?.checked, slider: extras?.slider, progress: extras?.progress,
            running: extras?.running, focused: extras?.focused,
            accessibilityLabel: accessibilityLabel, color: color, background: background,
            enabled: enabled, hidden: hidden, clip: clip, cornerRadius: cornerRadius,
            borderWidth: borderWidth, borderColor: borderColor, opacity: opacity,
            fontSize: fontSize, fontWeight: fontWeight, textAlign: textAlign,
            drawing: extras?.drawing, aria: extras?.aria, scrollOffset: extras?.scrollOffset,
            transform: transform, animation: animation)
    }

    private mutating func transform() throws -> WidgetTransform {
        WidgetTransform(
            translateX: WidgetOffset(points: try float(), percent: try float()),
            translateY: WidgetOffset(points: try float(), percent: try float()),
            scaleX: try float(), scaleY: try float(), rotate: try float())
    }

    private mutating func animation() throws -> WidgetAnimation {
        let parts = try uint8()
        func has(_ bit: UInt8) -> Bool { parts & (1 << bit) != 0 }

        return WidgetAnimation(
            transition: has(0) ? try transition() : nil, keyframes: has(1) ? try keyframes() : nil,
            entering: has(2) ? try presence() : nil, exiting: has(3) ? try presence() : nil,
            layout: has(4) ? try curve() : nil)
    }

    private mutating func transition() throws -> WidgetTransition {
        let properties = try uint8()
        let delayMs = try float()

        return WidgetTransition(properties: properties, curve: try curve(), delayMs: delayMs)
    }

    private mutating func keyframes() throws -> WidgetKeyframes {
        let durationMs = try float()
        let delayMs = try float()
        let iterations = try float()
        let alternate = try uint8() != 0
        let easing = try self.easing()
        var frames: [WidgetKeyframe] = []

        for _ in 0..<(try uint8()) {
            let offset = try float()
            let visual = try self.visual()
            let easing = try uint8() != 0 ? try self.easing() : nil
            frames.append(WidgetKeyframe(offset: offset, visual: visual, easing: easing))
        }

        return WidgetKeyframes(
            frames: frames, durationMs: durationMs, delayMs: delayMs,
            iterations: iterations.isInfinite ? .infinite : .count(iterations), alternate: alternate, easing: easing)
    }

    private mutating func presence() throws -> WidgetPresence {
        let visual = try self.visual()
        let delayMs = try float()

        return WidgetPresence(visual: visual, curve: try curve(), delayMs: delayMs)
    }

    private mutating func visual() throws -> WidgetVisual {
        let present = try uint8()

        return WidgetVisual(
            opacity: present & 1 != 0 ? try float() : nil, transform: present & 2 != 0 ? try transform() : nil)
    }

    private mutating func curve() throws -> WidgetCurve {
        switch try uint8() {
        case 0: return .timing(durationMs: try float(), easing: try easing())
        case 1: return .spring(mass: try float(), stiffness: try float(), damping: try float())
        default: throw malformed
        }
    }

    private mutating func easing() throws -> WidgetEasing {
        switch try uint8() {
        case 0: return .linear
        case 1: return .cubicBezier(try float(), try float(), try float(), try float())
        default: throw malformed
        }
    }

    private mutating func choice(_ names: [String]) throws -> String {
        let index = Int(try uint8())

        guard index < names.count else { throw malformed }

        return names[index]
    }

    private mutating func extras() throws -> WidgetExtras {
        let json = try take(Int(try uint32()))
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase

        return try decoder.decode(WidgetExtras.self, from: Data(json))
    }
}
