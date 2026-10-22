import AppKit
import CLeptosKitBridge
import LeptosKitBridge

private struct RuntimeFailure: Error, LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

private final class AppHandle {
    let raw: OpaquePointer
    init(_ raw: OpaquePointer) { self.raw = raw }
    deinit { lk_app_destroy(raw) }
}

private final class FlippedView: NSView {
    override var isFlipped: Bool { true }
}

/// An AppKit renderer and owner of the guest runtime and native plugin tasks.
/// The runtime is confined to the main actor; callbacks never reenter Wasm.
@MainActor
public final class LeptosKitMacViewController: NSViewController, NSTextFieldDelegate {
    private var app: AppHandle?
    private let dispatcher = NativeDispatcher()
    private lazy var pluginContext = NativePluginContext(owner: self)
    private var jobs: [UInt64: Task<Void, Never>] = [:]
    private let jobBudget = NativeJobBudget()
    private var pumpScheduled = false
    private var widgets: [Int: NSView] = [:]
    private var started = false
    private var applying = false
    private var lastSize = CGSize.zero

    public init(
        wasm: Data,
        register: (OpaquePointer, NativeDispatcher, NativePluginContext) throws -> Void
    ) throws {
        super.init(nibName: nil, bundle: nil)
        guard let identifier = Bundle.main.bundleIdentifier else { throw PluginError.invalidArgument }
        let manager = FileManager.default
        let dataDirectory = try manager.url(
            for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true
        ).appendingPathComponent(identifier, isDirectory: true)
        let cacheDirectory = try manager.url(
            for: .cachesDirectory, in: .userDomainMask, appropriateFor: nil, create: true
        ).appendingPathComponent(identifier, isDirectory: true)
        try manager.createDirectory(at: dataDirectory, withIntermediateDirectories: true)
        try manager.createDirectory(at: cacheDirectory, withIntermediateDirectories: true)
        let dataPath = Array(dataDirectory.path.utf8)
        let cachePath = Array(cacheDirectory.path.utf8)
        var error = lk_buffer()
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
        guard let raw else { throw RuntimeFailure(message: try NativeBuffer(taking: error).utf8()) }
        app = AppHandle(raw)
        guard let bridge = lk_app_plugins(raw) else { throw PluginError.native }
        try register(bridge, dispatcher, pluginContext)
    }

    required init?(coder: NSCoder) { nil }

    public override func loadView() {
        view = FlippedView()
        view.wantsLayer = true
        view.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
    }

    public override func viewDidLayout() {
        super.viewDidLayout()
        guard let app, view.bounds.width > 0, view.bounds.height > 0 else { return }
        do {
            if !started {
                started = true
                try check(lk_app_start(app.raw, Float(view.bounds.width), Float(view.bounds.height)))
            } else if lastSize != view.bounds.size {
                try check(lk_app_resize(
                    app.raw, Float(view.bounds.width), Float(view.bounds.height),
                    view.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
                ))
            }
            lastSize = view.bounds.size
            try pump()
        } catch { fail(error) }
    }

