import AppKit

/// Scroll commands for the AppKit renderer.
///
/// A scroll view is moved only when its command changes, so an unrelated prop
/// change never re-applies an offset the user has since scrolled away from,
/// and the clip view's own bounds changes are reported to the guest as events.
@MainActor
extension MacWidgetRenderer {
    /// Moves a scroll view to the offset its props ask for, once per command.
    func applyScroll(_ command: WidgetScroll?, to widget: NSView, node: Int) {
        guard let command, let scroll = widget as? NSScrollView else { return }

        if scrollCommands[node] == command { return }

        scrollCommands[node] = command
        let origin = NSPoint(x: command.offsetX, y: command.offsetY)

        if command.animated {
            NSAnimationContext.runAnimationGroup { context in
                context.duration = Self.scrollAnimationSeconds
                scroll.contentView.animator().setBoundsOrigin(origin)
            }
        } else {
            scroll.contentView.scroll(to: origin)
            scroll.reflectScrolledClipView(scroll.contentView)
        }
    }

    /// Reports the offset a scroll view scrolled to, in layout units.
    @objc func scrolled(_ notification: Notification) {
        guard let clip = notification.object as? NSClipView,
            let scroll = clip.enclosingScrollView,
            let id = node(scroll)
        else { return }

        let offset = clip.bounds.origin
        event(id, Self.scroll, #"{"x":\#(offset.x),"y":\#(offset.y)}"#)
    }
}
