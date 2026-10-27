import UIKit

// Mounting, swapping, and removing a scene's guests, shared by the release
// store and `leptoskit dev`.
extension LeptosKitApplicationController {
    /// Mounts a guest in the content area, below `sibling` when one is given.
    func attach(_ controller: LeptosKitViewController, below sibling: LeptosKitViewController? = nil) {
        controller.onCanvasColorChange = { [weak self, weak controller] in
            if let self, controller != nil, self.active === controller { self.activeChanged() }
        }
        addChild(controller)
        controller.view.frame = chrome.content.bounds
        controller.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]

        if let sibling {
            chrome.content.insertSubview(controller.view, belowSubview: sibling.view)
        } else {
            chrome.content.addSubview(controller.view)
        }

        controller.didMove(toParent: self)
    }

    /// A candidate renders underneath, invisible and hidden from accessibility, while the
    /// running guest keeps the screen and stays interactive. `reveal` swaps them in one
    /// main-thread turn, so no frame or accessibility snapshot shows both trees.
    func attachCandidate(_ controller: LeptosKitViewController) {
        pending = controller
        controller.view.alpha = active == nil ? 1 : 0
        controller.view.accessibilityElementsHidden = active != nil
        attach(controller, below: active)
    }

    /// Makes a rendered candidate the active guest; the caller removes the one it replaces.
    func reveal(_ controller: LeptosKitViewController) {
        active = controller
        pending = nil
        controller.onReady = nil
        controller.onFailure = { [weak self] error in self?.showError(error) }
        controller.view.isUserInteractionEnabled = true
        controller.view.alpha = 1
        controller.view.accessibilityElementsHidden = false
        UIAccessibility.post(notification: .screenChanged, argument: nil)
    }

    /// A thrown native close is a completed join; ownership cleanup still finishes.
    func remove(_ controller: LeptosKitViewController) async -> Error? {
        let failure = await MobileApplicationTeardown.remove(controller)

        if pending === controller { pending = nil }

        if active === controller { active = nil }

        return failure
    }
}
