import LeptosKitBridge
import UIKit

/// Owns scene status and update controls separately from guest replacement state.
@MainActor
final class MobileApplicationChrome {
    let content = UIView()
    let status = UILabel()
    let update = UIButton(type: .system)
    private static let toolbarSpacing: CGFloat = 6
    private static let toolbarInset: CGFloat = 8
    private static let toolbarMargin: CGFloat = 16
    private static let errorInset: CGFloat = 20

    func configureToolbar(in view: UIView, onUpdate: @escaping () -> Void) {
        status.numberOfLines = 0
        status.font = .preferredFont(forTextStyle: .caption1)
        status.accessibilityIdentifier = "leptoskit-update-status"
        update.accessibilityIdentifier = "leptoskit-update-action"
        update.setTitle("Check for update", for: .normal)
        update.isEnabled = false
        update.addAction(UIAction { _ in onUpdate() }, for: .touchUpInside)
        let toolbar = UIStackView(arrangedSubviews: [status, update])

        toolbar.axis = .vertical
        toolbar.spacing = Self.toolbarSpacing
        toolbar.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(toolbar)
        NSLayoutConstraint.activate([
            toolbar.topAnchor.constraint(
                equalTo: view.safeAreaLayoutGuide.topAnchor, constant: Self.toolbarInset),
            toolbar.leadingAnchor.constraint(
                equalTo: view.leadingAnchor, constant: Self.toolbarMargin),
            toolbar.trailingAnchor.constraint(
                equalTo: view.trailingAnchor, constant: -Self.toolbarMargin),
            content.topAnchor.constraint(
                equalTo: toolbar.bottomAnchor, constant: Self.toolbarInset),
        ])
    }

    func showError(_ error: Error, hasActive: Bool) {
        status.text = error.localizedDescription
        update.isEnabled = hasActive

        guard !hasActive else { return }
        let label = UILabel(frame: content.bounds.insetBy(dx: Self.errorInset, dy: Self.errorInset))
        label.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        label.numberOfLines = 0
        label.text = error.localizedDescription
        label.accessibilityIdentifier = "leptoskit-error"

        content.addSubview(label)
    }
}
