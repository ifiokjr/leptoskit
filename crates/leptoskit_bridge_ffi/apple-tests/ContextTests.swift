import LeptosKitBridge
import Testing

#if canImport(AppKit)
import AppKit

@Test
@MainActor
func contextDoesNotLoadOrRetainItsOwner() throws {
    var controller: NSViewController? = NSViewController()
    weak var weakOwner: NSViewController?
    weakOwner = controller
    let context = NativePluginContext(owner: try #require(controller))
    #expect(throws: PluginError.unavailable) { try context.requirePresentationOwner() }
    #expect(controller?.isViewLoaded == false)
    controller = nil
    #expect(weakOwner == nil)
    #expect(throws: PluginError.unavailable) { try context.requirePresentationOwner() }
}

@Test
@MainActor
func contextUsesOnlyItsOwnVisibleWindowAndInvalidatesOnClose() throws {
    _ = NSApplication.shared
    let controller = NSViewController()
    controller.view = NSView(frame: NSRect(x: 0, y: 0, width: 100, height: 100))
    let context = NativePluginContext(owner: controller)
    let window = NSWindow(contentViewController: controller)
    window.isReleasedWhenClosed = false
    defer {
        window.orderOut(nil)
        window.contentViewController = nil
    }
    #expect(throws: PluginError.unavailable) { try context.requirePresentationOwner() }
    window.orderFront(nil)
    #expect(try context.requirePresentationOwner() === controller)

    let detached = NSViewController()
    detached.view = NSView()
    let other = NativePluginContext(owner: detached)
    #expect(throws: PluginError.unavailable) { try other.requirePresentationOwner() }
    context.invalidate()
    #expect(throws: PluginError.unavailable) { try context.requirePresentationOwner() }
}
#endif
