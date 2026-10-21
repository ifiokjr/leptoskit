import CLeptosKitBridge
import LeptosKitBridge
import UIKit

struct RuntimeFailure: Error, LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

/// The controller and its tasks are UI-actor confined. The handle is never sent
/// to a worker; its separate owner releases the runtime even on init failure.
private final class AppHandle {
    let raw: OpaquePointer
    init(_ raw: OpaquePointer) { self.raw = raw }
    deinit { lk_app_destroy(raw) }
}

/// Renders the guest into UIKit controls and owns all native plugin tasks.
@MainActor
public final class LeptosKitViewController: UIViewController, UITextFieldDelegate {
    private var app: AppHandle?
    private let dispatcher = NativeDispatcher()
    private var jobs: [UInt64: Task<Void, Never>] = [:]
    private let jobBudget = NativeJobBudget()
    private var pumpScheduled = false
    private var widgets: [Int: UIView] = [:]
    private var started = false
    private var applying = false
    private var lastSize = CGSize.zero
    private var readinessScheduled = false
    private var readinessDelivered = false

    /// Ready means a native root was attached and survived a subsequent UI turn.
    public var onReady: (() -> Void)?
    /// Owners can recover a candidate; standalone controllers retain their error UI.
    public var onFailure: ((Error) -> Void)?

    public init(
        wasm: Data,
        register: (OpaquePointer, NativeDispatcher) throws -> Void
    ) throws {
        super.init(nibName: nil, bundle: nil)
        var error = lk_buffer()
        let manager = FileManager.default
        let dataDirectory = try manager.url(
            for: .applicationSupportDirectory, in: .userDomainMask,
            appropriateFor: nil, create: true
        )
        let cacheDirectory = try manager.url(
            for: .cachesDirectory, in: .userDomainMask,
            appropriateFor: nil, create: true
        )
        let dataPath = Array(dataDirectory.path.utf8)
        let cachePath = Array(cacheDirectory.path.utf8)
        let raw = dataPath.withUnsafeBufferPointer { data in
            cachePath.withUnsafeBufferPointer { cache in
                var context = lk_app_context(
                    data_dir: data.baseAddress, data_dir_len: data.count,
                    cache_dir: cache.baseAddress, cache_dir_len: cache.count
                )
                return wasm.withUnsafeBytes {
                    leptoskit_app_create_with_context(
                        $0.bindMemory(to: UInt8.self).baseAddress, $0.count, &context, &error
                    )
                }
            }
        }
        guard let raw else {
            throw RuntimeFailure(message: try NativeBuffer(taking: error).utf8())
        }
        app = AppHandle(raw)
        guard let bridge = lk_app_plugins(raw) else { throw PluginError.native }
        do { try register(bridge, dispatcher) } catch {
            dispatcher.close()
            app = nil
            throw error
        }
    }

    required init?(coder: NSCoder) { nil }

    public override func loadView() {
        view = UIView()
        view.backgroundColor = .systemBackground
    }

