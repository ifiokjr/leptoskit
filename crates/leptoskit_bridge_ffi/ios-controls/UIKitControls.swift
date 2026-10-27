import LeptosKitBridge
import UIKit

@_silgen_name("fixture_reset") private func fixtureReset(_ mode: Int32)
@_silgen_name("fixture_releases") private func fixtureReleases(_ identifier: Int32) -> Int32
@_silgen_name("fixture_polls") private func fixturePolls(_ identifier: Int32) -> Int32
@_silgen_name("fixture_hold") private func fixtureHold(_ value: Int32)
@_silgen_name("fixture_finish") private func fixtureFinish()

@_silgen_name("fixture_async_started") private func fixtureAsyncStarted() -> Int32
@_silgen_name("fixture_async_cancelled") private func fixtureAsyncCancelled() -> Int32
@_silgen_name("fixture_async_finish") private func fixtureAsyncFinish()

@MainActor
@main
final class UIKitControls: UIResponder, UIApplicationDelegate {
    var window: UIWindow?
    private var failures = 0
    private let output = URL.documentsDirectory.appending(path: "result.txt")

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions options: [UIApplication.LaunchOptionsKey: Any]?
    ) -> Bool {
        window = UIWindow(frame: UIScreen.main.bounds)
        window?.rootViewController = UIViewController()
        window?.makeKeyAndVisible()
        Task { await run() }

        return true
    }

    private func record(_ condition: Bool, _ message: String) {
        let line = "\(condition ? "PASS" : "FAIL") \(message)\n"

        if !condition { failures += 1 }
        print(line, terminator: "")
        do {
            let previous = try String(contentsOf: output, encoding: .utf8)
            try (previous + line).write(to: output, atomically: true, encoding: .utf8)
        } catch {
            fatalError("Cannot record control result: \(error)")
        }
    }

    private func until(_ predicate: () -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + .seconds(4)

        while !predicate() && ContinuousClock.now < deadline {
            await Task.yield()
            window?.layoutIfNeeded()
        }

        return predicate()
    }

    private func descendants(_ view: UIView) -> [UIView] {
        [view] + view.subviews.flatMap { descendants($0) }
    }

    private func status(_ controller: UIViewController) -> String {
        descendants(controller.view).compactMap { $0 as? UILabel }
            .first { $0.accessibilityIdentifier == "leptoskit-update-status" }?.text ?? ""
    }

    private func button(_ controller: UIViewController) -> UIButton {
        guard
            let button = descendants(controller.view).compactMap({ $0 as? UIButton })
                .first(where: { $0.accessibilityIdentifier == "leptoskit-update-action" })
        else {
            fatalError("Missing actual update control")
        }

        return button
    }

    private func mount(_ mode: Int32) throws -> LeptosKitApplicationController {
        fixtureReset(mode)
        let controller = try LeptosKitApplicationController(
            embedded: Data("fixture".utf8), configuration: Data(#"{"enabled":true}"#.utf8),
            register: { _, _, _ in }
        )
        window?.rootViewController = controller
        controller.loadViewIfNeeded()
        window?.layoutIfNeeded()

        return controller
    }

    private func rgba(_ color: UIColor?) -> [Int] {
        var red: CGFloat = 0, green: CGFloat = 0, blue: CGFloat = 0, alpha: CGFloat = 0
        guard color?.getRed(&red, green: &green, blue: &blue, alpha: &alpha) == true else { return [] }

        return [red, green, blue, alpha].map { Int(($0 * 255).rounded()) }
    }

    private func advance(_ controller: LeptosKitApplicationController) async {
        button(controller).sendActions(for: .touchUpInside)
        _ = await until { self.button(controller).isEnabled }
        button(controller).sendActions(for: .touchUpInside)
    }

    private func coldCandidate() async throws {
        let controller = try mount(1)
        let recovered = await until { status(controller).contains("Running 2") }
        record(recovered, "cold joined-error candidate recovers to visible guest 2")
        record(controller.children.count == 1, "cold recovery detaches failed child")
        record(fixtureReleases(1) == 1, "cold joined error releases exactly once")
        record(status(controller).contains("-88"), "cold recovery preserves close diagnostic")
        record(
            status(controller).contains("candidate service failed"),
            "cold recovery preserves primary diagnostic")
        record(controller.children.first?.view.alpha == 1, "recovered guest is visible")

        record(
            guestLabels(controller).contains("Guest 2"), "cold recovery exposes guest 2 controls")

        do { try await controller.closeAndWait() } catch {
            record(false, "fallback scene closes successfully")
        }
    }

    private func replacement() async throws {
        let controller = try mount(2)
        record(
            await until { status(controller).contains("Running 1") },
            "initial actual UIKit guest is ready")
        record(
            controller.view.backgroundColor == .systemBackground
                && controller.preferredStatusBarStyle == .default,
            "a guest root without a background keeps the system canvas")
        fixtureHold(1)
        await advance(controller)
        record(
            await until { controller.children.count == 2 },
            "replacement keeps both children during pending join")
        record(await until { fixturePolls(1) > 0 }, "old guest close actually polls pending")
        record(fixtureReleases(1) == 0, "pending old interpreter is retained")
        fixtureFinish()
        let replaced = await until { status(controller).contains("Running 2") }
        record(replaced, "healthy confirmed replacement survives old joined error")
        record(controller.children.count == 1, "replacement detaches closed old active")
        record(fixtureReleases(1) == 1, "replacement releases old active once")
        record(status(controller).contains("-88"), "replacement keeps joined-error diagnostic")
        record(
            controller.children.first?.view.isUserInteractionEnabled == true,
            "replacement remains interactive")

        record(
            controller.children.first?.view.accessibilityElementsHidden == false,
            "replacement is accessible")
        record(guestLabels(controller).contains("Guest 2"), "replacement exposes guest 2 controls")
        record(
            rgba(controller.view.backgroundColor) == [2, 6, 23, 255],
            "the confirmed guest paints its root background behind the safe areas")
        record(
            controller.preferredStatusBarStyle == .lightContent,
            "a dark canvas uses light status bar content")

        do { try await controller.closeAndWait() } catch {
            record(false, "replacement scene closes successfully")
        }
    }

    /// Both waiters must join the same children despite one caller cancellation.
    private func scene() async throws {
        let controller = try mount(2)
        _ = await until { status(controller).contains("Running 1") }
        fixtureHold(1)
        await advance(controller)
        _ = await until { controller.children.count == 2 }
        weak let old = controller.children.first
        weak let candidate = controller.children.last
        let first = Task { try await controller.closeAndWait() }

        let second = Task { try await controller.closeAndWait() }
        first.cancel()

        await Task.yield()
        record(await until { fixturePolls(2) > 0 }, "scene pending candidate join actually polls")
        record(
            controller.children.count == 2,
            "scene retains pending children after caller cancellation")
        record(
            fixtureReleases(1) == 0 && fixtureReleases(2) == 0,
            "scene has no premature native releases")
        fixtureFinish()

        for waiter in [first, second] {
            await expectJoinedFailure(waiter)
        }

        record(controller.children.isEmpty, "scene detaches every completed child despite error")
        record(
            await until { old == nil && candidate == nil },
            "scene clears both completed guest ownership slots")
        record(
            fixtureReleases(1) == 1 && fixtureReleases(2) == 1,
            "scene awaits and releases both children once")

        do {
            try await controller.closeAndWait()
            record(false, "repeat scene close preserves error")
        } catch {
            record(
                error.localizedDescription.contains("-88"),
                "repeat scene close returns stable error")
        }
    }

    private func expectJoinedFailure(_ waiter: Task<Void, Error>) async {
        do {
            try await waiter.value
            record(false, "scene preserves joined failure")
        } catch {
            record(
                error.localizedDescription.contains("-88"),
                "scene waiter observes original close error")
        }
    }

    private func guestLabels(_ controller: UIViewController) -> [String] {
        controller.children.flatMap { descendants($0.view) }.compactMap { ($0 as? UILabel)?.text }
    }

    private func normalScene() async throws {
        let controller = try mount(0)
        record(
            await until { status(controller).contains("Running 1") }, "normal guest becomes ready")
        fixtureHold(1)
        let waiter = Task { try await controller.closeAndWait() }
        waiter.cancel()

        await Task.yield()
        record(await until { fixturePolls(1) > 0 }, "normal cancelled close actually polls pending")
        record(fixtureReleases(1) == 0, "normal cancelled waiter retains pending interpreter")
        fixtureFinish()
        try await waiter.value
        record(controller.children.isEmpty, "normal cancelled waiter still completes scene cleanup")

        try await controller.closeAndWait()
        record(fixtureReleases(1) == 1, "normal duplicate close releases once")

        let unawaited = try mount(0)
        _ = await until { status(unawaited).contains("Running 1") }
        unawaited.close()
        record(
            await until { unawaited.children.isEmpty }, "scene cleanup completes without a waiter")
        try await unawaited.closeAndWait()
    }

    private func asyncRequestDuringClose() async throws {
        let controller = try mount(3)
        record(
            await until { fixtureAsyncStarted() == 1 },
            "service-first runtime admits upstream async operation")
        fixtureHold(1)
        var joined = false
        let waiter = Task {
            try await controller.closeAndWait()
            joined = true
        }

        record(
            await until { fixtureAsyncCancelled() == 1 },
            "scene close cancels admitted async operation")
        record(await until { fixturePolls(1) > 0 }, "combined service close actually polls Pending")
        record(
            !joined && controller.children.count == 1,
            "combined owner retains child until both joins")
        record(fixtureReleases(1) == 0, "combined Pending service retains native owner")
        fixtureFinish()
        record(
            await until { fixtureReleases(1) == 1 },
            "combined service join completes while async callback remains Pending"
        )
        let prematurelyJoined = await until { joined }
        record(
            !prematurelyJoined && controller.children.count == 1,
            "service completion cannot bypass Pending async callback join"
        )
        fixtureAsyncFinish()
        try await waiter.value

        record(
            joined && controller.children.isEmpty, "combined async and service joins retire child")
        record(fixtureReleases(1) == 1, "combined close releases native service exactly once")
    }

    private func run() async {
        do {
            try "".write(to: output, atomically: true, encoding: .utf8)
            try await coldCandidate()
            try await replacement()
            try await scene()
            try await normalScene()
            try await asyncRequestDuringClose()
        } catch { record(false, "unexpected control error: \(error)") }
        record(failures == 0, "ALL CONTROLS COMPLETE")
    }
}
