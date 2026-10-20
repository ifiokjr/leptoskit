import UIKit
import UserNotifications

final class CaptureViewController: UIViewController {
    private let status = UILabel()
    private let name = UITextField()
    private var count = 0

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = UIColor(red: 0.96, green: 0.96, blue: 0.99, alpha: 1)
        let title = UILabel()
        title.text = "Native capture lab"
        title.font = .boldSystemFont(ofSize: 30)
        title.textColor = .systemIndigo
        let subtitle = UILabel()
        subtitle.text = "Real text, native controls, system dialogs."
        subtitle.numberOfLines = 0
        name.borderStyle = .roundedRect
        name.placeholder = "Your name"
        name.accessibilityIdentifier = "name"
        status.text = "Ready"
        status.accessibilityIdentifier = "status"
        status.numberOfLines = 0
        let increment = button("Increment", id: "increment", action: #selector(incrementCount))
        let greet = button("Greet", id: "greet", action: #selector(greetUser))
        let permission = button("Enable notifications", id: "permission", action: #selector(requestPermission))
        let stack = UIStackView(arrangedSubviews: [title, subtitle, name, greet, increment, permission, status])
        stack.axis = .vertical
        stack.spacing = 24
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 24),
            stack.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -24),
            stack.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 32)
        ])
    }

    private func button(_ title: String, id: String, action: Selector) -> UIButton {
        let button = UIButton(type: .system)
        var configuration = UIButton.Configuration.filled()
        configuration.title = title
        configuration.baseBackgroundColor = .systemIndigo
        configuration.contentInsets = NSDirectionalEdgeInsets(top: 14, leading: 12, bottom: 14, trailing: 12)
        button.configuration = configuration
        button.accessibilityIdentifier = id
        button.addTarget(self, action: action, for: .touchUpInside)
        return button
    }

    @objc private func incrementCount() {
        view.endEditing(true)
        count += 1
        status.text = "Count: \(count)"
    }

    @objc private func greetUser() {
        view.endEditing(true)
        status.text = "Hello, \(name.text ?? "")"
    }

    @objc private func requestPermission() {
        view.endEditing(true)
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { granted, error in
            DispatchQueue.main.async {
                self.status.text = error.map { "Error: \($0.localizedDescription)" }
                    ?? (granted ? "Notifications allowed" : "Notifications denied")
            }
        }
    }
}

final class AppDelegate: UIResponder, UIApplicationDelegate {
    var window: UIWindow?
    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions options: [UIApplication.LaunchOptionsKey: Any]?
    ) -> Bool {
        window = UIWindow(frame: UIScreen.main.bounds)
        window?.rootViewController = CaptureViewController()
        window?.makeKeyAndVisible()
        return true
    }
}

UIApplicationMain(CommandLine.argc, CommandLine.unsafeArgv, nil, NSStringFromClass(AppDelegate.self))
