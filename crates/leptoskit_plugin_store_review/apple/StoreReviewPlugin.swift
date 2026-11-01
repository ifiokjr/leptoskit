import Foundation
import LeptosKitBridge
import StoreKit
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

@MainActor
protocol ReviewService {
    func request(from owner: NativePresentationOwner) throws
}

@MainActor
private struct SystemReviewService: ReviewService {
    func request(from owner: NativePresentationOwner) throws {
        #if canImport(UIKit)
        guard let scene = owner.viewIfLoaded?.window?.windowScene,
              scene.activationState == .foregroundActive else { throw PluginError.unavailable }
        AppStore.requestReview(in: scene)
        #else
        AppStore.requestReview(in: owner)
        #endif
    }
}

/// Uses only the context supplied by its host. StoreKit provides no presentation
/// or review completion callback, so success acknowledges the API call only.
@MainActor
public final class StoreReviewPlugin: NativePlugin {
    public let name = "store-review"
    public let methods: [UInt32] = [0, 1]
    private let context: NativePluginContext
    private let service: any ReviewService
    private var closed = false

    /// Readiness is a wire status: the app can present, it cannot, or it is busy.
    private static let ready = 1
    private static let inactive = 2

    public init(context: NativePluginContext) {
        self.context = context
        service = SystemReviewService()
    }

    init(context: NativePluginContext, service: any ReviewService) {
        self.context = context
        self.service = service
    }

    /// Answers readiness (`0`) or starts one review flow (`1`). Every call is
    /// refused once the host closes the plugin.
    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        try checkRequest(method: method, payload: payload)

        if method == 0 { return availability() }

        return try request()
    }

    /// Every call is refused once closed, and every method takes no argument.
    private func checkRequest(method: UInt32, payload: NativeBuffer) throws {
        guard !closed else { throw PluginError.cancelled }

        guard methods.contains(method) else { throw PluginError.unavailable }

        guard payload.count == 0 else { throw PluginError.invalidArgument }

        try Task.checkCancellation()
    }

    /// Reports whether this app has an owner that can present store UI.
    private func availability() -> Data {
        do {
            _ = try context.requirePresentationOwner()

            return Data([Self.ready, 0, 0, 0])
        } catch PluginError.unavailable {
            return Data([Self.inactive, 0, 0, 0])
        }
    }

    /// Starts the store flow; the reply acknowledges the call, not a review.
    private func request() throws -> Data {
        let owner = try context.requirePresentationOwner()

        try service.request(from: owner)

        return Data([Self.ready, 0, 0, 0])
    }

    /// Store-owned UI cannot be dismissed by the plugin after request returns.
    public func close() { closed = true }
}
