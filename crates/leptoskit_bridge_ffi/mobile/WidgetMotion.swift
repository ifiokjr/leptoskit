import CoreGraphics

// The `transform` and `animation` props, as `leptoskit_animation` describes them. The guest has already
// sanitized every value: finite, bounded, and at most sixteen keyframes.

/// A distance: points plus a percentage of the widget's own size along the same axis.
struct WidgetOffset: Equatable {
    var points: CGFloat = 0
    var percent: CGFloat = 0

    func resolve(_ length: CGFloat) -> CGFloat { points + percent / 100 * length }
}

/// Moves, scales and rotates a widget's drawing about its center, as CSS's `translate`, `scale` and `rotate`.
struct WidgetTransform: Equatable {
    static let identity = Self()
    var translateX = WidgetOffset()
    var translateY = WidgetOffset()
    var scaleX: CGFloat = 1
    var scaleY: CGFloat = 1
    /// Clockwise, in degrees.
    var rotate: CGFloat = 0

    /// The parts that differ from no transform.
    var parts: WidgetTransformParts {
        var parts: WidgetTransformParts = []
        if translateX != WidgetOffset() || translateY != WidgetOffset() { parts.insert(.translate) }
        if scaleX != 1 || scaleY != 1 { parts.insert(.scale) }
        if rotate != 0 { parts.insert(.rotate) }

        return parts
    }

    /// This transform with `parts` taken from `other`, as one CSS transform property overrides another.
    func with(_ other: Self, parts: WidgetTransformParts) -> Self {
        var result = self
        if parts.contains(.translate) { (result.translateX, result.translateY) = (other.translateX, other.translateY) }
        if parts.contains(.scale) { (result.scaleX, result.scaleY) = (other.scaleX, other.scaleY) }
        if parts.contains(.rotate) { result.rotate = other.rotate }

        return result
    }

    func mix(_ other: Self, _ progress: CGFloat) -> Self {
        func mix(_ from: CGFloat, _ to: CGFloat) -> CGFloat { from + (to - from) * progress }

        return Self(
            translateX: WidgetOffset(
                points: mix(translateX.points, other.translateX.points),
                percent: mix(translateX.percent, other.translateX.percent)),
            translateY: WidgetOffset(
                points: mix(translateY.points, other.translateY.points),
                percent: mix(translateY.percent, other.translateY.percent)),
            scaleX: mix(scaleX, other.scaleX), scaleY: mix(scaleY, other.scaleY), rotate: mix(rotate, other.rotate))
    }
}

struct WidgetTransformParts: OptionSet {
    static let translate = Self(rawValue: 1)
    static let scale = Self(rawValue: 2)
    static let rotate = Self(rawValue: 4)
    let rawValue: UInt8
}

/// Opacity and transform, each unset when it stays at the widget's own value.
struct WidgetVisual: Equatable {
    var opacity: CGFloat?
    var transform: WidgetTransform?

    /// The widget's own look with this visual's set values over it.
    func over(opacity own: CGFloat, transform base: WidgetTransform) -> (CGFloat, WidgetTransform) {
        (opacity ?? own, transform.map { base.with($0, parts: $0.parts) } ?? base)
    }
}

enum WidgetEasing: Equatable {
    case linear
    case cubicBezier(CGFloat, CGFloat, CGFloat, CGFloat)

    /// The eased progress at `time`, from 0 to 1, as CSS's `cubic-bezier()`.
    func sample(_ time: CGFloat) -> CGFloat {
        guard case let .cubicBezier(x1, y1, x2, y2) = self else { return time }
        let time = min(max(time, 0), 1)

        func bezier(_ along: CGFloat, _ first: CGFloat, _ second: CGFloat) -> CGFloat {
            let rest = 1 - along

            return 3 * rest * rest * along * first + 3 * rest * along * along * second + along * along * along
        }

        // The curve's x rises monotonically, so bisection finds the point along it at `time`.
        var (low, high, along) = (CGFloat(0), CGFloat(1), time)
        for _ in 0..<24 {
            if bezier(along, x1, x2) < time { low = along } else { high = along }
            along = (low + high) / 2
        }

        return bezier(along, y1, y2)
    }
}

enum WidgetCurve: Equatable {
    case timing(durationMs: CGFloat, easing: WidgetEasing)
    case spring(mass: CGFloat, stiffness: CGFloat, damping: CGFloat)
}

/// Which property changes animate: bit 1 opacity, 2 transform, 4 colors.
struct WidgetTransition: Equatable {
    static let opacity: UInt8 = 1
    static let transform: UInt8 = 2
    static let colors: UInt8 = 4
    var properties: UInt8
    var curve: WidgetCurve
    var delayMs: CGFloat = 0

