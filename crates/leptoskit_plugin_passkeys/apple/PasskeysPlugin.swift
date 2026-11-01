import AuthenticationServices
import Foundation
import LeptosKitBridge

/// Passkeys and Sign in with Apple through `ASAuthorizationController`. The
/// relying party must list this app under `webcredentials` in its
/// `apple-app-site-association`, and the app needs the matching associated
/// domain. User verification is always required.
@MainActor
public final class PasskeysPlugin: NSObject, NativePlugin {
    public let name = "passkeys"
    public let methods: [UInt32] = [0, 1, 2]
    private let context: NativePluginContext
    private var controller: ASAuthorizationController?
    private var waiting: CheckedContinuation<ASAuthorization, Error>?
    private var invocation: UUID?
    private var closed = false

    public init(context: NativePluginContext) {
        self.context = context
    }

    public func close() {
        closed = true
        cancel()
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        let request: ASAuthorizationRequest
        switch method {
        case 0:
            let options = try CreationOptions(payload: payload)
            let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(
                relyingPartyIdentifier: options.relyingParty)
            let registration = provider.createCredentialRegistrationRequest(
                challenge: options.challenge, name: options.userName, userID: options.userID)
            registration.userVerificationPreference = .required

            if #available(iOS 17.4, macOS 14.4, *) {
                registration.excludedCredentials = options.excluded.map {
                    ASAuthorizationPlatformPublicKeyCredentialDescriptor(credentialID: $0)
                }
            }

            request = registration
        case 1:
            let options = try RequestOptions(payload: payload)
            let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(
                relyingPartyIdentifier: options.relyingParty)
            let assertion = provider.createCredentialAssertionRequest(challenge: options.challenge)
            assertion.userVerificationPreference = .required
            assertion.allowedCredentials = options.allowed.map {
                ASAuthorizationPlatformPublicKeyCredentialDescriptor(credentialID: $0)
            }
            request = assertion
        case 2:
            let apple = ASAuthorizationAppleIDProvider().createRequest()
            apple.requestedScopes = [.fullName, .email]
            apple.nonce = try appleNonce(payload)
            request = apple
        default:
            throw PluginError.unavailable
        }

        guard controller == nil else { throw PluginError.busy }
        _ = try context.requirePresentationOwner()
        try Task.checkCancellation()
        let invocation = UUID()
        self.invocation = invocation

        let authorization = try await withTaskCancellationHandler {
            try await perform(request)
        } onCancel: {
            Task { @MainActor [weak self] in

                guard let self, self.invocation == invocation else { return }
                self.cancel()
            }
        }

        return try Self.answer(authorization.credential)
    }

    private func perform(_ request: ASAuthorizationRequest) async throws -> ASAuthorization {
        try await withCheckedThrowingContinuation { continuation in
            let controller = ASAuthorizationController(authorizationRequests: [request])
            controller.delegate = self
            controller.presentationContextProvider = self
            self.controller = controller
            waiting = continuation
            controller.performRequests()
        }
    }

    private func cancel() {
        if #available(iOS 16.0, macOS 13.0, *) {
            controller?.cancel()
        }

        finish(.failure(PluginError.cancelled))
    }

    private func finish(_ result: Result<ASAuthorization, Error>) {
        let waiting = waiting
        self.waiting = nil
        controller?.delegate = nil
        controller = nil
        invocation = nil
        waiting?.resume(with: result)
    }

    private static func answer(_ credential: ASAuthorizationCredential) throws -> Data {
        if let made = credential as? ASAuthorizationPlatformPublicKeyCredentialRegistration {
            guard let attestation = made.rawAttestationObject else { throw PluginError.native }
            return try registrationJSON(
                credentialID: made.credentialID, clientData: made.rawClientDataJSON,
                attestationObject: attestation)
        }

        if let signed = credential as? ASAuthorizationPlatformPublicKeyCredentialAssertion {
            return try assertionJSON(
                credentialID: signed.credentialID, clientData: signed.rawClientDataJSON,
                authenticatorData: signed.rawAuthenticatorData, signature: signed.signature,
                userID: signed.userID)
        }

        if let apple = credential as? ASAuthorizationAppleIDCredential, let token = apple.identityToken {
            return try appleIDJSON(
                identityToken: token, givenName: apple.fullName?.givenName,
                familyName: apple.fullName?.familyName)
        }

        throw PluginError.native
    }

    /// How an authorization failure reads to the guest.
    nonisolated static func failure(_ error: Error) -> PluginError {
        guard let error = error as? ASAuthorizationError else { return .native }
        switch error.code {
        case .canceled: return .cancelled
        case .notHandled, .notInteractive: return .unavailable
        default: return .native
        }
    }
}

// AuthenticationServices calls these on the main actor.
extension PasskeysPlugin: ASAuthorizationControllerDelegate {
    public func authorizationController(
        controller: ASAuthorizationController, didCompleteWithAuthorization authorization: ASAuthorization
    ) {
        guard controller === self.controller else { return }
        finish(.success(authorization))
    }

    public func authorizationController(controller: ASAuthorizationController, didCompleteWithError error: Error) {
        guard controller === self.controller else { return }
        finish(.failure(Self.failure(error)))
    }
}

extension PasskeysPlugin: ASAuthorizationControllerPresentationContextProviding {
    public func presentationAnchor(for controller: ASAuthorizationController) -> ASPresentationAnchor {
        (try? context.requirePresentationOwner().view.window) ?? ASPresentationAnchor()
    }
}
