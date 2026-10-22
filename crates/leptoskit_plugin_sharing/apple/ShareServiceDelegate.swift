#if canImport(AppKit)
import AppKit
import LeptosKitBridge

/// Each selection gets its own delegate; a late service callback retains its
/// original session identity even if AppKit reuses the same service object.
@MainActor
final class ShareServiceDelegate: NSObject, NSSharingServiceDelegate {
    private let context: NativePluginContext
    private let completed: (Result<Data, Error>) -> Void

    init(context: NativePluginContext, completed: @escaping (Result<Data, Error>) -> Void) {
        self.context = context
        self.completed = completed
    }

    func sharingService(_ sharingService: NSSharingService, didShareItems items: [Any]) {
        completed(.success(Data()))
    }

    func sharingService(_ sharingService: NSSharingService, didFailToShareItems items: [Any], error: any Error) {
        let error = error as NSError
        completed(.failure(error.domain == NSCocoaErrorDomain && error.code == NSUserCancelledError
                           ? PluginError.cancelled : PluginError.native))
    }

    func sharingService(_ sharingService: NSSharingService, sourceWindowForShareItems items: [Any],
                        sharingContentScope: UnsafeMutablePointer<NSSharingService.SharingContentScope>) -> NSWindow? {
        return try? context.requirePresentationOwner().view.window
    }
}
#endif
