import CLeptosKitBridge
import AppKit
import LeptosKitBridge

/// Owns the platform view while the shared runtime owns native admission and teardown.
@MainActor
public final class LeptosKitMacViewController: NSViewController, NSTextFieldDelegate {
    private var runtime: NativeAppRuntime?
    private var renderer: MacWidgetRenderer?
    private var development: NativeDevelopmentGuests?
    private let register: (OpaquePointer, NativeDispatcher, NativePluginContext) throws -> Void
    private var riveAnimator: NativeRiveAnimator?
    private static let errorInset = CGSize(width: 20, height: 40)

    public init(
        wasm: Data,
        register: @escaping (OpaquePointer, NativeDispatcher, NativePluginContext) throws -> Void
    ) throws {
        self.register = register
        super.init(nibName: nil, bundle: nil)
        runtime = try makeRuntime(wasm)

        if let url = NativeDevelopmentGuests.url {
            development = NativeDevelopmentGuests(url: url, running: wasm) { [weak self] wasm in
                self?.reload(wasm)
            }
        }
    }

    required init?(coder: NSCoder) { nil }
    isolated deinit { close() }

    public override func loadView() {
        view = FlippedView()
        view.wantsLayer = true
        view.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        installRenderer()
    }

    public override func viewDidAppear() {
        super.viewDidAppear()
        riveAnimator?.setPresented(true)
    }

    public override func viewWillDisappear() {
        riveAnimator?.setPresented(false)
        super.viewWillDisappear()
    }

    public override func viewDidLayout() {
        super.viewDidLayout()

        do { try layoutRuntime() } catch { fail(error) }
    }

    public func close() {
        development?.close()
        development = nil
        closeGuest()
    }

    public func closeAndWait() async throws {
        close()
        try await runtime?.closeAndWait()
    }

    public func controlTextDidChange(_ notification: Notification) {
        renderer?.controlTextDidChange(notification)
    }

    public func controlTextDidBeginEditing(_ notification: Notification) {
        renderer?.controlTextDidBeginEditing(notification)
    }

    public func controlTextDidEndEditing(_ notification: Notification) {
        renderer?.controlTextDidEndEditing(notification)
    }

    private func makeRuntime(_ wasm: Data) throws -> NativeAppRuntime {
        guard let identifier = Bundle.main.bundleIdentifier else { throw PluginError.invalidArgument }
        let runtime = try NativeAppRuntime(wasm: wasm, identifier: identifier, owner: self, register: register)
        runtime.onRender = { [weak self] raw in try self?.render(raw) }
        runtime.onFailure = { [weak self] error in self?.fail(error) }

        return runtime
    }

    private func installRenderer() {
        let renderer = MacWidgetRenderer(view: view)
        renderer.onEvent = { [weak self] node, kind, text in self?.runtime?.event(node, kind, text) }
        self.renderer = renderer
        let animator = NativeRiveAnimator(view: view)
        animator.sink = renderer
        animator.hasScenes = { [weak runtime = self.runtime] in runtime?.hasRive == true }
        animator.tick = { [weak runtime = self.runtime] elapsed, scale, flags, callbacks in
            try runtime?.riveTick(elapsed, scale: scale, flags: flags, callbacks: callbacks) ?? false
        }
        animator.onFailure = { [weak self] error in self?.fail(error) }

        animator.setPresented(view.window != nil)
        riveAnimator = animator
    }

    private func layoutRuntime() throws {
        let dark = view.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        try runtime?.layout(size: view.bounds.size, dark: dark)
    }

    /// Replaces the guest with one from `leptoskit dev` in the same window, which
    /// keeps its size and position. The replaced guest's teardown finishes behind it.
    private func reload(_ wasm: Data) {
        riveAnimator?.close()
        riveAnimator = nil

        if let previous = runtime {
            previous.close()
            Task {
                do { try await previous.closeAndWait() } catch {
                    NSLog("leptoskit dev: previous guest cleanup: %@", error.localizedDescription)
                }
            }
        }

        runtime = nil
        renderer?.close()
        view.subviews.forEach { $0.removeFromSuperview() }

        do {
            runtime = try makeRuntime(wasm)
            installRenderer()
            try layoutRuntime()
            NSLog("leptoskit dev: reloaded")
        } catch { fail(error) }
    }

    private func closeGuest() {
        riveAnimator?.close()
        riveAnimator = nil
        runtime?.close()
        renderer?.close()
    }

    /// Shows why the guest stopped. A development session keeps following, so
    /// the next rebuilt guest replaces the message.
    private func fail(_ error: Error) {
        closeGuest()
        NSLog("leptoskit: %@", error.localizedDescription)
        let label = NSTextField(wrappingLabelWithString: error.localizedDescription)
        label.setAccessibilityIdentifier("leptoskit-error")
        label.translatesAutoresizingMaskIntoConstraints = false
        view.subviews.forEach { $0.removeFromSuperview() }
        view.addSubview(label)
        // Constraints, not a frame: a launch failure can arrive before layout.
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: Self.errorInset.width),
            label.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -Self.errorInset.width),
            label.topAnchor.constraint(equalTo: view.topAnchor, constant: Self.errorInset.height),
            label.bottomAnchor.constraint(lessThanOrEqualTo: view.bottomAnchor, constant: -Self.errorInset.height)
        ])
    }

    private func render(_ raw: OpaquePointer) throws {
        let buffer = NativeBuffer(taking: lk_app_render(raw))
        let transaction = try buffer.withUnsafeBytes {
            let decoder = JSONDecoder()
            decoder.keyDecodingStrategy = .convertFromSnakeCase

            return try decoder.decode(Transaction.self, from: Data($0))
        }
        try renderer?.render(transaction)

        if lk_app_has_rive(raw) == 1 { riveAnimator?.requestFrame() } else { riveAnimator?.stop() }
    }
}
