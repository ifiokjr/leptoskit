import Foundation
import LeptosKitBridge
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

@MainActor
public final class DocumentPickerPlugin: NSObject, NativePlugin {
    public let name = "document-picker"
    public let methods: [UInt32] = [0]
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

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard method == 0 else { throw PluginError.unavailable }
        let limit = try payload.withUnsafeBytes { bytes -> Int in

            guard bytes.count == 4 else { throw PluginError.invalidArgument }

            return Int(UInt32(littleEndian: bytes.loadUnaligned(as: UInt32.self)))
        }

        guard limit <= DocumentReader.maximum else { throw PluginError.tooLarge }

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
        let url = try await withTaskCancellationHandler {
            try await choose()
        } onCancel: {
            Task { @MainActor [weak self] in

                guard self?.selectionID == identifier else { return }
                self?.cancelSelection()
            }
        }
        try Task.checkCancellation()
        let reader = DocumentReadJob(url: url, limit: limit)
        self.reader = reader
        let result = try await reader.run()
        try Task.checkCancellation()

        guard !closed else { throw PluginError.cancelled }

        return result
    }

    private func choose() async throws -> URL {
        let owner = try context.requirePresentationOwner()

        #if canImport(UIKit)

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

        #elseif canImport(AppKit)

        guard let window = owner.view.window else { throw PluginError.unavailable }

        guard window.attachedSheet == nil else { throw PluginError.busy }
        let panel = NSOpenPanel()
        panel.canChooseDirectories = false
        panel.canChooseFiles = true
        panel.allowsMultipleSelection = false
        self.panel = panel

        return try await withCheckedThrowingContinuation { continuation in
            let identifier = selection.begin(continuation)

            panel.beginSheetModal(for: window) { [weak self, weak panel] result in

                guard let self, let panel, self.panel === panel,

                      self.selection.identifier == identifier else { return }

                if result == .OK, let url = panel.url {
                    self.finish(.success(url))
                } else {
                    self.finish(.failure(PluginError.cancelled))
                }
            }
        }
        #else
        throw PluginError.unavailable
        #endif
    }

    private func cancelSelection() {
        #if canImport(UIKit)
        picker?.dismiss(animated: false)
        #elseif canImport(AppKit)
        panel?.cancel(nil)
        #endif
        finish(.failure(PluginError.cancelled))
    }

    private func finish(_ result: Result<URL, Error>) {
        #if canImport(UIKit)
        picker?.delegate = nil
        picker = nil
        #elseif canImport(AppKit)
        panel = nil
        #endif
        selection.finish(result)
    }
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
