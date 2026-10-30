import CoreGraphics

/// The scroll bookkeeping the iOS and macOS renderers share.
///
/// A target is applied only when it differs from the last one its scroll view
/// received, so the person's own scrolling stays put while the app keeps
/// sending the same target. Scroll views that move while a transaction is
/// applied report where they settled afterwards, on the next main-actor turn,
/// so the guest is never re-entered while its own render is applied.
@MainActor
final class ScrollTracker {
    /// The ABI's scroll event kind.
    static let event: UInt32 = 5
    private var targets: [Int: ScrollOffset] = [:]
    private var retargeted: [Int] = []
    private var moved = Set<Int>()
    private var reportScheduled = false

    /// Notes a widget's target when it differs from the last one it received.
    /// Only scroll views move; a target on any other widget is kept and ignored.
    func receive(_ target: ScrollOffset?, for id: Int) {
        guard targets[id] != target else { return }
        targets[id] = target

        if target != nil { retargeted.append(id) }
    }

    /// The targets received since the last call that differ from the ones before.
    func takeChanged() -> [(id: Int, target: ScrollOffset)] {
        defer { retargeted.removeAll() }

        return retargeted.compactMap { id in targets[id].map { (id, $0) } }
    }

    func forget(_ id: Int) {
        targets[id] = nil
        moved.remove(id)
    }

    func removeAll() {
        targets.removeAll()
        retargeted.removeAll()
        moved.removeAll()
    }

    /// Holds back a scroll view's report until the transaction is applied.
    func movedWhileApplying(_ id: Int) { moved.insert(id) }

    /// Reports the scroll views that moved while the transaction was applied.
    func transactionApplied(_ report: @escaping @MainActor (Int) -> Void) {
        guard !moved.isEmpty, !reportScheduled else { return }
        reportScheduled = true
        Task { [weak self] in
            guard let self else { return }
            reportScheduled = false
            let ids = moved.sorted()
            moved.removeAll()
            ids.forEach(report)
        }
    }

    /// A scroll event's payload: the content offset in points.
    static func payload(_ offset: CGPoint) -> String {
        "{\"x\":\(Float(offset.x)),\"y\":\(Float(offset.y))}"
    }

    /// The target kept inside the scrollable range, which neither UIKit nor
    /// AppKit enforces for an offset set in code.
    static func clamp(_ target: ScrollOffset, least: CGPoint, most: CGPoint) -> CGPoint {
        CGPoint(
            x: min(max(target.offsetX, least.x), max(least.x, most.x)),
            y: min(max(target.offsetY, least.y), max(least.y, most.y)))
    }
}
