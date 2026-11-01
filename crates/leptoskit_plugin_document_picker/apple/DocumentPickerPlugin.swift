import Foundation
import LeptosKitBridge
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

/// The request and reply framing this host reads and writes.
private enum Wire {
    /// The request carries one u32 byte limit.
    static let limitBytes = 4

    /// The only method: present the picker and read one document.
    static let open: UInt32 = 0
}

/// Imports one document, retaining no bookmark and requesting no storage permission.
@MainActor
public final class DocumentPickerPlugin: NSObject, NativePlugin {
    public let name = "document-picker"
    public let methods: [UInt32] = [Wire.open]
    private let context: NativePluginContext
    private var closed = false
    private var busy = false
    private let selection = DocumentSelection()
    private var selectionID: UUID?
    private var reader: DocumentReadJob?

    #if canImport(UIKit)
    private var picker: UIDocumentPickerViewController?
    #elseif canImport(AppKit)
    private var panel: NSOpenPanel?
    #endif

    public init(context: NativePluginContext) { self.context = context }

    public func close() {
        closed = true
        cancelSelection()
        reader?.cancel()
    }

    /// Chooses one document, then reads it within the caller's byte limit.
    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard method == Wire.open else { throw PluginError.unavailable }

        let limit = try limit(from: payload)

        guard !busy else { throw PluginError.busy }

        try Task.checkCancellation()
        busy = true
        let identifier = UUID()

        selectionID = identifier
        defer {
            busy = false
            selectionID = nil
            reader = nil
        }
        let url = try await choose(identifier: identifier)
        try Task.checkCancellation()

        return try await read(url, limit: limit)
    }

    /// Reads the caller's byte limit, refusing one this reader cannot honour.
    private func limit(from payload: NativeBuffer) throws -> Int {
        let limit = try payload.withUnsafeBytes { bytes -> Int in

            guard bytes.count == Wire.limitBytes else { throw PluginError.invalidArgument }

            return Int(UInt32(littleEndian: bytes.loadUnaligned(as: UInt32.self)))
        }

        guard limit <= DocumentReader.maximum else { throw PluginError.tooLarge }

        return limit
    }

    /// Reads the chosen document off the presenter's thread.
    private func read(_ url: URL, limit: Int) async throws -> Data {
        let reader = DocumentReadJob(url: url, limit: limit)
        self.reader = reader
        let result = try await reader.run()
        try Task.checkCancellation()

        guard !closed else { throw PluginError.cancelled }

        return result
    }

    /// Abandons the chooser when this call is cancelled before it answers.
    private func choose(identifier: UUID) async throws -> URL {
        try await withTaskCancellationHandler {
            try await present()
        } onCancel: {
            Task { @MainActor [weak self] in
                self?.cancel(identifier: identifier)
            }
        }
    }

    /// Abandons the chooser only while this call still owns the selection.
    private func cancel(identifier: UUID) {
        guard selectionID == identifier else { return }

        cancelSelection()
    }

    private func cancelSelection() {
        dismissChooser()
        finish(.failure(PluginError.cancelled))
    }

    private func finish(_ result: Result<URL, Error>) {
        releaseChooser()
        selection.finish(result)
    }

    #if canImport(UIKit)
    /// Presents the document picker, then waits for the delegate to answer.
    private func present() async throws -> URL {
        let owner = try context.requirePresentationOwner()

        guard owner.presentedViewController == nil else { throw PluginError.busy }

        let picker = UIDocumentPickerViewController(forOpeningContentTypes: [.data], asCopy: false)
        picker.allowsMultipleSelection = false
        picker.delegate = self
        self.picker = picker

        return try await withCheckedThrowingContinuation { continuation in
            selection.begin(continuation)
            owner.present(picker, animated: true)
            picker.presentationController?.delegate = self
        }
    }

    private func dismissChooser() {
        picker?.dismiss(animated: false)
    }

    private func releaseChooser() {
        picker?.delegate = nil
        picker = nil
    }
    #elseif canImport(AppKit)
    /// Presents the open panel as a sheet, then waits for the user to answer.
    private func present() async throws -> URL {
        let owner = try context.requirePresentationOwner()
        let window = try window(of: owner)

        guard window.attachedSheet == nil else { throw PluginError.busy }

        let panel = NSOpenPanel()
        panel.canChooseDirectories = false
        panel.canChooseFiles = true
        panel.allowsMultipleSelection = false
        self.panel = panel

        return try await withCheckedThrowingContinuation { continuation in
            let identifier = selection.begin(continuation)

            panel.beginSheetModal(for: window) { [weak self, weak panel] result in
                self?.panelResponded(result, panel: panel, identifier: identifier)
            }
        }
    }

    /// The window the sheet must attach to; a host without one cannot present.
    private func window(of owner: NativePresentationOwner) throws -> NSWindow {
        guard let window = owner.view.window else { throw PluginError.unavailable }

        return window
    }

    /// Answers the sheet once; a callback for a replaced panel is ignored.
    private func panelResponded(_ result: NSApplication.ModalResponse, panel: NSOpenPanel?, identifier: UUID) {
        guard let panel, self.panel === panel, selection.identifier == identifier else { return }

        if result == .OK, let url = panel.url {
            finish(.success(url))
        } else {
            finish(.failure(PluginError.cancelled))
        }
    }

    private func dismissChooser() {
        panel?.cancel(nil)
    }

    private func releaseChooser() {
        panel = nil
    }
    #else
    private func present() async throws -> URL {
        throw PluginError.unavailable
    }

    private func dismissChooser() {}

    private func releaseChooser() {}
    #endif
}

#if canImport(UIKit)
extension DocumentPickerPlugin: UIDocumentPickerDelegate, UIAdaptivePresentationControllerDelegate {
    public func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        guard controller === picker else { return }

        guard urls.count == 1, let url = urls.first else {
            finish(.failure(PluginError.invalidArgument))

            return
        }

        finish(.success(url))
    }

    public func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
        guard controller === picker else { return }
        finish(.failure(PluginError.cancelled))
    }

    public func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
        guard presentationController.presentedViewController === picker else { return }
        finish(.failure(PluginError.cancelled))
    }
}
#endif
