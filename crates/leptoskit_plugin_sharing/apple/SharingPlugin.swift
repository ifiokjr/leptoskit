import Foundation
import LeptosKitBridge
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

@MainActor
public final class SharingPlugin: NSObject, NativePlugin {
    public let name = "sharing"
    public let methods: [UInt32] = [0]
    private let context: NativePluginContext
    private var session: ShareSession?
    private var closed = false
    #if canImport(UIKit)
    private var activity: UIActivityViewController?
    #elseif canImport(AppKit)
    private var picker: NSSharingServicePicker?
    private var service: NSSharingService?
    private var serviceDelegate: ShareServiceDelegate?
    #endif

    public init(context: NativePluginContext) { self.context = context }

    public func close() {
        closed = true
        cancel()
        // AppKit cannot cancel a service after the user has chosen it. That
        // external compose UI now belongs to the service, not this runtime.
        finish(.failure(PluginError.cancelled))
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        guard method == 0 else { throw PluginError.unavailable }
        guard session == nil else { throw PluginError.busy }
        let content = try payload.withUnsafeBytes(ShareContent.decode)
        let owner = try context.requirePresentationOwner()
        try Task.checkCancellation()
        let invocation = UUID()
        currentInvocation = invocation
        return try await withTaskCancellationHandler {
            try await present(content, owner: owner)
        } onCancel: {
            Task { @MainActor [weak self] in
                guard self?.currentInvocation == invocation else { return }
                self?.cancel()
            }
        }
    }

    private var currentInvocation: UUID?

    private func present(_ content: ShareContent, owner: NativePresentationOwner) async throws -> Data {
        #if canImport(UIKit)
        guard owner.presentedViewController == nil, !owner.isBeingDismissed,
              !owner.isBeingPresented, owner.transitionCoordinator == nil else { throw PluginError.busy }
        let activity = UIActivityViewController(activityItems: [content.item], applicationActivities: nil)
        self.activity = activity
        let bounds = owner.view.bounds
        if let popover = activity.popoverPresentationController {
            popover.sourceView = owner.view
            popover.sourceRect = CGRect(x: bounds.midX, y: bounds.midY, width: 1, height: 1)
        }
        return try await withCheckedThrowingContinuation { continuation in
            let session = ShareSession(continuation)
            self.session = session
            activity.completionWithItemsHandler = { [weak self] _, completed, _, error in
                guard let self, self.session?.id == session.id else { return }
                if error != nil {
                    self.finish(.failure(PluginError.native))
                } else if completed {
                    self.finish(.success(Data()))
                } else {
                    self.finish(.failure(PluginError.cancelled))
                }
            }
            owner.present(activity, animated: true)
            activity.presentationController?.delegate = self
        }
        #elseif canImport(AppKit)
        guard let window = owner.view.window, window.attachedSheet == nil else { throw PluginError.busy }
        guard !NSSharingService.sharingServices(forItems: [content.item]).isEmpty else {
            throw PluginError.unavailable
        }
        let picker = NSSharingServicePicker(items: [content.item])
        picker.delegate = self
        self.picker = picker
        return try await withCheckedThrowingContinuation { continuation in
            session = ShareSession(continuation)
            let bounds = owner.view.bounds
            picker.show(relativeTo: NSRect(x: bounds.midX, y: bounds.midY, width: 1, height: 1),
                        of: owner.view, preferredEdge: .minY)
        }
        #else
        throw PluginError.unavailable
        #endif
    }

    private func cancel() {
        #if canImport(UIKit)
        activity?.dismiss(animated: false)
        finish(.failure(PluginError.cancelled))
        #elseif canImport(AppKit)
        if service == nil {
            picker?.close()
            finish(.failure(PluginError.cancelled))
        } else {
            // Retain busy admission until the service ends; cancellation is
            // not a promise to retract content already handed to that service.
            session?.complete(.failure(PluginError.cancelled))
        }
        #endif
    }

    private func finish(_ result: Result<Data, Error>) {
        let pending = session
        session = nil
        currentInvocation = nil
        #if canImport(UIKit)
        activity?.completionWithItemsHandler = nil
        activity = nil
        #elseif canImport(AppKit)
        picker?.delegate = nil
        picker = nil
        service?.delegate = nil
        service = nil
        serviceDelegate = nil
        #endif
        pending?.complete(result)
    }
}

#if canImport(UIKit)
extension SharingPlugin: UIAdaptivePresentationControllerDelegate {
    public func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
        guard presentationController.presentedViewController === activity else { return }
        finish(.failure(PluginError.cancelled))
    }
}
#elseif canImport(AppKit)
// AppKit invokes picker delegates on its UI thread; the older protocol lacks
// actor annotations. Swift checks this main-actor isolation at runtime.
extension SharingPlugin: @preconcurrency NSSharingServicePickerDelegate {
    public func sharingServicePicker(_ sharingServicePicker: NSSharingServicePicker,
                                     delegateFor sharingService: NSSharingService) -> (any NSSharingServiceDelegate)? {
        guard sharingServicePicker === picker, let id = session?.id else { return nil }
        service = sharingService
        let delegate = ShareServiceDelegate(context: context) { [weak self] result in
            guard let self, self.session?.id == id else { return }
            self.finish(result)
        }
        serviceDelegate = delegate
        return delegate
    }

    public func sharingServicePicker(_ sharingServicePicker: NSSharingServicePicker,
                                     didChoose sharingService: NSSharingService?) {
        guard sharingServicePicker === picker else { return }
        if sharingService == nil { finish(.failure(PluginError.cancelled)) }
    }
}
#endif
