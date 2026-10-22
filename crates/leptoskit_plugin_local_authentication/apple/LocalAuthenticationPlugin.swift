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

    public override init() {
        super.init()
        #if os(iOS)
        let notification = UIApplication.didEnterBackgroundNotification
        #else
        let notification = NSApplication.didHideNotification
        #endif
        NotificationCenter.default.addObserver(
            self, selector: #selector(hostEnteredBackground), name: notification, object: nil
        )
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
    }

    @objc private func hostEnteredBackground() {
        // Losing focus to the OS authentication UI is not a background transition.
        cancel(transport: true)
    }

    private var foreground: Bool {
        #if os(iOS)
        UIApplication.shared.applicationState == .active
        #else
        NSApplication.shared.isActive && !NSApplication.shared.isHidden
        #endif
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        switch method {
        case 0:
            let policy = try payload.withUnsafeBytes { bytes -> UInt32 in
                guard bytes.count == 4 else { throw PluginError.invalidArgument }
                let value = bytes.loadUnaligned(as: UInt32.self).littleEndian
                guard value <= 1 else { throw PluginError.invalidArgument }
                return value
            }
            return capabilities(policy)
        case 1: return try await authenticate(AuthenticationRequest(payload: payload))
        case 2:
            guard payload.count == 0 else { throw PluginError.invalidArgument }
            cancel(transport: false)
            return Data()
        default: throw PluginError.unavailable
        }
    }

    public func close() {
        closed = true
        cancel(transport: true)
    }

    private func capabilities(_ policy: UInt32) -> Data {
        let context = LAContext()
        defer { context.invalidate() }
        var error: NSError?
        let available = context.canEvaluatePolicy(nativePolicy(policy), error: &error)
        let modality: UInt32 = switch context.biometryType {
        case .touchID: 1
        case .faceID: 2
        case .opticID: 8
        default: 0
        }
        let status = available ? 0 : readiness(error)
        let credentialContext = LAContext()
        defer { credentialContext.invalidate() }
        var credentialError: NSError?
        let credential = credentialContext.canEvaluatePolicy(.deviceOwnerAuthentication, error: &credentialError)
        let secure: UInt32 = credential ? 2 : (credentialError?.code == LAError.passcodeNotSet.rawValue ? 1 : 0)
        return authenticationFields([1, policy, status, modality, secure])
    }

    private func nativePolicy(_ policy: UInt32) -> LAPolicy {
        policy == 0 ? .deviceOwnerAuthenticationWithBiometrics : .deviceOwnerAuthentication
    }

    private func readiness(_ error: NSError?) -> UInt32 {
        guard error?.domain == LAError.errorDomain, let code = error?.code else { return 7 }
        switch code {
        case LAError.biometryNotAvailable.rawValue: return 1
        case LAError.biometryNotEnrolled.rawValue: return 2
        case LAError.biometryLockout.rawValue: return 3
        case LAError.passcodeNotSet.rawValue: return 4
        default: return 7
        }
    }

    private func authenticate(_ request: AuthenticationRequest) async throws -> Data {
        guard !Task.isCancelled else { throw PluginError.cancelled }
        guard active == nil else { throw PluginError.busy }
        guard foreground else { throw PluginError.unavailable }
        let context = LAContext()
        context.localizedCancelTitle = request.cancelLabel
        context.touchIDAuthenticationAllowableReuseDuration = 0
        if request.policy == 0 { context.localizedFallbackTitle = "" }
        var error: NSError?
        let available = context.canEvaluatePolicy(nativePolicy(request.policy), error: &error)
        #if os(iOS)
        if context.biometryType == .faceID {
            let purpose = Bundle.main.object(forInfoDictionaryKey: "NSFaceIDUsageDescription") as? String
            guard let purpose, !purpose.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                context.invalidate()
                throw PluginError.invalidArgument
            }
        }
        #endif
        if !available {
            context.invalidate()
            return try failure(error)
        }
        return try await evaluate(context, request: request) { completion in
            context.evaluatePolicy(
                self.nativePolicy(request.policy), localizedReason: request.reason, reply: completion
            )
        }
    }

    /// Keeps lifecycle/cancellation independent from the system prompt entry point.
    func evaluate(
        _ context: LAContext, request: AuthenticationRequest,
        start: (@escaping @Sendable (Bool, (any Error)?) -> Void) -> Void
    ) async throws -> Data {
        let token = UUID()
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { completion in
                guard !Task.isCancelled, !closed else {
                    context.invalidate()
                    completion.resume(throwing: PluginError.cancelled)
                    return
                }
                guard active == nil else {
                    context.invalidate()
                    completion.resume(throwing: PluginError.busy)
                    return
                }
                active = Session(token: token, context: context, policy: request.policy, completion: completion)
                start { [weak self] success, error in
                    Task { @MainActor [weak self] in self?.complete(token, success: success, error: error) }
                }
            }
        } onCancel: {
            Task { @MainActor [weak self] in
                guard self?.active?.token == token else { return }
                self?.cancel(transport: true)
            }
        }
    }

    private func complete(_ token: UUID, success: Bool, error: (any Error)?) {
        guard let session = active, session.token == token else { return }
        active = nil
        session.context.invalidate()
        do {
            let result = success ? authenticationFields([1, 0, session.policy == 0 ? 1 : 0]) : try failure(error)
            session.completion.resume(returning: result)
        } catch {
            session.completion.resume(throwing: error)
        }
    }

    private func cancel(transport: Bool) {
        guard let session = active else { return }
        active = nil
        session.context.invalidate()
        if transport {
            session.completion.resume(throwing: PluginError.cancelled)
        } else {
            session.completion.resume(returning: authenticationFields([1, 4, 0]))
        }
    }

    private func failure(_ error: (any Error)?) throws -> Data {
        guard let error = error as NSError?, error.domain == LAError.errorDomain else { throw PluginError.native }
        let outcome: UInt32 = switch error.code {
        case LAError.authenticationFailed.rawValue: 1
        case LAError.userCancel.rawValue, LAError.userFallback.rawValue: 2
        case LAError.systemCancel.rawValue: 3
        case LAError.appCancel.rawValue: 4
        case LAError.biometryLockout.rawValue: 5
        case LAError.biometryNotEnrolled.rawValue: 6
        case LAError.biometryNotAvailable.rawValue, LAError.notInteractive.rawValue: 7
        case LAError.passcodeNotSet.rawValue: 8
        default: throw PluginError.native
        }
        return authenticationFields([1, outcome, 0])
    }
}
