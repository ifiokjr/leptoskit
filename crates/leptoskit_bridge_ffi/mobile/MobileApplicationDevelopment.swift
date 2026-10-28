import Foundation
import UIKit

// `leptoskit dev` reloads, which swap guests without the release store.
extension LeptosKitApplicationController {
    /// Follows the guests `leptoskit dev` serves, starting after the one running.
    func followDevelopment(_ url: String, running: Data) -> NativeDevelopmentGuests {
        NativeDevelopmentGuests(url: url, running: running) { [weak self] wasm in self?.reload(wasm) }
    }

    /// Mounts a guest from `leptoskit dev` hidden below the running one and swaps
    /// them once it renders, so a reload never shows an empty screen. A newer
    /// guest replaces a candidate that has not rendered yet.
    private func reload(_ wasm: Data) {
        guard !closed else { return }

        if let stale = pending { discard(stale) }

        do {
            let controller = try LeptosKitViewController(wasm: wasm, register: register)
            controller.onReady = { [weak self, weak controller] in self?.promoteDevelopment(controller) }
            controller.onFailure = { [weak self, weak controller] error in
                self?.developmentFailed(controller, error)
            }
            attachCandidate(controller)
        } catch { NSLog("leptoskit dev: %@", error.localizedDescription) }
    }

    private func promoteDevelopment(_ controller: LeptosKitViewController?) {
        guard let controller, pending === controller, !closed else { return }
        let previous = active
        reveal(controller)
        NSLog("leptoskit dev: reloaded")

        guard let previous else { return }
        // The replaced guest sits above its successor until its teardown joins.
        previous.view.isHidden = true
        Task {
            if let failure = await remove(previous) {
                NSLog("leptoskit dev: previous guest cleanup: %@", failure.localizedDescription)
            }
        }
    }

    /// A candidate that fails keeps the running guest on screen.
    private func developmentFailed(_ controller: LeptosKitViewController?, _ error: Error) {
        NSLog("leptoskit dev: %@", error.localizedDescription)

        guard let controller, pending === controller else { return }
        discard(controller)
    }

    /// Tears down a candidate that will never be shown.
    private func discard(_ candidate: LeptosKitViewController) {
        pending = nil
        Task { _ = await remove(candidate) }
    }
}
