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

    public init(context: NativePluginContext) {
        self.context = context
        service = SystemReviewService()
    }

    init(context: NativePluginContext, service: any ReviewService) {
        self.context = context
        self.service = service
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard methods.contains(method) else { throw PluginError.unavailable }

        guard payload.count == 0 else { throw PluginError.invalidArgument }
        try Task.checkCancellation()

        if method == 0 {
            do {
                _ = try context.requirePresentationOwner()

                return Data([1, 0, 0, 0])
            } catch PluginError.unavailable {
                return Data([2, 0, 0, 0])
            }
        }

        let owner = try context.requirePresentationOwner()
        try service.request(from: owner)

        return Data([1, 0, 0, 0])
    }

    /// Store-owned UI cannot be dismissed by the plugin after request returns.
    public func close() { closed = true }
}
