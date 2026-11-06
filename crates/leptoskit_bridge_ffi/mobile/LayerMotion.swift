#if canImport(UIKit)
import UIKit
#else
import AppKit
#endif

/// Runs widgets' `transform` and `animation` props on Core Animation, so they play on the render server
/// however busy the guest is.
///
/// The renderer reports each widget it creates or is about to change; after the transaction, `finish`
/// sets every touched layer's transform and adds the animations its changes call for, starting from what
/// the layer showed on screen before the transaction.
@MainActor
final class LayerMotion {
    private static let keyframesKey = "lk.keyframes"
    /// Keyframes are sampled this often, which keeps a spinning widget's steps a few degrees apart.
    private static let samplesPerSecond: CGFloat = 60
    private static let maximumSamples: CGFloat = 600

    /// What a layer showed before this transaction changed it.
    private struct Before {
        let opacity: Float
        let transform: CATransform3D
        let position: CGPoint
        let size: CGSize
        let background: CGColor?
        let border: CGColor?
    }

    /// The keyframes a layer is playing, and the look and size they were sampled from.
    private struct Playing: Equatable {
        let keyframes: WidgetKeyframes
        let opacity: CGFloat
        let transform: WidgetTransform
        let size: CGSize
    }

    private var touched: Set<Int> = []
    private var created: Set<Int> = []
    private var before: [Int: Before] = [:]
    private var playing: [Int: Playing] = [:]

    /// Whether props move or animate at all; most widgets' don't, and cost nothing here.
    static func moves(_ props: WidgetProps?) -> Bool { props?.transform != nil || props?.animation != nil }

    func created(_ id: Int, _ props: WidgetProps) {
        guard Self.moves(props) else { return }
        touched.insert(id)
        created.insert(id)
    }

    /// Call before changing an existing widget's props or frame.
    func willChange(_ id: Int, _ layer: CALayer, from old: WidgetProps?, to new: WidgetProps?) {
        guard Self.moves(old) || Self.moves(new) else { return }
        touched.insert(id)

        guard before[id] == nil, old?.animation != nil || new?.animation != nil, !created.contains(id) else { return }
        let shown = layer.presentation() ?? layer
        before[id] = Before(
            opacity: shown.opacity, transform: shown.transform, position: shown.position, size: shown.bounds.size,
            background: shown.backgroundColor, border: shown.borderColor)
    }

    func forget(_ id: Int) {
        touched.remove(id)
        created.remove(id)
        before[id] = nil
        playing[id] = nil
    }

    func removeAll() {
        touched.removeAll()
        created.removeAll()
        before.removeAll()
        playing.removeAll()
    }

    /// Applies the transaction's transforms and starts its animations.
    func finish(layer: (Int) -> CALayer?, props: (Int) -> WidgetProps?) {
        guard !touched.isEmpty else { return }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        defer {
            CATransaction.commit()
            touched.removeAll()
            created.removeAll()
            before.removeAll()
        }

        for id in touched {
            guard let layer = layer(id), let props = props(id) else { continue }
            let model = Self.matrix(props.transform ?? .identity, in: layer)

            if !CATransform3DEqualToTransform(layer.transform, model) { layer.transform = model }

            if let before = before[id] {
                if let transition = props.animation?.transition { animate(layer, from: before, transition) }
                if let curve = props.animation?.layout { relayout(layer, from: before, curve) }
            }

            if created.contains(id), let entering = props.animation?.entering {
                let own = (props.opacity ?? 1, props.transform ?? .identity)
                let (opacity, transform) = entering.visual.over(opacity: own.0, transform: own.1)
                add(layer, opacity: (opacity, own.0), transform: (transform, own.1), entering, fill: .backwards)
            }
            play(id, layer, props)
        }
    }

    /// Animates a removed widget to its exiting look, calling `done` once it gets there. Returns false,
    /// without calling `done`, when the widget has no exiting animation.
    func exit(_ layer: CALayer, _ props: WidgetProps?, done: @escaping () -> Void) -> Bool {
        guard let presence = props?.animation?.exiting else { return false }
        let own = (props?.opacity ?? 1, props?.transform ?? .identity)
        let (opacity, transform) = presence.visual.over(opacity: own.0, transform: own.1)
        let shown = layer.presentation() ?? layer
        let delay = presence.delayMs / 1000
        // Inside a group, begin times count from the group's start.
        let timing = (curve: presence.curve, delayMs: CGFloat(0))
        let animations = [
            Self.animation("opacity", from: shown.opacity, to: Float(opacity), timing, in: layer),
            Self.animation("transform", from: shown.transform, to: Self.matrix(transform, in: layer), timing, in: layer)
        ].compactMap { $0 }

        guard let longest = animations.map(\.duration).max() else {
            done()

            return true
        }
        layer.removeAnimation(forKey: Self.keyframesKey + ".opacity")
        layer.removeAnimation(forKey: Self.keyframesKey + ".transform")

        for animation in animations {
            animation.beginTime = delay
            animation.fillMode = .both
        }
        let group = CAAnimationGroup()
        group.animations = animations
        group.duration = delay + longest
        group.fillMode = .forwards
        group.isRemovedOnCompletion = false
        group.delegate = MotionCompletion(done)
        layer.add(group, forKey: "lk.exit")

        return true
    }

