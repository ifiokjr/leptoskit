import CLeptosKitBridge
import AppKit
import LeptosKitBridge

/// Owns the platform view while the shared runtime owns native admission and teardown.
@MainActor
public final class LeptosKitMacViewController: NSViewController, NSTextFieldDelegate {
    private var runtime: NativeAppRuntime?
    private var renderer: MacWidgetRenderer?
    private static let errorInset = CGSize(width: 20, height: 40)

    public init(
        wasm: Data,
        register: (OpaquePointer, NativeDispatcher, NativePluginContext) throws -> Void
    ) throws {
        super.init(nibName: nil, bundle: nil)

        guard let identifier = Bundle.main.bundleIdentifier else { throw PluginError.invalidArgument }
        runtime = try NativeAppRuntime(wasm: wasm, identifier: identifier, owner: self, register: register)
        runtime?.onRender = { [weak self] raw in try self?.render(raw) }
        runtime?.onFailure = { [weak self] error in self?.fail(error) }
    }

    required init?(coder: NSCoder) { nil }
    isolated deinit { close() }

    public override func loadView() {
        view = FlippedView()
        view.wantsLayer = true
        view.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        let renderer = MacWidgetRenderer(view: view)
        renderer.onEvent = { [weak self] node, kind, text in self?.runtime?.event(node, kind, text) }
        self.renderer = renderer
    }

    public override func viewDidLayout() {
        super.viewDidLayout()
        do {
            let dark = view.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            try runtime?.layout(size: view.bounds.size, dark: dark)
        } catch { fail(error) }
    }

    public func close() {
        runtime?.close()
        renderer?.close()
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
    private func fail(_ error: Error) {
        close()
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
    }
}
