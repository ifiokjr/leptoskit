import Foundation
import UIKit

/// A completed native join releases child ownership even when it reports failure.
@MainActor
enum MobileApplicationTeardown {
    static func remove(_ controller: LeptosKitViewController) async -> Error? {
        defer { detach(controller) }

        do {
            try await controller.closeAndWait()

            return nil
        } catch { return error }
    }

    static func join(_ controllers: [LeptosKitViewController], releases: NativeReleases?) async
        -> Error?
    {
        var failure: Error?

        for controller in controllers {
            if let error = await remove(controller) {
                failure =
                    failure.map { MobileApplicationFailure(primary: $0, cleanup: error) } ?? error
            }
        }

        releases?.close()

        return failure
    }

    private static func detach(_ controller: LeptosKitViewController) {
        guard controller.parent != nil else { return }
        controller.willMove(toParent: nil)
        controller.view.removeFromSuperview()
        controller.removeFromParent()
    }
}

/// Keep the guest failure and every completed teardown diagnostic visible.
struct MobileApplicationFailure: LocalizedError {
    let primary: Error
    let cleanup: Error
    var errorDescription: String? {
        "\(primary.localizedDescription)\nCleanup: \(cleanup.localizedDescription)"
    }
}
