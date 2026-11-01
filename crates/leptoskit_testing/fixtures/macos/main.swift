import AppKit

final class CaptureController: NSObject {
    let window: NSWindow
    private let status = NSTextField(labelWithString: "Ready")
    private let name = NSTextField(string: "")
    private var count = 0

    override init() {
        window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 520, height: 460),
            styleMask: [.titled, .closable, .miniaturizable],
            backing: .buffered,
            defer: false
        )
        super.init()
        window.title = "Native capture lab"
        let title = NSTextField(labelWithString: "Native capture lab")
        title.font = .boldSystemFont(ofSize: 30)
        title.textColor = .systemIndigo
        let subtitle = NSTextField(wrappingLabelWithString: "Real text, native controls, system dialogs.")
        name.placeholderString = "Your name"
        name.setAccessibilityIdentifier("name")

        status.setAccessibilityIdentifier("status")
        let greet = button("Greet", id: "greet", action: #selector(greetUser))
        let increment = button("Increment", id: "increment", action: #selector(incrementCount))
        let permission = button("Choose evidence", id: "permission", action: #selector(chooseEvidence))
        let stack = NSStackView(views: [title, subtitle, name, greet, increment, permission, status])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 18
        stack.edgeInsets = NSEdgeInsets(top: 32, left: 24, bottom: 32, right: 24)
        stack.translatesAutoresizingMaskIntoConstraints = false
        let content = NSView()
        content.wantsLayer = true
        content.layer?.backgroundColor = NSColor(red: 0.96, green: 0.96, blue: 0.99, alpha: 1).cgColor
        content.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            stack.topAnchor.constraint(equalTo: content.topAnchor),
            name.widthAnchor.constraint(equalToConstant: 320)
        ])
        window.contentView = content
        window.center()
    }

    private func button(_ title: String, id: String, action: Selector) -> NSButton {
        let button = NSButton(title: title, target: self, action: action)
        button.bezelStyle = .push
        button.bezelColor = .systemIndigo
        button.setAccessibilityIdentifier(id)

        return button
    }

    @objc private func incrementCount() {
        count += 1
        status.stringValue = "Count: \(count)"
    }

    @objc private func greetUser() {
        status.stringValue = "Hello, \(name.stringValue)"
    }

    /// Presents the real open panel as a sheet. Pointing its directory at
    /// the evidence file selects that file, so "Open" is enabled at once.
    @objc private func chooseEvidence() {
        let panel = NSOpenPanel()
        panel.message = "Choose the evidence file"
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.directoryURL = Bundle.main.bundleURL
            .deletingLastPathComponent()
            .appendingPathComponent("evidence.txt")

        panel.beginSheetModal(for: window) { response in
            self.status.stringValue = response == .OK
                ? "Selected: \(panel.url?.lastPathComponent ?? "")"
                : "Cancelled"
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var controller: CaptureController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let controller = CaptureController()
        self.controller = controller
        controller.window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