    public override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        guard let app, view.bounds.width > 0, view.bounds.height > 0 else { return }
        do {
            if !started {
                started = true
                try check(lk_app_start(app.raw, Float(view.bounds.width), Float(view.bounds.height)))
            } else if lastSize != view.bounds.size {
                try check(lk_app_resize(
                    app.raw, Float(view.bounds.width), Float(view.bounds.height),
                    traitCollection.userInterfaceStyle == .dark
                ))
            }
            lastSize = view.bounds.size
            try pump()
        } catch { fail(error) }
    }

    /// Called by the scene delegate before disconnecting this guest.
    public func close() {
        onReady = nil
        onFailure = nil
        for job in jobs.values { job.cancel() }
        dispatcher.close()
        app = nil
        widgets.removeAll()
    }

    private func check(_ status: Int32) throws {
        guard status >= 0 else {
            let message = app.map { NativeBuffer(taking: lk_app_error($0.raw)) }
            throw RuntimeFailure(message: try message?.utf8() ?? "Application closed")
        }
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
        label.frame = view.bounds.insetBy(dx: 20, dy: 40)
        view.subviews.forEach { $0.removeFromSuperview() }
        view.addSubview(label)
    }

    private func pump() throws {
        guard let app else { return }
        guard let bridge = lk_app_plugins(app.raw) else { throw PluginError.native }
        for index in 0..<64 {
            let cancelled = lk_bridge_next_cancelled(bridge)
            if cancelled == 0 { break }
            guard cancelled > 0 else { throw PluginError.native }
            jobs[UInt64(cancelled)]?.cancel()
            if index == 63 { schedulePump() }
        }
        for _ in 0..<64 {
            guard jobBudget.canTakeRequest else { try render(); return }
            var request = lk_request()
            let status = lk_app_next_request(app.raw, &request)
            try check(status)
            if status == 0 { try render(); return }
            if status == 2 { break }
            try dispatch(request)
        }
        try render()
        schedulePump()
    }

    private func schedulePump() {
        guard app != nil, !pumpScheduled else { return }
        pumpScheduled = true
        Task { [weak self] in
            self?.pumpScheduled = false
            do { try self?.pump() } catch { self?.fail(error) }
        }
    }

    private func dispatch(_ request: lk_request) throws {
        let id = request.id
        do {
            try jobBudget.acquire(id: id, payloadBytes: request.payload.len)
        } catch {
            lk_buffer_release(request.payload)
            throw error
        }
        var borrowed = request
        let worker = app.flatMap { app in
            lk_app_worker(app.raw, &borrowed).map { NativeRustWorker(taking: $0) }
        }
        if worker != nil { lk_buffer_release(request.payload) }
        // Start on a later actor turn: no reentrancy into the interpreter.
        jobs[id] = Task { [weak self] in
            guard let self else {
                if worker == nil { lk_buffer_release(request.payload) }
                return
            }
            await runRequest(request, worker: worker)
        }
    }

    private func runRequest(_ request: lk_request, worker: NativeRustWorker?) async {
        let id = request.id
        defer {
            jobs.removeValue(forKey: id)
            jobBudget.release(id: id)
            schedulePump()
        }
        guard !Task.isCancelled else {
            if worker == nil { lk_buffer_release(request.payload) }
            return
        }
        if let worker { await runWorker(worker, id: id); return }
        let result: Result<Data, Error>
        do { result = .success(try await dispatcher.dispatch(request)) } catch { result = .failure(error) }
        guard let app = self.app, !Task.isCancelled else { return }
        do {
            let status = try complete(id, result)
            if status == -PluginError.invalidCall.rawValue { return }
            try check(status)
            try check(lk_app_wake(app.raw))
        } catch { fail(error) }
    }

    private func runWorker(_ worker: NativeRustWorker, id: UInt64) async {
        let reply = await worker.run()
        guard let app, !Task.isCancelled else { return }
        do {
            guard let bridge = lk_app_plugins(app.raw) else { throw PluginError.cancelled }
            let status = reply.complete(on: bridge, id: id)
            if status == -PluginError.invalidCall.rawValue { return }
            try check(status)
            try check(lk_app_wake(app.raw))
        } catch { fail(error) }
    }

    private func complete(_ id: UInt64, _ result: Result<Data, Error>) throws -> Int32 {
        guard let app, let bridge = lk_app_plugins(app.raw) else { throw PluginError.cancelled }
        switch result {
        case .success(let data):
            return data.withUnsafeBytes {
                lk_bridge_complete(bridge, id, 0, $0.bindMemory(to: UInt8.self).baseAddress, $0.count)
            }
        case .failure(let error):
            let code = (error as? PluginError)?.rawValue ?? PluginError.native.rawValue
            if !(error is PluginError) { NSLog("LeptosKit plugin: %@", error.localizedDescription) }
            return lk_bridge_complete(bridge, id, code, nil, 0)
        }
    }
}

