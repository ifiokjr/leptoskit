import AuthenticationServices
import Foundation
import LeptosKitBridge

/// Sign-in in the system browser with `ASWebAuthenticationSession`, never
/// a web view in the app. The session returns to this app's private-use
/// scheme, which the system hands only to the session that started it.
@MainActor
public final class WebAuthPlugin: NSObject, NativePlugin {
    public let name = "web-auth"
    public let methods: [UInt32] = [0, 1]
    private let context: NativePluginContext
    private let bundleIdentifier: String?
    private var begun: (id: UInt32, returnTo: String)?
    private var next: UInt32 = 0
    private var session: ASWebAuthenticationSession?
    private var invocation: UUID?
    private var closed = false

    public init(context: NativePluginContext) {
        self.context = context
        bundleIdentifier = Bundle.main.bundleIdentifier
    }

    init(context: NativePluginContext, bundleIdentifier: String?) {
        self.context = context
        self.bundleIdentifier = bundleIdentifier
    }

    public func close() {
        closed = true
        session?.cancel()
        session = nil
        begun = nil
    }

    /// Answers one call: starts a session, or opens the one just started.
    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        switch method {
        case 0:
            guard payload.count == 0 else { throw PluginError.invalidArgument }

            return try beginSession()
        case 1:
            return try await continueSession(payload)
        default:
            throw PluginError.unavailable
        }
    }

    /// Starts a session, and answers with the address the system returns to.
    private func beginSession() throws -> Data {
        guard session == nil else { throw PluginError.busy }

        guard let identifier = bundleIdentifier,
              let returnTo = webAuthReturnAddress(bundleIdentifier: identifier) else {
            throw PluginError.unavailable
        }
        next &+= 1
        begun = (next, returnTo)

        return encodeWebAuthSession(id: next, returnTo: returnTo)
    }

    /// Opens the session's URL, and waits for the system to hand the app back.
    private func continueSession(_ payload: NativeBuffer) async throws -> Data {
        let request = try WebAuthRequest(payload: payload)

        guard session == nil else { throw PluginError.busy }

        guard let begun, begun.id == request.session else { throw PluginError.invalidCall }
        let returnTo = begun.returnTo
        self.begun = nil
        let owner = try context.requirePresentationOwner()
        try Task.checkCancellation()
        let invocation = UUID()
        self.invocation = invocation

        return try await withTaskCancellationHandler {
            try await authenticate(request.url, returnTo: returnTo, owner: owner)
        } onCancel: {
            Task { @MainActor [weak self] in

                guard let self, self.invocation == invocation else { return }
                self.session?.cancel()
            }
        }
    }

    /// Presents the sheet for `url` and answers the address it came back to.
    private func authenticate(_ url: URL, returnTo: String, owner: NativePresentationOwner) async throws -> Data {
        guard let scheme = URLComponents(string: returnTo)?.scheme else { throw PluginError.invalidArgument }
        let anchor = Anchor(owner.view.window)
        defer {
            session = nil
            invocation = nil
        }
        let back = try await open(url, scheme: scheme, anchor: anchor)
        withExtendedLifetime(anchor) {}

        guard webAuthCameBack(back, to: returnTo) else { throw PluginError.native }

        return Data(back.absoluteString.utf8)
    }

    /// Starts the sheet, and waits for its callback.
    private func open(_ url: URL, scheme: String, anchor: Anchor) async throws -> URL {
        try await withCheckedThrowingContinuation { continuation in
            let session = makeSession(url, scheme: scheme, anchor: anchor, resuming: continuation)

            self.session = session

            guard session.start() else {
                self.session = nil
                continuation.resume(throwing: PluginError.unavailable)

                return
            }
        }
    }

    /// Builds the session the running OS supports, anchored to the window.
    private func makeSession(
        _ url: URL,
        scheme: String,
        anchor: Anchor,
        resuming continuation: CheckedContinuation<URL, any Error>
    ) -> ASWebAuthenticationSession {
        let completion = Self.completion(resuming: continuation)
        let session: ASWebAuthenticationSession

        if #available(iOS 17.4, macOS 14.4, *) {
            session = ASWebAuthenticationSession(
                url: url, callback: .customScheme(scheme), completionHandler: completion)
        } else {
            session = ASWebAuthenticationSession(url: url, callbackURLScheme: scheme, completionHandler: completion)
        }

        session.presentationContextProvider = anchor

        return session
    }

    /// Resumes the waiting call with what the sheet's callback carried.
    private static func completion(
        resuming continuation: CheckedContinuation<URL, any Error>
    ) -> ASWebAuthenticationSession.CompletionHandler {
        { url, error in

            if let url {
                continuation.resume(returning: url)
            } else if let error = error as? ASWebAuthenticationSessionError, error.code == .canceledLogin {
                continuation.resume(throwing: PluginError.cancelled)
            } else {
                continuation.resume(throwing: PluginError.native)
            }
        }
    }
}

/// The window the session's sheet attaches to.
private final class Anchor: NSObject, ASWebAuthenticationPresentationContextProviding, @unchecked Sendable {
    private let window: ASPresentationAnchor?

    init(_ window: ASPresentationAnchor?) {
        self.window = window
    }

    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        window ?? ASPresentationAnchor()
    }
}
