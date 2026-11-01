import Foundation
import LeptosKitBridge
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

/// The request framing this host reads.
private enum Wire {
    /// The only method: present the system share UI.
    static let share: UInt32 = 0
}

/// Shares one validated text or URL through the platform's own UI.
@MainActor
public final class SharingPlugin: NSObject, NativePlugin {
    public let name = "sharing"
    public let methods: [UInt32] = [Wire.share]
    private let context: NativePluginContext
    private var session: ShareSession?
    private var currentInvocation: UUID?
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

    /// Presents the share UI for one caller's content, and answers when it ends.
    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard method == Wire.share else { throw PluginError.unavailable }

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
                self?.cancel(invocation: invocation)
            }
        }
    }

    /// Cancels the presentation only while this call still owns it.
    private func cancel(invocation: UUID) {
        guard currentInvocation == invocation else { return }

        cancel()
    }

    private func cancel() {
        dismissChooser()
    }

    private func finish(_ result: Result<Data, Error>) {
        let pending = session
        session = nil
        currentInvocation = nil
        releaseChooser()
        pending?.complete(result)
    }

    #if canImport(UIKit)
    /// Presents the activity sheet and answers when it completes.
    private func present(_ content: ShareContent, owner: NativePresentationOwner) async throws -> Data {
        guard isIdle(owner) else { throw PluginError.busy }

        let activity = makeActivity(content, owner: owner)
        self.activity = activity

        return try await withCheckedThrowingContinuation { continuation in
            let session = ShareSession(continuation)
            self.session = session
            activity.completionWithItemsHandler = { [weak self] _, completed, _, error in
                self?.completed(session, completed: completed, error: error)
            }
            owner.present(activity, animated: true)
            activity.presentationController?.delegate = self
        }
    }

    /// A presenter that is already showing, dismissing, or presenting cannot host the sheet.
    private func isIdle(_ owner: NativePresentationOwner) -> Bool {
        owner.presentedViewController == nil && !owner.isBeingDismissed && !owner.isBeingPresented
            && owner.transitionCoordinator == nil
    }

    /// Anchors the sheet to the middle of its presenter, as a popover requires.
    private func makeActivity(_ content: ShareContent, owner: NativePresentationOwner) -> UIActivityViewController {
        let activity = UIActivityViewController(activityItems: [content.item], applicationActivities: nil)
        let bounds = owner.view.bounds
        let centre = CGRect(x: bounds.midX, y: bounds.midY, width: 1, height: 1)

        activity.popoverPresentationController?.sourceView = owner.view
        activity.popoverPresentationController?.sourceRect = centre

        return activity
    }

    /// Answers the one session the sheet belongs to.
    private func completed(_ session: ShareSession, completed: Bool, error: (any Error)?) {
        guard self.session?.id == session.id else { return }

        if error != nil {
            finish(.failure(PluginError.native))
        } else if completed {
            finish(.success(Data()))
        } else {
            finish(.failure(PluginError.cancelled))
        }
    }

    private func dismissChooser() {
        activity?.dismiss(animated: false)
        finish(.failure(PluginError.cancelled))
    }

    private func releaseChooser() {
        activity?.completionWithItemsHandler = nil
        activity = nil
    }
    #elseif canImport(AppKit)
    /// Presents the sharing picker beside the window's centre.
    private func present(_ content: ShareContent, owner: NativePresentationOwner) async throws -> Data {
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
            let anchor = NSRect(x: bounds.midX, y: bounds.midY, width: 1, height: 1)

            picker.show(relativeTo: anchor, of: owner.view, preferredEdge: .minY)
        }
    }

    /// AppKit cannot cancel a service after the user has chosen it.
    private func dismissChooser() {
        guard service == nil else {
            // Retain busy admission until the service ends; cancellation is not
            // a promise to retract content already handed to that service.
            session?.complete(.failure(PluginError.cancelled))

            return
        }

        picker?.close()
        finish(.failure(PluginError.cancelled))
    }

    private func releaseChooser() {
        picker?.delegate = nil
        picker = nil
        service?.delegate = nil
        service = nil
        serviceDelegate = nil
    }
    #else
    private func present(_ content: ShareContent, owner: NativePresentationOwner) async throws -> Data {
        throw PluginError.unavailable
    }

    private func dismissChooser() {}

    private func releaseChooser() {}
    #endif
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
    public func sharingServicePicker(
        _ sharingServicePicker: NSSharingServicePicker,
        delegateFor sharingService: NSSharingService
    ) -> (any NSSharingServiceDelegate)? {
        guard sharingServicePicker === picker, let id = session?.id else { return nil }

        service = sharingService
        let delegate = ShareServiceDelegate(context: context) { [weak self] result in
            self?.serviceEnded(id, result: result)
        }
        serviceDelegate = delegate

        return delegate
    }

    /// Answers the session the chosen service belongs to.
    private func serviceEnded(_ id: UUID, result: Result<Data, Error>) {
        guard session?.id == id else { return }

        finish(result)
    }

    public func sharingServicePicker(
        _ sharingServicePicker: NSSharingServicePicker,
        didChoose sharingService: NSSharingService?
    ) {
        guard sharingServicePicker === picker else { return }

        if sharingService == nil { finish(.failure(PluginError.cancelled)) }
    }
}
#endif
