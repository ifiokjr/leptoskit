import Foundation
import LeptosKitBridge
@preconcurrency import LocalAuthentication
#if os(iOS)
import UIKit
#else
import AppKit
#endif

@MainActor
public final class LocalAuthenticationPlugin: NSObject, NativePlugin {
    public let name = "local-authentication"
    public let methods: [UInt32] = [0, 1, 2]
    private var closed = false
    private var active: Session?

    private struct Session {
        let token: UUID
        let context: LAContext
        let policy: UInt32
        let completion: CheckedContinuation<Data, any Error>
    }

    /// The wire methods the Rust crate calls.
    private enum Method: UInt32 {
        case capabilities = 0
        case authenticate = 1
        case cancel = 2
    }

    public override init() {
        super.init()
        NotificationCenter.default.addObserver(
            self, selector: #selector(hostEnteredBackground), name: Self.departure, object: nil
        )
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
    }

    /// The notification the host posts when the app can no longer show UI.
    private static var departure: Notification.Name {
        #if os(iOS)

        return UIApplication.didEnterBackgroundNotification
        #else

        return NSApplication.didHideNotification
        #endif
    }

    @objc private func hostEnteredBackground() {
        // Losing focus to the OS authentication UI is not a background transition.
        cancel(transport: true)
    }

    private var foreground: Bool {
        #if os(iOS)

        return UIApplication.shared.applicationState == .active
        #else

        return NSApplication.shared.isActive && !NSApplication.shared.isHidden
        #endif
    }

