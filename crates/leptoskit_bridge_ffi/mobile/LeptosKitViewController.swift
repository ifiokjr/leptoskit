import CLeptosKitBridge
import LeptosKitBridge
import UIKit

// WidgetStyle shares the controller's runtime diagnostic.
typealias RuntimeFailure = NativeAppError

/// Owns the platform view while the shared runtime owns native admission and teardown.
@MainActor
public final class LeptosKitViewController: UIViewController, UITextFieldDelegate {
    private var runtime: NativeAppRuntime?
    private var renderer: MobileWidgetRenderer?
    private var riveAnimator: NativeRiveAnimator?
    private static let errorInset = CGSize(width: 20, height: 40)
    public var onReady: (() -> Void)?
    public var onFailure: ((Error) -> Void)?
    /// Called when ``canvasColor`` changes.
    public var onCanvasColorChange: (() -> Void)?
    /// The guest root's opaque background, which a host paints behind the safe areas.
    public var canvasColor: UIColor? { renderer?.canvas }
    /// Status bar content that contrasts with ``canvasColor``.
    public var canvasStatusBarStyle: UIStatusBarStyle {
        guard let canvas = canvasColor else { return .default }
        var red: CGFloat = 0, green: CGFloat = 0, blue: CGFloat = 0
        canvas.getRed(&red, green: &green, blue: &blue, alpha: nil)
        let luminance = 0.2126 * red + 0.7152 * green + 0.0722 * blue

        return luminance > 0.5 ? .darkContent : .lightContent
    }

    public init(
        wasm: Data,
        register: (OpaquePointer, NativeDispatcher, NativePluginContext) throws -> Void
    ) throws {
        super.init(nibName: nil, bundle: nil)
        runtime = try NativeAppRuntime(wasm: wasm, identifier: nil, owner: self, register: register)
        runtime?.onRender = { [weak self] raw in try self?.render(raw) }
        runtime?.onFailure = { [weak self] error in self?.fail(error) }
    }

    required init?(coder: NSCoder) { nil }
    isolated deinit { close() }

    public override var supportedInterfaceOrientations: UIInterfaceOrientationMask {
        runtime?.pluginContext.orientationMask ?? super.supportedInterfaceOrientations
    }

    public override func loadView() {
        view = UIView()
        view.backgroundColor = .systemBackground
        let renderer = MobileWidgetRenderer(view: view)
        renderer.onEvent = { [weak self] node, kind, text in self?.runtime?.event(node, kind, text) }
        renderer.onReady = { [weak self] in self?.onReady?() }
        renderer.onCanvasChange = { [weak self] in self?.onCanvasColorChange?() }
        self.renderer = renderer
        let animator = NativeRiveAnimator(view: view)
        animator.sink = renderer
        animator.hasScenes = { [weak runtime = self.runtime] in runtime?.hasRive == true }
        animator.tick = { [weak runtime = self.runtime] elapsed, scale, flags, callbacks in
            try runtime?.riveTick(elapsed, scale: scale, flags: flags, callbacks: callbacks) ?? false
        }
        animator.onFailure = { [weak self] error in self?.fail(error) }
        renderer.onRiveViewportChange = { [weak animator] in animator?.requestFrame() }
        riveAnimator = animator
    }

    public override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        riveAnimator?.setPresented(true)
    }

    public override func viewWillDisappear(_ animated: Bool) {
        riveAnimator?.setPresented(false)
        super.viewWillDisappear(animated)
    }

    public override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        do {
            try runtime?.layout(size: view.bounds.size, dark: traitCollection.userInterfaceStyle == .dark)
        } catch { fail(error) }
    }

    public func close() {
        riveAnimator?.close()
        onReady = nil
        onFailure = nil
        onCanvasColorChange = nil
        runtime?.close()
        renderer?.close()
    }

    public func closeAndWait() async throws {
        close()
        try await runtime?.closeAndWait()
    }

    public func textFieldShouldReturn(_ textField: UITextField) -> Bool {
        renderer?.textFieldShouldReturn(textField) ?? true
    }

    public func textFieldDidBeginEditing(_ textField: UITextField) {
        renderer?.textFieldDidBeginEditing(textField)
    }

    public func textFieldDidEndEditing(_ textField: UITextField) {
        renderer?.textFieldDidEndEditing(textField)
    }

    private func fail(_ error: Error) {
        let callback = onFailure
        close()

        if let callback {
            callback(error)

            return
        }

        let label = UILabel()
        label.text = error.localizedDescription
        label.accessibilityIdentifier = "leptoskit-error"

        label.numberOfLines = 0
        label.translatesAutoresizingMaskIntoConstraints = false
        view.subviews.forEach { $0.removeFromSuperview() }
        view.addSubview(label)
        // Constraints, not a frame: a launch failure can arrive before layout.
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: Self.errorInset.width),
            label.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -Self.errorInset.width),
            label.topAnchor.constraint(equalTo: view.topAnchor, constant: Self.errorInset.height),
            label.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -Self.errorInset.height)
        ])
    }

    private func render(_ raw: OpaquePointer) throws {
        // Most pumps follow a plugin reply that changed nothing on screen.
        guard lk_app_render_pending(raw) != 0 else {
            if lk_app_has_rive(raw) == 1 { riveAnimator?.requestFrame() } else { riveAnimator?.stop() }

            return
        }

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
