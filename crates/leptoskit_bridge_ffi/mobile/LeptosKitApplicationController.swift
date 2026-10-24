import Foundation
import LeptosKitBridge
import UIKit

/// Owns one mobile scene and keeps the working guest alive until its replacement
/// has rendered native controls and Rust has durably confirmed the launch.
@MainActor
public final class LeptosKitApplicationController: UIViewController {
    private struct Configuration: Decodable { let enabled: Bool }
    private let embedded: Data
    private let register: (OpaquePointer, NativeDispatcher, NativePluginContext) throws -> Void
    private let releases: NativeReleases?
    private let chrome = MobileApplicationChrome()
    // The active guest owns orientation policy (its plugin context); UIKit asks
    // only this container, so forward it and re-query whenever the guest changes.
    private var active: LeptosKitViewController? {
        didSet { setNeedsUpdateOfSupportedInterfaceOrientations() }
    }
    private var pending: LeptosKitViewController?
    private var operation: Task<Void, Never>?
    private var staged = false
    private var closed = false
    private var teardown: Task<Void, Error>?
    private var pendingFailure: Error?
    private var pendingRecovery: Error?
    private var confirming = false

    public init(
        embedded: Data, configuration: Data,
        register: @escaping (OpaquePointer, NativeDispatcher, NativePluginContext) throws -> Void
    ) throws {
        self.embedded = embedded
        self.register = register
        let enabled = try JSONDecoder().decode(Configuration.self, from: configuration).enabled
        releases = enabled ? NativeReleases(configuration: configuration, embedded: embedded) : nil
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { nil }

    public override var supportedInterfaceOrientations: UIInterfaceOrientationMask {
        active?.supportedInterfaceOrientations ?? super.supportedInterfaceOrientations
    }

    public override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground
        chrome.content.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(chrome.content)
        NSLayoutConstraint.activate([
            chrome.content.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            chrome.content.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            chrome.content.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor)
        ])

