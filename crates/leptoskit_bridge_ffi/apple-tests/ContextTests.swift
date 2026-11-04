import CLeptosKitBridge
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

@Test
@MainActor
func appContextBridgeResolvesLiveOwnerAndReleasesCleanly() throws {
    _ = NSApplication.shared
    typealias Resolver = @convention(c) (UnsafeMutableRawPointer?, Int32) -> UnsafeMutableRawPointer?

    var appContext = lk_app_context(
        data_dir: nil, data_dir_len: 0,
        cache_dir: nil, cache_dir_len: 0,
        platform_vm: nil, platform_context: nil, release_platform_context: nil
    )

    // 1. Verify that retainForAppContext retains NativePluginContext without retaining its owner.
    do {
        var ephemeral: NSViewController? = NSViewController()
        weak var weakEphemeral = ephemeral
        let context = NativePluginContext(owner: try #require(ephemeral))
        context.retainForAppContext(&appContext)
        ephemeral = nil
        #expect(weakEphemeral == nil)
        let resolver = unsafeBitCast(try #require(appContext.platform_vm), to: Resolver.self)
        #expect(resolver(appContext.platform_context, 0) == nil)
        appContext.release_platform_context?(appContext.platform_vm, appContext.platform_context)
    }

    // 2. Verify attached vs presentation owner resolution and invalidation.
    let controller = NSViewController()
    controller.view = NSView(frame: NSRect(x: 0, y: 0, width: 100, height: 100))
    weak var weakContext: NativePluginContext?

    do {
        let context = NativePluginContext(owner: controller)
        weakContext = context
        context.retainForAppContext(&appContext)
    }

    // The retained C handle keeps NativePluginContext alive until release_platform_context.
    #expect(weakContext != nil)
    let resolver = unsafeBitCast(try #require(appContext.platform_vm), to: Resolver.self)

    // Before attaching to a window, both attached (0) and presentation (1) queries return nil.
    #expect(resolver(appContext.platform_context, 0) == nil)
    #expect(resolver(appContext.platform_context, 1) == nil)

    let window = NSWindow(contentViewController: controller)
    window.isReleasedWhenClosed = false
    defer {
        window.orderOut(nil)
        window.contentViewController = nil
    }
    // Attached to an ordered-out window: kind 0 succeeds, kind 1 (visible) returns nil.
    let expectedPtr = Unmanaged.passUnretained(controller).toOpaque()
    #expect(resolver(appContext.platform_context, 0) == expectedPtr)
    #expect(resolver(appContext.platform_context, 1) == nil)

    window.orderFront(nil)
    #expect(resolver(appContext.platform_context, 0) == expectedPtr)
    #expect(resolver(appContext.platform_context, 1) == expectedPtr)
    #expect(resolver(appContext.platform_context, 99) == nil)

    // Invalidation clears the weak owner immediately for Rust callers too.
    weakContext?.invalidate()
    #expect(resolver(appContext.platform_context, 0) == nil)
    #expect(resolver(appContext.platform_context, 1) == nil)

    appContext.release_platform_context?(appContext.platform_vm, appContext.platform_context)
    #expect(weakContext == nil)
}
#endif
