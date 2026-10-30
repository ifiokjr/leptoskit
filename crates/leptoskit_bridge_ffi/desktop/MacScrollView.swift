import AppKit

/// A scroll view reporting each move of its content. AppKit reflects every
/// change of the clip view's bounds here: scrolling, code, and animation.
final class MacScrollView: NSScrollView {
    var onScroll: (() -> Void)?
    private var reported: CGPoint?

    override func reflectScrolledClipView(_ clipView: NSClipView) {
        super.reflectScrolledClipView(clipView)
        guard clipView === contentView, reported != clipView.bounds.origin else { return }
        reported = clipView.bounds.origin
        onScroll?()
    }

    /// Scrolls to a target, kept inside the document, once the document is sized.
    func scroll(to target: ScrollOffset) {
        guard let document = documentView else { return }
        let visible = contentView.bounds.size
        let point = ScrollTracker.clamp(
            target, least: .zero,
            most: CGPoint(x: document.frame.width - visible.width, y: document.frame.height - visible.height))

        if target.animated == true {
            NSAnimationContext.runAnimationGroup { _ in contentView.animator().setBoundsOrigin(point) }
        } else {
            contentView.scroll(to: point)
            reflectScrolledClipView(contentView)
        }
    }
}