    public func close() {
        for job in jobs.values { job.cancel() }
        dispatcher.close()
        pluginContext.invalidate()
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
        close()
        let label = NSTextField(wrappingLabelWithString: error.localizedDescription)
        label.setAccessibilityIdentifier("leptoskit-error")
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

extension LeptosKitMacViewController {
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
        for change in transaction.mutations { try apply(change) }
        if let root = transaction.root, let widget = widgets[root], widget.superview !== view {
            view.addSubview(widget)
        }
        for scroll in widgets.values.compactMap({ $0 as? NSScrollView }) {
            guard let document = scroll.documentView else { continue }
            document.setFrameSize(document.subviews.reduce(scroll.contentSize) { size, child in
                CGSize(width: max(size.width, child.frame.maxX), height: max(size.height, child.frame.maxY))
            })
        }
    }

    private func create(_ item: Change.Widget) throws {
        let widget: NSView
        switch item.kind {
        case "view", "View": widget = FlippedView()
        case "text", "text_run", "Text", "TextRun": widget = NSTextField(wrappingLabelWithString: "")
        case "button", "Button":
            let button = NSButton(title: "", target: self, action: #selector(pressed(_:)))
            button.bezelStyle = .rounded
            widget = button
        case "text_input", "TextInput":
            let field = NSTextField()
            field.delegate = self
            field.target = self
            field.action = #selector(submitted(_:))
            widget = field
        case "scroll_view", "ScrollView":
            let scroll = NSScrollView()
            scroll.documentView = FlippedView()
            scroll.hasVerticalScroller = true
            scroll.hasHorizontalScroller = true
            scroll.drawsBackground = false
            widget = scroll
        case "image", "Image": widget = NSImageView()
        default: throw RuntimeFailure(message: "Unsupported native widget: \(item.kind ?? "missing kind")")
        }
        widget.identifier = NSUserInterfaceItemIdentifier(String(item.id))
        widgets[item.id] = widget
        try apply(item.props, to: widget)
    }

    private func apply(_ change: Change) throws {
        if let item = change.create { try create(item) }
        if let item = change.updateProps, let widget = widgets[item.id] { try apply(item.props, to: widget) }
        if let item = change.updateLayout, let widget = widgets[item.id] {
            widget.frame = CGRect(x: item.offsetX, y: item.offsetY, width: item.width, height: item.height)
        }
        if let edge = change.insertChild, let parent = widgets[edge.parent], let child = widgets[edge.child] {
            let container = (parent as? NSScrollView)?.documentView ?? parent
            child.removeFromSuperview()
            let index = min(edge.index ?? container.subviews.count, container.subviews.count)
            let sibling = index < container.subviews.count ? container.subviews[index] : nil
            container.addSubview(child, positioned: sibling == nil ? .above : .below, relativeTo: sibling)
        }
        if let edge = change.removeChild { widgets[edge.child]?.removeFromSuperview() }
        if let item = change.delete { widgets.removeValue(forKey: item.id)?.removeFromSuperview() }
    }

    private func node(_ widget: NSView) -> Int? { widget.identifier.flatMap { Int($0.rawValue) } }

    @objc private func pressed(_ sender: NSButton) {
        if let id = node(sender) { event(id, 0) }
    }

    @objc private func submitted(_ sender: NSTextField) {
        if let id = node(sender) { event(id, 2, sender.stringValue) }
    }

    public func controlTextDidChange(_ notification: Notification) {
        if let field = notification.object as? NSTextField, let id = node(field) { event(id, 1, field.stringValue) }
    }

    public func controlTextDidBeginEditing(_ notification: Notification) {
        if let field = notification.object as? NSTextField, let id = node(field) { event(id, 3) }
    }

    public func controlTextDidEndEditing(_ notification: Notification) {
        if let field = notification.object as? NSTextField, let id = node(field) { event(id, 4) }
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

    private func apply(_ props: WidgetProps, to widget: NSView) throws {
        widget.setAccessibilityLabel(props.accessibilityLabel)
        widget.setAccessibilityIdentifier(props.accessibilityLabel)
        widget.wantsLayer = true
        widget.layer?.backgroundColor = props.background.map(color)?.cgColor
        widget.isHidden = props.hidden ?? false
        widget.alphaValue = props.opacity ?? 1
        widget.layer?.cornerRadius = props.cornerRadius ?? 0
        widget.layer?.borderWidth = props.borderWidth ?? 0
        widget.layer?.borderColor = props.borderColor.map(color)?.cgColor
        widget.layer?.masksToBounds = (props.clip ?? false) || (props.cornerRadius ?? 0) > 0
        applyText(props, to: widget)
        if let image = widget as? NSImageView {
            image.imageScaling = .scaleProportionallyUpOrDown
            if let source = props.source {
                guard let url = URL(string: source), url.isFileURL,
                      let loaded = NSImage(contentsOf: url) else {
                    throw RuntimeFailure(message: "Image requires a readable file URL: \(source)")
                }
                image.image = loaded
            } else { image.image = nil }
        }
    }

    private func applyText(_ props: WidgetProps, to widget: NSView) {
        let weight: NSFont.Weight = switch props.fontWeight {
        case "bold": .bold
        case "semibold": .semibold
        case "medium": .medium
        default: .regular
        }
        let font = NSFont.systemFont(ofSize: props.fontSize ?? 14, weight: weight)
        let alignment: NSTextAlignment = switch props.textAlign {
        case "center": .center
        case "end": .right
        default: .natural
        }
        if let button = widget as? NSButton {
            button.title = props.text ?? ""
            button.font = font
            button.contentTintColor = props.color.map(color)
            button.isEnabled = props.enabled ?? true
        }
        if let field = widget as? NSTextField {
            let value = field.isEditable ? (props.value ?? props.text ?? "") : (props.text ?? "")
            if field.stringValue != value { field.stringValue = value }
            field.placeholderString = props.placeholder
            field.font = font
            field.textColor = props.color.map(color) ?? .labelColor
            field.alignment = alignment
            field.isEnabled = props.enabled ?? true
        }
    }

    private func color(_ rgba: UInt32) -> NSColor {
        NSColor(red: CGFloat((rgba >> 24) & 255) / 255,
                green: CGFloat((rgba >> 16) & 255) / 255,
                blue: CGFloat((rgba >> 8) & 255) / 255,
                alpha: CGFloat(rgba & 255) / 255)
    }
}