    /// Forgets an exit when its widget comes back before the animation ends.
    static func cancelExit(_ layer: CALayer) { layer.removeAnimation(forKey: "lk.exit") }

    /// The layer transform for `transform`, about the layer's center whatever its anchor point.
    static func matrix(_ transform: WidgetTransform, in layer: CALayer) -> CATransform3D {
        guard transform != .identity else { return CATransform3DIdentity }
        let size = layer.bounds.size
        // A zero scale has no inverse, and Core Animation interpolates transforms by decomposing them.
        let scale = CATransform3DMakeScale(
            transform.scaleX == 0 ? 0.0001 : transform.scaleX, transform.scaleY == 0 ? 0.0001 : transform.scaleY, 1)
        let rotate = CATransform3DMakeRotation(transform.rotate * .pi / 180, 0, 0, 1)
        let translate = CATransform3DMakeTranslation(
            transform.translateX.resolve(size.width), transform.translateY.resolve(size.height), 0)
        // Scale, then rotate, then move, as CSS applies `translate`, `rotate` and `scale`.
        let matrix = CATransform3DConcat(CATransform3DConcat(scale, rotate), translate)
        let center = CGPoint(
            x: (0.5 - layer.anchorPoint.x) * size.width, y: (0.5 - layer.anchorPoint.y) * size.height)

        guard center != .zero else { return matrix }

        return CATransform3DConcat(
            CATransform3DConcat(CATransform3DMakeTranslation(-center.x, -center.y, 0), matrix),
            CATransform3DMakeTranslation(center.x, center.y, 0))
    }

    private func animate(_ layer: CALayer, from before: Before, _ transition: WidgetTransition) {
        let timing = (curve: transition.curve, delayMs: transition.delayMs)

        func change(_ keyPath: String, from: Any?, to: Any?) {
            if let added = Self.animation(keyPath, from: from, to: to, timing, in: layer) {
                layer.add(added, forKey: "lk.transition." + keyPath)
            }
        }

        if transition.animates(WidgetTransition.opacity), before.opacity != layer.opacity {
            change("opacity", from: before.opacity, to: layer.opacity)
        }

        if transition.animates(WidgetTransition.transform),
           !CATransform3DEqualToTransform(before.transform, layer.transform) {
            change("transform", from: before.transform, to: layer.transform)
        }

        guard transition.animates(WidgetTransition.colors) else { return }

        if before.background != layer.backgroundColor {
            change("backgroundColor", from: before.background, to: layer.backgroundColor)
        }

        if before.border != layer.borderColor { change("borderColor", from: before.border, to: layer.borderColor) }
    }

    private func relayout(_ layer: CALayer, from before: Before, _ curve: WidgetCurve) {
        let timing = (curve: curve, delayMs: CGFloat(0))

        if before.position != layer.position,
           let added = Self.animation("position", from: before.position, to: layer.position, timing, in: layer) {
            layer.add(added, forKey: "lk.layout.position")
        }

        if before.size != layer.bounds.size,
           let added = Self.animation("bounds.size", from: before.size, to: layer.bounds.size, timing, in: layer) {
            layer.add(added, forKey: "lk.layout.size")
        }
    }

    private func add(
        _ layer: CALayer, opacity: (from: CGFloat, to: CGFloat),
        transform: (from: WidgetTransform, to: WidgetTransform), _ presence: WidgetPresence,
        fill: CAMediaTimingFillMode
    ) {
        let timing = (curve: presence.curve, delayMs: presence.delayMs)

        if opacity.from != opacity.to, let added = Self.animation(
            "opacity", from: Float(opacity.from), to: Float(opacity.to), timing, in: layer) {
            added.fillMode = fill
            layer.add(added, forKey: "lk.presence.opacity")
        }

        if transform.from != transform.to, let added = Self.animation(
            "transform", from: Self.matrix(transform.from, in: layer), to: Self.matrix(transform.to, in: layer),
            timing, in: layer) {
            added.fillMode = fill
            layer.add(added, forKey: "lk.presence.transform")
        }
    }