extension LeptosKitViewController {
    private func render() throws {
        guard let app else { return }
        let buffer = NativeBuffer(taking: lk_app_render(app.raw))
        let transaction = try buffer.withUnsafeBytes {
            let decoder = JSONDecoder()
            decoder.keyDecodingStrategy = .convertFromSnakeCase
            return try decoder.decode(Transaction.self, from: Data($0))
        }
        guard transaction.version == 1 else { throw PluginError.unavailable }
        applying = true
        defer { applying = false }
        for change in transaction.mutations {
            try apply(change)
        }
        if let root = transaction.root, let widget = widgets[root], widget.superview !== view {
            view.addSubview(widget)
        }
        if let root = transaction.root, let widget = widgets[root], widget.superview === view,
           widget.bounds.width > 0, widget.bounds.height > 0, !readinessScheduled {
            readinessScheduled = true
            Task { [weak self, weak widget] in
                guard let self, let widget, self.app != nil, !readinessDelivered,
                      widget.superview === view, view.window != nil else {
                    self?.readinessScheduled = false
                    return
                }
                readinessDelivered = true
                onReady?()
            }
        }
        for scroll in widgets.values.compactMap({ $0 as? UIScrollView }) {
            scroll.contentSize = scroll.subviews.reduce(CGSize.zero) { size, child in
                CGSize(width: max(size.width, child.frame.maxX), height: max(size.height, child.frame.maxY))
            }
        }
    }

    private func create(_ item: Change.Widget) throws {
        let widget: UIView
        switch item.kind {
        case "view", "View": widget = UIView()
        case "text", "text_run", "Text", "TextRun":
            let label = UILabel()
            label.numberOfLines = 0
            widget = label
        case "button", "Button":
            let button = UIButton(type: .system)
            button.addAction(UIAction { [weak self] _ in self?.event(item.id, 0) }, for: .touchUpInside)
            widget = button
        case "text_input", "TextInput":
            let field = UITextField()
            field.delegate = self
            field.addAction(UIAction { [weak self, weak field] _ in
                self?.event(item.id, 1, field?.text ?? "")
            }, for: .editingChanged)
            widget = field
        case "scroll_view", "ScrollView": widget = UIScrollView()
        case "image", "Image": widget = UIImageView()
        default: throw RuntimeFailure(message: "Unsupported native widget: \(item.kind ?? "missing kind")")
        }
        widget.tag = item.id
        widgets[item.id] = widget
        try WidgetStyle.apply(item.props, to: widget)
    }

    private func apply(_ change: Change) throws {
        if let item = change.create { try create(item) }
        if let item = change.updateProps, let widget = widgets[item.id] {
            try WidgetStyle.apply(item.props, to: widget)
        }
        if let item = change.updateLayout, let widget = widgets[item.id] {
            widget.frame = CGRect(x: item.offsetX, y: item.offsetY, width: item.width, height: item.height)
        }
        if let edge = change.insertChild, let parent = widgets[edge.parent], let child = widgets[edge.child] {
            child.removeFromSuperview()
            parent.insertSubview(child, at: min(edge.index ?? parent.subviews.count, parent.subviews.count))
        }
        if let edge = change.removeChild { widgets[edge.child]?.removeFromSuperview() }
        if let item = change.delete { widgets.removeValue(forKey: item.id)?.removeFromSuperview() }
    }

    private func event(_ node: Int, _ kind: UInt32, _ text: String = "") {
        guard let app, !applying else { return }
        do {
            let bytes = Array(text.utf8)
            try check(bytes.withUnsafeBufferPointer {
                lk_app_event(app.raw, node, kind, $0.baseAddress, $0.count)
            })
            try pump()
        } catch { fail(error) }
    }

    public func textFieldShouldReturn(_ textField: UITextField) -> Bool {
        event(textField.tag, 2, textField.text ?? "")
        return true
    }

    public func textFieldDidBeginEditing(_ textField: UITextField) { event(textField.tag, 3) }
    public func textFieldDidEndEditing(_ textField: UITextField) { event(textField.tag, 4) }
}