        if releases != nil {
            chrome.configureToolbar(in: view) { [weak self] in self?.updateTapped() }
            launch()

        } else {
            chrome.content.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor).isActive = true
            mountEmbedded()
        }
    }

    private func mountEmbedded() {
        do {
            let wasm = try NativeReleases.entry(of: embedded)
            let controller = try LeptosKitViewController(wasm: wasm, register: register)
            attach(controller)
            active = controller
        } catch { chrome.showError(error, hasActive: active != nil) }
    }

    /// Call on scene disconnection. Network work has a bounded Rust timeout;
    /// queued destruction retains its owner and never blocks the UI thread.
    public func close() {
        guard !closed else { return }
        closed = true
        operation?.cancel()
        operation = nil
        let controllers = [pending, active].compactMap { $0 }

        for controller in controllers { controller.close() }
        let releases = releases
        teardown = Task { [weak self] in
            let failure = await MobileApplicationTeardown.join(controllers, releases: releases)
            self?.pending = nil
            self?.active = nil
            self?.pendingRecovery = nil

            if let failure { throw failure }
        }
    }

    /// Waits for retained teardown, including when the scene's calling task cancels.
    public func closeAndWait() async throws {
        close()
        try await teardown?.value
    }

    /// A staged update activates directly; otherwise the store synchronizes first.
    private func updateTapped() {
        guard operation == nil, let releases, !closed else { return }

        if staged {
            staged = false
            chrome.update.setTitle("Check for update", for: .normal)
            launch()

            return
        }

        chrome.update.isEnabled = false
        chrome.status.text = "Checking for update…"

        operation = runReleaseOperation { [weak self] in
            let report = try await releases.sync()
            self?.didSync(report)
        } onFailure: { [weak self] error in self?.didFailSync(error) }
    }

    private func didSync(_ report: NativeSyncReport) {
        guard !closed, !Task.isCancelled else { return }
        staged = report.status == .staged
        chrome.status.text = staged ? "Update \(report.version) (\(report.build)) downloaded" : "Up to date"
        chrome.update.setTitle(staged ? "Activate update" : "Check for update", for: .normal)
        chrome.update.isEnabled = true
        operation = nil
    }

    private func didFailSync(_ error: Error) {
        guard !closed, !Task.isCancelled else { return }
        chrome.status.text = error.localizedDescription
        chrome.update.isEnabled = true
        operation = nil
    }

    private func launch() {
        guard let releases, !closed else { return }
        chrome.update.isEnabled = false
        chrome.status.text = "Loading application…"
        operation = runReleaseOperation { [weak self] in
            let launch = try await releases.launch()
            self?.didLoadLaunch(launch)
        } onFailure: { [weak self] error in self?.didFailLaunch(error) }
    }

    private func didLoadLaunch(_ launch: NativeLaunchBundle) {
        guard !closed, !Task.isCancelled else { return }
        operation = nil
        mount(launch)
    }

    private func didFailLaunch(_ error: Error) {
        guard !closed, !Task.isCancelled else { return }
        operation = nil
        runBundledIfIdle(after: error)
    }

    private func mount(_ launch: NativeLaunchBundle, recovery: Error? = nil) {
        pendingFailure = nil
        pendingRecovery = recovery
        confirming = false
        do {
            let controller = try LeptosKitViewController(wasm: launch.wasm, register: register)
            pending = controller
            controller.onReady = { [weak self, weak controller] in
                self?.candidateReady(controller, launch.metadata)
            }
            controller.onFailure = { [weak self, weak controller] error in
                self?.candidateFailed(controller, launch.metadata, error)
            }
            // A candidate renders underneath, invisible and hidden from
            // accessibility, while the confirmed guest keeps the screen and stays
            // interactive. Confirmation swaps them in one main-thread turn, so no
            // frame or accessibility snapshot shows both trees.
            controller.view.alpha = active == nil ? 1 : 0
            controller.view.accessibilityElementsHidden = active != nil
            attach(controller, below: active)
        } catch { recover(launch.metadata, error: error) }
    }

    private func candidateReady(_ controller: LeptosKitViewController?, _ launch: NativeLaunch) {
        guard let controller, pending === controller else { return }
        confirm(controller, launch)
    }

    private func candidateFailed(_ controller: LeptosKitViewController?, _ launch: NativeLaunch, _ error: Error) {
        guard let controller, pending === controller else { return }
        pendingFailure = error

        guard !confirming else {
            chrome.status.text = "Application failed while confirming: \(error.localizedDescription)"

            return
        }

        recover(launch, error: error)
    }

    private func confirm(_ controller: LeptosKitViewController, _ launch: NativeLaunch) {
        guard let releases, operation == nil, !confirming else { return }
        confirming = true
        controller.view.isUserInteractionEnabled = false
        operation = runReleaseOperation { [weak self, weak controller] in
            try await releases.confirm(launch.receipt)
            await self?.commitCandidate(controller, launch)
        } onFailure: { [weak self] error in self?.confirmationFailed(launch, error) }
    }

    /// Recheck the candidate after every suspension before making it visible.
    private func commitCandidate(_ controller: LeptosKitViewController?, _ launch: NativeLaunch) async {
        guard let controller, canConfirm(controller, launch) else { return }
        var cleanupFailure: Error?

        if let active {
            cleanupFailure = await remove(active)
        }

        guard canConfirm(controller, launch) else { return }
        operation = nil
        confirming = false
        active = controller
        pending = nil
        controller.onReady = nil
        controller.onFailure = { [weak self] error in self?.showError(error) }
        controller.view.isUserInteractionEnabled = true

        controller.view.alpha = 1
        controller.view.accessibilityElementsHidden = false
        UIAccessibility.post(notification: .screenChanged, argument: nil)
        chrome.status.text = "Running \(launch.version) (\(launch.build))"

        presentDiagnostics(launch, cleanupFailure)
        pendingRecovery = nil
        chrome.update.isEnabled = true
    }

    private func canConfirm(_ controller: LeptosKitViewController, _ launch: NativeLaunch) -> Bool {
        guard !closed, !Task.isCancelled, pending === controller else { return false }

        guard let pendingFailure else { return true }
        operation = nil
        confirming = false
        recover(launch, error: pendingFailure)

        return false
    }

    private func confirmationFailed(_ launch: NativeLaunch, _ error: Error) {
        guard !closed, !Task.isCancelled else { return }
        operation = nil
        confirming = false
        recover(launch, error: error)
    }

    private func recover(_ launch: NativeLaunch, error: Error) {
        let failed = pending
        failed?.close()
        chrome.status.text = "Application failed: \(error.localizedDescription)"
        operation = Task { [weak self] in await self?.recoverLaunch(failed, launch, error) }
    }

    /// Join the failed candidate before asking the release store for recovery.
    private func recoverLaunch(_ failed: LeptosKitViewController?, _ launch: NativeLaunch, _ error: Error) async {
        var failure = error

        if let failed, let cleanup = await remove(failed) {
            failure = MobileApplicationFailure(primary: error, cleanup: cleanup)
        }

        guard !closed, !Task.isCancelled else { return }

        do {
            try await rejectCandidate(launch, failure)
        } catch { didFailLaunch(MobileApplicationFailure(primary: failure, cleanup: error)) }
    }

    /// Reject only candidates; a running confirmed guest remains interactive.
    private func rejectCandidate(_ launch: NativeLaunch, _ error: Error) async throws {
        guard launch.candidate, let releases else {
            finishRecovery(error)

            return
        }

        try await releases.reject(launch.receipt)

        guard !closed, !Task.isCancelled else { return }

        guard active == nil else {
            operation = nil
            chrome.status.text = "Update could not start: \(error.localizedDescription)"
            chrome.update.isEnabled = true

            return
        }

        let fallback = try await releases.launch()

        guard !closed, !Task.isCancelled else { return }
        operation = nil
        mount(fallback, recovery: error)
    }

    /// The update store failed before any guest ran. The store-shipped guest still
    /// works without it, so run that rather than leave the scene empty.
    private func runBundledIfIdle(after error: Error) {
        guard active == nil, pending == nil else {
            chrome.showError(error, hasActive: active != nil)

            return
        }

        chrome.status.text = "Updates unavailable: \(error.localizedDescription)"
        chrome.update.isEnabled = false
        do {
            let wasm = try NativeReleases.entry(of: embedded)
            let controller = try LeptosKitViewController(wasm: wasm, register: register)
            controller.onFailure = { [weak self] error in self?.showError(error) }
            attach(controller)
            active = controller
        } catch { chrome.showError(error, hasActive: active != nil) }
    }
}