    func animates(_ property: UInt8) -> Bool { properties & property != 0 }
}

enum WidgetIterations: Equatable {
    case count(CGFloat)
    case infinite
}

struct WidgetKeyframe: Equatable {
    var offset: CGFloat
    var visual: WidgetVisual
    var easing: WidgetEasing?
}

/// A keyframe sequence, as CSS `@keyframes` with `animation`.
struct WidgetKeyframes: Equatable {
    var frames: [WidgetKeyframe]
    var durationMs: CGFloat
    var delayMs: CGFloat = 0
    var iterations: WidgetIterations
    var alternate = false
    var easing: WidgetEasing

    var animatesOpacity: Bool { frames.contains { $0.visual.opacity != nil } }

    /// The transform parts any keyframe sets; the others stay at the widget's own value.
    var animatedParts: WidgetTransformParts {
        frames.compactMap(\.visual.transform).reduce([]) { $0.union($1.parts) }
    }

    /// The look at `progress` through one pass, from the widget's own look, as `Keyframes::sample` in Rust.
    func sample(
        _ progress: CGFloat, opacity own: CGFloat, transform base: WidgetTransform
    ) -> (CGFloat, WidgetTransform) {
        let opacity = track(progress, own, \.opacity) { $0 + ($1 - $0) * $2 }
        let tracked = track(progress, base, \.transform) { $0.mix($1, $2) }

        return (opacity, base.with(tracked, parts: animatedParts))
    }

    /// One property's value, between the keyframes that set it; the widget's own value fills both ends.
    private func track<Value>(
        _ progress: CGFloat, _ own: Value, _ read: (WidgetVisual) -> Value?, mix: (Value, Value, CGFloat) -> Value
    ) -> Value {
        var previous = (offset: CGFloat(0), value: own, easing: easing)

        for frame in frames {
            guard let value = read(frame.visual) else { continue }

            if frame.offset >= progress {
                let span = frame.offset - previous.offset
                guard span > 0 else { return value }

                return mix(previous.value, value, previous.easing.sample((progress - previous.offset) / span))
            }
            previous = (frame.offset, value, frame.easing ?? easing)
        }

        let span = 1 - previous.offset
        guard span > 0 else { return previous.value }

        return mix(previous.value, own, previous.easing.sample((progress - previous.offset) / span))
    }
}

/// How a widget enters when added or exits when removed: from or to `visual`.
struct WidgetPresence: Equatable {
    var visual: WidgetVisual
    var curve: WidgetCurve
    var delayMs: CGFloat = 0
}

struct WidgetAnimation: Equatable {
    var transition: WidgetTransition?
    var keyframes: WidgetKeyframes?
    var entering: WidgetPresence?
    var exiting: WidgetPresence?
    /// Animates position and size changes after layout.
    var layout: WidgetCurve?
}

// MARK: - JSON, with `convertFromSnakeCase` keys and serde's defaults for omitted fields.

private enum MotionKeys: String, CodingKey {
    case points, percent, translateX, translateY, scaleX, scaleY, rotate, opacity, transform
    case durationMs, easing, timing, spring, mass, stiffness, damping, properties, curve, delayMs
    case count, offset, frames, iterations, alternate, visual, transition, keyframes, entering, exiting, layout
    case cubicBezier
}

extension WidgetOffset: Decodable {
    init(from decoder: Decoder) throws {
        let keys = try decoder.container(keyedBy: MotionKeys.self)
        self.init(
            points: try keys.decodeIfPresent(CGFloat.self, forKey: .points) ?? 0,
            percent: try keys.decodeIfPresent(CGFloat.self, forKey: .percent) ?? 0)
    }
}

extension WidgetTransform: Decodable {
    init(from decoder: Decoder) throws {
        let keys = try decoder.container(keyedBy: MotionKeys.self)
        self.init(
            translateX: try keys.decodeIfPresent(WidgetOffset.self, forKey: .translateX) ?? WidgetOffset(),
            translateY: try keys.decodeIfPresent(WidgetOffset.self, forKey: .translateY) ?? WidgetOffset(),
            scaleX: try keys.decodeIfPresent(CGFloat.self, forKey: .scaleX) ?? 1,
            scaleY: try keys.decodeIfPresent(CGFloat.self, forKey: .scaleY) ?? 1,
            rotate: try keys.decodeIfPresent(CGFloat.self, forKey: .rotate) ?? 0)
    }
}