    /// Starts, restarts or stops the layer's keyframes when they, or the look and size they move from, change.
    private func play(_ id: Int, _ layer: CALayer, _ props: WidgetProps) {
        let next = props.animation?.keyframes.map {
            Playing(
                keyframes: $0, opacity: props.opacity ?? 1, transform: props.transform ?? .identity,
                size: layer.bounds.size)
        }

        guard next != playing[id] else { return }
        playing[id] = next
        layer.removeAnimation(forKey: Self.keyframesKey + ".opacity")
        layer.removeAnimation(forKey: Self.keyframesKey + ".transform")

        guard let next, next.keyframes.durationMs > 0 else { return }
        let keyframes = next.keyframes
        let wanted = (keyframes.durationMs / 1000 * Self.samplesPerSecond).rounded(.up)
        let count = Int(min(max(wanted, 2), Self.maximumSamples))
        let samples = (0...count).map {
            keyframes.sample(CGFloat($0) / CGFloat(count), opacity: next.opacity, transform: next.transform)
        }
        let times = (0...count).map { NSNumber(value: Double($0) / Double(count)) }
        var tracks: [(String, [Any])] = []

        if keyframes.animatesOpacity { tracks.append(("opacity", samples.map { Float($0.0) })) }

        if !keyframes.animatedParts.isEmpty {
            tracks.append(("transform", samples.map { NSValue(caTransform3D: Self.matrix($0.1, in: layer)) }))
        }

        for (keyPath, values) in tracks {
            let animation = CAKeyframeAnimation(keyPath: keyPath)
            animation.values = values
            animation.keyTimes = times
            animation.duration = keyframes.durationMs / 1000
            animation.autoreverses = keyframes.alternate

            switch keyframes.iterations {
            // With `autoreverses`, one repeat plays forward and back: two of CSS's alternating passes.
            case let .count(count): animation.repeatCount = Float(keyframes.alternate ? count / 2 : count)
            case .infinite: animation.repeatCount = .infinity
            }

            if keyframes.delayMs > 0 {
                animation.beginTime = layer.convertTime(CACurrentMediaTime(), from: nil) + keyframes.delayMs / 1000
            }
            // Kept through the app moving to the background, which removes finished-on-completion animations.
            animation.isRemovedOnCompletion = false
            layer.add(animation, forKey: Self.keyframesKey + "." + keyPath)
        }
    }

    /// A timing curve's or spring's animation of `keyPath`, or nil when it would take no time.
    private static func animation(
        _ keyPath: String, from: Any?, to: Any?, _ timing: (curve: WidgetCurve, delayMs: CGFloat), in layer: CALayer
    ) -> CABasicAnimation? {
        let animation: CABasicAnimation
        let delayMs = timing.delayMs

        switch timing.curve {
        case let .timing(durationMs, easing):
            guard durationMs > 0 else { return nil }
            animation = CABasicAnimation(keyPath: keyPath)
            animation.duration = durationMs / 1000
            animation.timingFunction = easing.timingFunction
        case let .spring(mass, stiffness, damping):
            let spring = CASpringAnimation(keyPath: keyPath)
            spring.mass = mass
            spring.stiffness = stiffness
            spring.damping = damping
            spring.duration = spring.settlingDuration
            animation = spring
        }
        animation.fromValue = from.map(Self.value)
        animation.toValue = to.map(Self.value)

        if delayMs > 0 {
            animation.beginTime = layer.convertTime(CACurrentMediaTime(), from: nil) + delayMs / 1000
            animation.fillMode = .backwards
        }

        return animation
    }

    private static func value(_ value: Any) -> Any {
        switch value {
        case let transform as CATransform3D: NSValue(caTransform3D: transform)
        #if canImport(UIKit)
        case let point as CGPoint: NSValue(cgPoint: point)
        case let size as CGSize: NSValue(cgSize: size)
        #else
        case let point as CGPoint: NSValue(point: point)
        case let size as CGSize: NSValue(size: size)
        #endif
        default: value
        }
    }
}

extension WidgetEasing {
    var timingFunction: CAMediaTimingFunction {
        switch self {
        case .linear: CAMediaTimingFunction(name: .linear)
        case let .cubicBezier(x1, y1, x2, y2):
            CAMediaTimingFunction(controlPoints: Float(x1), Float(y1), Float(x2), Float(y2))
        }
    }
}

/// Calls back once an animation stops, finished or not.
private final class MotionCompletion: NSObject, CAAnimationDelegate {
    private let done: () -> Void

    init(_ done: @escaping () -> Void) { self.done = done }

    func animationDidStop(_ animation: CAAnimation, finished: Bool) { done() }
}