// Child-controller and error presentation helpers, kept outside the class body.
private extension LeptosKitApplicationController {
    func presentDiagnostics(_ launch: NativeLaunch, _ cleanup: Error?) {
        if let recovery = launch.recovery { chrome.status.text?.append("\nRecovered: \(recovery)") }

        if let pendingRecovery { chrome.status.text?.append("\nRecovered: \(pendingRecovery.localizedDescription)") }

        if let cleanup { chrome.status.text?.append("\nPrevious guest cleanup: \(cleanup.localizedDescription)") }
    }

    private func finishRecovery(_ error: Error) {
        operation = nil

        if active == nil { chrome.showError(error, hasActive: active != nil) }
        chrome.update.isEnabled = active != nil
    }

    func showError(_ error: Error) {
        chrome.showError(error, hasActive: active != nil)
    }

    /// Mounts a guest in the content area, below `sibling` when one is given.
    func attach(_ controller: LeptosKitViewController, below sibling: LeptosKitViewController? = nil) {
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

    /// A thrown native close is a completed join; ownership cleanup still finishes.
    func remove(_ controller: LeptosKitViewController) async -> Error? {
        let failure = await MobileApplicationTeardown.remove(controller)

        if pending === controller { pending = nil }

        if active === controller { active = nil }

        return failure
    }
}

/// Keep the task's error boundary outside view ownership and route every failure.
@MainActor
private func runReleaseOperation(
    _ operation: @escaping @MainActor () async throws -> Void,
    onFailure: @escaping @MainActor (Error) -> Void
) -> Task<Void, Never> {
    Task {
        do { try await operation() } catch { onFailure(error) }
    }
}