extension WidgetVisual: Decodable {
    init(from decoder: Decoder) throws {
        let keys = try decoder.container(keyedBy: MotionKeys.self)
        self.init(
            opacity: try keys.decodeIfPresent(CGFloat.self, forKey: .opacity),
            transform: try keys.decodeIfPresent(WidgetTransform.self, forKey: .transform))
    }
}

/// `"linear"` or `{"cubic_bezier": [x1, y1, x2, y2]}`.
extension WidgetEasing: Decodable {
    init(from decoder: Decoder) throws {
        if let name = try? decoder.singleValueContainer().decode(String.self), name == "linear" {
            self = .linear

            return
        }
        let keys = try decoder.container(keyedBy: MotionKeys.self)
        let points = try keys.decode([CGFloat].self, forKey: .cubicBezier)
        guard points.count == 4 else {
            throw DecodingError.dataCorruptedError(
                forKey: .cubicBezier, in: keys, debugDescription: "A cubic Bézier has four numbers")
        }
        self = .cubicBezier(points[0], points[1], points[2], points[3])
    }
}

/// `{"timing": {...}}` or `{"spring": {...}}`.
extension WidgetCurve: Decodable {
    init(from decoder: Decoder) throws {
        let keys = try decoder.container(keyedBy: MotionKeys.self)
        if keys.contains(.spring) {
            let spring = try keys.nestedContainer(keyedBy: MotionKeys.self, forKey: .spring)
            self = .spring(
                mass: try spring.decode(CGFloat.self, forKey: .mass),
                stiffness: try spring.decode(CGFloat.self, forKey: .stiffness),
                damping: try spring.decode(CGFloat.self, forKey: .damping))
        } else {
            let timing = try keys.nestedContainer(keyedBy: MotionKeys.self, forKey: .timing)
            self = .timing(
                durationMs: try timing.decode(CGFloat.self, forKey: .durationMs),
                easing: try timing.decode(WidgetEasing.self, forKey: .easing))
        }
    }
}

extension WidgetTransition: Decodable {
    init(from decoder: Decoder) throws {
        let keys = try decoder.container(keyedBy: MotionKeys.self)
        self.init(
            properties: try keys.decode(UInt8.self, forKey: .properties),
            curve: try keys.decode(WidgetCurve.self, forKey: .curve),
            delayMs: try keys.decodeIfPresent(CGFloat.self, forKey: .delayMs) ?? 0)
    }
}

/// `"infinite"` or `{"count": n}`.
extension WidgetIterations: Decodable {
    init(from decoder: Decoder) throws {
        if let name = try? decoder.singleValueContainer().decode(String.self), name == "infinite" {
            self = .infinite

            return
        }
        self = .count(try decoder.container(keyedBy: MotionKeys.self).decode(CGFloat.self, forKey: .count))
    }
}

/// A keyframe's visual is flattened into it: `{"offset", "opacity"?, "transform"?, "easing"?}`.
extension WidgetKeyframe: Decodable {
    init(from decoder: Decoder) throws {
        let keys = try decoder.container(keyedBy: MotionKeys.self)
        self.init(
            offset: try keys.decode(CGFloat.self, forKey: .offset),
            visual: try WidgetVisual(from: decoder),
            easing: try keys.decodeIfPresent(WidgetEasing.self, forKey: .easing))
    }
}

extension WidgetKeyframes: Decodable {
    init(from decoder: Decoder) throws {
        let keys = try decoder.container(keyedBy: MotionKeys.self)
        self.init(
            frames: try keys.decode([WidgetKeyframe].self, forKey: .frames),
            durationMs: try keys.decode(CGFloat.self, forKey: .durationMs),
            delayMs: try keys.decodeIfPresent(CGFloat.self, forKey: .delayMs) ?? 0,
            iterations: try keys.decode(WidgetIterations.self, forKey: .iterations),
            alternate: try keys.decodeIfPresent(Bool.self, forKey: .alternate) ?? false,
            easing: try keys.decode(WidgetEasing.self, forKey: .easing))
    }
}

extension WidgetPresence: Decodable {
    init(from decoder: Decoder) throws {
        let keys = try decoder.container(keyedBy: MotionKeys.self)
        self.init(
            visual: try keys.decode(WidgetVisual.self, forKey: .visual),
            curve: try keys.decode(WidgetCurve.self, forKey: .curve),
            delayMs: try keys.decodeIfPresent(CGFloat.self, forKey: .delayMs) ?? 0)
    }
}

extension WidgetAnimation: Decodable {}