    /// Answers one call: a capability query never prompts, and cancel only dismisses
    /// this plugin's own prompt.
    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        switch Method(rawValue: method) {
        case .capabilities: return try capabilities(policy: payload)
        case .authenticate: return try await authenticate(AuthenticationRequest(payload: payload))
        case .cancel: return try cancelPrompt(payload)
        case nil: throw PluginError.unavailable
        }
    }

    public func close() {
        closed = true
        cancel(transport: true)
    }

    /// Reports what the OS can do for a policy, without showing a prompt.
    private func capabilities(policy payload: NativeBuffer) throws -> Data {
        let policy = try decodePolicy(payload)
        let context = LAContext()
        defer { context.invalidate() }
        var error: NSError?
        let available = context.canEvaluatePolicy(nativePolicy(policy), error: &error)
        let status = available ? Readiness.ready : readiness(for: error)
        let sensor = modality(context.biometryType)

        return authenticationFields([1, policy, status, sensor, deviceSecure()])
    }

    /// Whether a device credential is set: yes, no, or unknown.
    private func deviceSecure() -> UInt32 {
        let context = LAContext()
        defer { context.invalidate() }
        var error: NSError?

        if context.canEvaluatePolicy(.deviceOwnerAuthentication, error: &error) { return 2 }

        return error?.code == LAError.passcodeNotSet.rawValue ? 1 : 0
    }

    private func nativePolicy(_ policy: UInt32) -> LAPolicy {
        policy == 0 ? .deviceOwnerAuthenticationWithBiometrics : .deviceOwnerAuthentication
    }

    /// Shows the system prompt and answers with its terminal outcome.
    private func authenticate(_ request: AuthenticationRequest) async throws -> Data {
        try checkReady()

        let context = makeContext(request)
        var error: NSError?

        let available = context.canEvaluatePolicy(nativePolicy(request.policy), error: &error)

        try checkFaceIDDescription(context)

        if !available {
            context.invalidate()

            return try failure(error)
        }

        return try await evaluate(context, request: request) { completion in
            prompt(with: context, request: request, completion: completion)
        }
    }

    /// Rejects a call the OS prompt cannot serve, before any context exists.
    private func checkReady() throws {
        if Task.isCancelled { throw PluginError.cancelled }

        if active != nil { throw PluginError.busy }

        if !foreground { throw PluginError.unavailable }
    }

    /// A context configured with the prompt text the caller asked for.
    private func makeContext(_ request: AuthenticationRequest) -> LAContext {
        let context = LAContext()
        context.localizedCancelTitle = request.cancelLabel
        context.touchIDAuthenticationAllowableReuseDuration = 0

        if request.policy == 0 { context.localizedFallbackTitle = "" }

        return context
    }

    /// Hands the prompt to the OS, which reports back through `completion`.
    private func prompt(
        with context: LAContext, request: AuthenticationRequest,
        completion: @escaping @Sendable (Bool, (any Error)?) -> Void
    ) {
        context.evaluatePolicy(nativePolicy(request.policy), localizedReason: request.reason, reply: completion)
    }

    #if os(iOS)
    /// Face ID needs a usage description before Apple shows the prompt.
    private func checkFaceIDDescription(_ context: LAContext) throws {
        guard context.biometryType == .faceID else { return }
        let purpose = Bundle.main.object(forInfoDictionaryKey: "NSFaceIDUsageDescription") as? String

        guard let purpose, !purpose.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            context.invalidate()
            throw PluginError.invalidArgument
        }
    }
    #else
    /// macOS has no biometric that requires a usage description.
    private func checkFaceIDDescription(_ context: LAContext) {}
    #endif

    /// Keeps lifecycle/cancellation independent from the system prompt entry point.
    func evaluate(
        _ context: LAContext, request: AuthenticationRequest,
        start: (@escaping @Sendable (Bool, (any Error)?) -> Void) -> Void
    ) async throws -> Data {
        let token = UUID()

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { completion in
                begin(token, context, request, completion, start)
            }
        } onCancel: {
            Task { @MainActor [weak self] in self?.cancelIfCurrent(token) }
        }
    }

    /// Cancels the session the caller abandoned, if it is still the one that runs.
    private func cancelIfCurrent(_ token: UUID) {
        guard active?.token == token else { return }

        cancel(transport: true)
    }

    /// Registers the session and starts the OS hop, or refuses without showing UI.
    private func begin(
        _ token: UUID,
        _ context: LAContext,
        _ request: AuthenticationRequest,
        _ completion: CheckedContinuation<Data, any Error>,
        _ start: (@escaping @Sendable (Bool, (any Error)?) -> Void) -> Void
    ) {
        guard let refusal = register(token, context, request, completion) else {
            start { [weak self] success, error in
                Task { @MainActor [weak self] in self?.complete(token, success: success, error: error) }
            }

            return
        }

        context.invalidate()
        completion.resume(throwing: refusal)
    }

    /// Records the session, or says why the prompt must not start.
    private func register(
        _ token: UUID,
        _ context: LAContext,
        _ request: AuthenticationRequest,
        _ completion: CheckedContinuation<Data, any Error>
    ) -> PluginError? {
        if Task.isCancelled || closed { return PluginError.cancelled }

        if active != nil { return PluginError.busy }

        active = Session(token: token, context: context, policy: request.policy, completion: completion)

        return nil
    }

    private func complete(_ token: UUID, success: Bool, error: (any Error)?) {
        guard let session = take(token) else { return }

        do {
            let result = success ? authenticationFields([1, 0, session.policy == 0 ? 1 : 0]) : try failure(error)
            session.completion.resume(returning: result)
        } catch {
            session.completion.resume(throwing: error)
        }
    }

    private func cancel(transport: Bool) {
        guard let session = take(nil) else { return }

        if transport {
            session.completion.resume(throwing: PluginError.cancelled)
        } else {
            session.completion.resume(returning: authenticationFields([1, Outcome.appCancelled, 0]))
        }
    }

    /// Takes the session back, so a late answer cannot revive a finished prompt.
    private func take(_ token: UUID?) -> Session? {
        guard let session = active else { return nil }

        if let token, token != session.token { return nil }
        active = nil
        session.context.invalidate()

        return session
    }

    /// Dismisses this plugin's prompt and answers with empty bytes.
    private func cancelPrompt(_ payload: NativeBuffer) throws -> Data {
        guard payload.count == 0 else { throw PluginError.invalidArgument }

        cancel(transport: false)

        return Data()
    }

    private func failure(_ error: (any Error)?) throws -> Data {
        let outcome = try outcome(for: error as NSError?)

        return authenticationFields([1, outcome, 0])
    }
}
