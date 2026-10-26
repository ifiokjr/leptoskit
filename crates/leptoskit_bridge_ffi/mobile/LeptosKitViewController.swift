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
    private static let errorInset = CGSize(width: 20, height: 40)
    public var onReady: (() -> Void)?
    public var onFailure: ((Error) -> Void)?

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
        self.renderer = renderer
    }

    public override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        do {
            try runtime?.layout(size: view.bounds.size, dark: traitCollection.userInterfaceStyle == .dark)
        } catch { fail(error) }
    }

    public func close() {
        onReady = nil
        onFailure = nil
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
        let buffer = NativeBuffer(taking: lk_app_render(raw))
        let transaction = try buffer.withUnsafeBytes {
            let decoder = JSONDecoder()
            decoder.keyDecodingStrategy = .convertFromSnakeCase

            return try decoder.decode(Transaction.self, from: Data($0))
        }
        try renderer?.render(transaction)
    }
}
