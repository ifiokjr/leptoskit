import Foundation
import LeptosKitBridge
import UIKit

/// Owns one mobile scene and keeps the working guest alive until its replacement
/// has rendered native controls and Rust has durably confirmed the launch.
@MainActor
public final class LeptosKitApplicationController: UIViewController {
    private struct Configuration: Decodable { let enabled: Bool }
    private let wasm: Data
    private let register: (OpaquePointer, NativeDispatcher) throws -> Void
    private let releases: NativeReleases?
    private let content = UIView()
    private let status = UILabel()
    private let update = UIButton(type: .system)
    private var active: LeptosKitViewController?
    private var pending: LeptosKitViewController?
    private var operation: Task<Void, Never>?
    private var staged = false
    private var closed = false
    private var pendingFailure: Error?
    private var confirming = false

    public init(
        wasm: Data, embedded: Data, configuration: Data,
        register: @escaping (OpaquePointer, NativeDispatcher) throws -> Void
    ) throws {
        self.wasm = wasm
        self.register = register
        let enabled = try JSONDecoder().decode(Configuration.self, from: configuration).enabled
        releases = enabled ? NativeReleases(configuration: configuration, embedded: embedded) : nil
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { nil }

    public override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground
        content.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(content)
        NSLayoutConstraint.activate([
            content.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            content.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            content.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor)
        ])
        if releases != nil {
            configureToolbar()
            launch()
        } else {
            content.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor).isActive = true
            do {
                let controller = try LeptosKitViewController(wasm: wasm, register: register)
                attach(controller)
                active = controller
            } catch { showError(error) }
        }
    }

    /// Call on scene disconnection. Network work has a bounded Rust timeout;
    /// queued destruction retains its owner and never blocks the UI thread.
    public func close() {
        guard !closed else { return }
        closed = true
        operation?.cancel()
        operation = nil
        pending?.close()
        active?.close()
        releases?.close()
    }

    private func configureToolbar() {
        status.numberOfLines = 0
        status.font = .preferredFont(forTextStyle: .caption1)
        status.accessibilityIdentifier = "leptoskit-update-status"
        update.accessibilityIdentifier = "leptoskit-update-action"
        update.setTitle("Check for update", for: .normal)
        update.isEnabled = false
        update.addAction(UIAction { [weak self] _ in self?.updateTapped() }, for: .touchUpInside)
        let toolbar = UIStackView(arrangedSubviews: [status, update])
        toolbar.axis = .vertical
        toolbar.spacing = 6
        toolbar.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(toolbar)
        NSLayoutConstraint.activate([
            toolbar.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 8),
            toolbar.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 16),
            toolbar.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -16),
            content.topAnchor.constraint(equalTo: toolbar.bottomAnchor, constant: 8)
        ])
    }

    private func updateTapped() {
        guard operation == nil, let releases, !closed else { return }
        if staged {
            staged = false
            update.setTitle("Check for update", for: .normal)
            launch()
            return
        }
        update.isEnabled = false
        status.text = "Checking for update…"
        operation = Task { [weak self] in
            do {
                let report = try await releases.sync()
                guard let self, !closed, !Task.isCancelled else { return }
                staged = report.status == .staged
                status.text = staged ? "Update \(report.version) (\(report.build)) downloaded" : "Up to date"
                update.setTitle(staged ? "Activate update" : "Check for update", for: .normal)
                update.isEnabled = true
                operation = nil
            } catch {
                guard let self, !closed, !Task.isCancelled else { return }
                status.text = error.localizedDescription
                update.isEnabled = true
                operation = nil
            }
        }
    }

    private func launch() {
        guard let releases, !closed else { return }
        update.isEnabled = false
        status.text = "Loading application…"
        operation = Task { [weak self] in
            do {
                let launch = try await releases.launch()
                guard let self, !closed, !Task.isCancelled else { return }
                operation = nil
                mount(launch)
            } catch {
                guard let self, !closed, !Task.isCancelled else { return }
                operation = nil
                showError(error)
            }
        }
    }

    private func mount(_ launch: NativeLaunchBundle) {
        pendingFailure = nil
        confirming = false
        do {
            let controller = try LeptosKitViewController(wasm: launch.wasm, register: register)
            pending = controller
            controller.onReady = { [weak self, weak controller] in
                guard let self, let controller, pending === controller else { return }
                confirm(controller, launch.metadata)
            }
            controller.onFailure = { [weak self, weak controller] error in
                guard let self, let controller, pending === controller else { return }
                pendingFailure = error
                if confirming {
                    status.text = "Application failed while confirming: \(error.localizedDescription)"
                    return
                }
                recover(launch.metadata, error: error)
            }
            active?.view.isUserInteractionEnabled = false
            attach(controller)
        } catch { recover(launch.metadata, error: error) }
    }

    private func confirm(_ controller: LeptosKitViewController, _ launch: NativeLaunch) {
        guard let releases, operation == nil, !confirming else { return }
        confirming = true
        controller.view.isUserInteractionEnabled = false
        operation = Task { [weak self, weak controller] in
            do {
                try await releases.confirm(launch.receipt)
                guard let self, let controller, !closed, !Task.isCancelled,
                    pending === controller
                else { return }
                operation = nil
                confirming = false
                if let pendingFailure {
                    recover(launch, error: pendingFailure)
                    return
                }
                if let active { remove(active) }
                active = controller
                pending = nil
                controller.onReady = nil
                controller.onFailure = { [weak self] error in self?.showError(error) }
                controller.view.isUserInteractionEnabled = true
                status.text = "Running \(launch.version) (\(launch.build))"
                if let recovery = launch.recovery { status.text?.append("\nRecovered: \(recovery)") }
                update.isEnabled = true
            } catch {
                guard let self, !closed, !Task.isCancelled else { return }
                operation = nil
                confirming = false
                recover(launch, error: error)
            }
        }
    }

    private func recover(_ launch: NativeLaunch, error: Error) {
        if let pending { remove(pending) }
        pending = nil
        active?.view.isUserInteractionEnabled = true
        status.text = "Application failed: \(error.localizedDescription)"
        guard launch.candidate, let releases else {
            if active == nil { showError(error) }
            update.isEnabled = active != nil
            return
        }
        operation = Task { [weak self] in
            do {
                try await releases.reject(launch.receipt)
                guard let self, !closed, !Task.isCancelled else { return }
                // The confirmed guest stayed interactive throughout the trial. Only a
                // cold-start candidate needs a fresh launch of the working release.
                if active != nil {
                    operation = nil
                    status.text = "Update could not start: \(error.localizedDescription)"
                    update.isEnabled = true
                    return
                }
                let fallback = try await releases.launch()
                guard !closed, !Task.isCancelled else { return }
                operation = nil
                mount(fallback)
            } catch {
                guard let self, !closed, !Task.isCancelled else { return }
                operation = nil
                showError(error)
            }
        }
    }

    private func attach(_ controller: LeptosKitViewController) {
        addChild(controller)
        controller.view.frame = content.bounds
        controller.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        content.addSubview(controller.view)
        controller.didMove(toParent: self)
    }

    private func remove(_ controller: LeptosKitViewController) {
        controller.close()
        controller.willMove(toParent: nil)
        controller.view.removeFromSuperview()
        controller.removeFromParent()
    }

    private func showError(_ error: Error) {
        status.text = error.localizedDescription
        update.isEnabled = active != nil
        guard active == nil else { return }
        let label = UILabel(frame: content.bounds.insetBy(dx: 20, dy: 20))
        label.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        label.numberOfLines = 0
        label.text = error.localizedDescription
        label.accessibilityIdentifier = "leptoskit-error"
        content.addSubview(label)
    }
}
