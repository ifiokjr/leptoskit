import QuartzCore

/// Removed widgets still drawn while their exiting animations play.
///
/// A renderer keeps a widget with an exiting animation on screen when a transaction removes it, after
/// the children it places, so their indices stay right. The exit starts once the transaction ends,
/// unless the transaction puts the widget back first.
@MainActor
final class ExitingViews<View: AnyObject> {
    private let superview: (View) -> View?
    private let layer: (View) -> CALayer?
    private let remove: (View) -> Void
    private var ghosts: [ObjectIdentifier: View] = [:]
    private var leaving: [(View, WidgetProps?)] = []

    init(superview: @escaping (View) -> View?, layer: @escaping (View) -> CALayer?, remove: @escaping (View) -> Void) {
        self.superview = superview
        self.layer = layer
        self.remove = remove
    }

    func contains(_ view: View) -> Bool { ghosts[ObjectIdentifier(view)] != nil }

    /// Keeps `view` drawn after its removal, to animate out after the transaction.
    func keep(_ view: View, _ props: WidgetProps?) {
        ghosts[ObjectIdentifier(view)] = view
        leaving.append((view, props))
    }

    /// Forgets `view`'s exit when it comes back; returns whether it was exiting.
    func release(_ view: View) -> Bool {
        guard ghosts.removeValue(forKey: ObjectIdentifier(view)) != nil else { return false }

        if let layer = layer(view) { LayerMotion.cancelExit(layer) }

        return true
    }

    /// Whether `view` is exiting, or inside a view that is; such a view goes when the exit ends.
    func holds(_ view: View) -> Bool {
        guard !ghosts.isEmpty else { return false }
        var current: View? = view

        while let candidate = current {
            if contains(candidate) { return true }
            current = superview(candidate)
        }

        return false
    }

    /// Takes away the exiting views drawn inside `parent`, which is going away itself.
    func drop(in parent: View) {
        for (key, ghost) in ghosts where superview(ghost) === parent {
            ghosts[key] = nil
            remove(ghost)
        }
    }

    /// Animates out the views this transaction removed and did not put back.
    func start(_ motion: LayerMotion) {
        let removed = leaving
        leaving.removeAll()

        for (view, props) in removed where ghosts[ObjectIdentifier(view)] === view {
            guard let layer = layer(view) else { continue }
            _ = motion.exit(layer, props) { [weak self, weak view] in
                guard let self, let view, ghosts.removeValue(forKey: ObjectIdentifier(view)) != nil else { return }
                remove(view)
            }
        }
    }

    func removeAll() {
        ghosts.removeAll()
        leaving.removeAll()
    }
}
