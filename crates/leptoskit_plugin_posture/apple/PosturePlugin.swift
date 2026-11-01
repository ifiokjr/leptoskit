import Foundation
import LeptosKitBridge
#if canImport(UIKit)
import UIKit
#endif

/// Fold state for the owning view's window. On iOS 27.1, `UIHingeInteraction`
/// reports the hinge's status and angle, and the view's `.division` reserved
/// region says where the fold crosses the window. Earlier systems cannot run on
/// a foldable iPhone and report a rigid device, as does an app built with an
/// older SDK, which has no hinge API to call. macOS has no hinge.
///
/// `canImport(UIKit, _version: 9127.0.85)` selects the iOS 27.1 SDK, whose UIKit
/// introduced the hinge API; a compiler version cannot tell 27.0 from 27.1.
@MainActor
public final class PosturePlugin: NativePlugin {
    public let name = "posture"
    public let methods: [UInt32] = [0, 1]
    private let context: NativePluginContext
    private var closed = false

    #if canImport(UIKit)
    private var observer: AnyObject?
    #endif

    public init(context: NativePluginContext) { self.context = context }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }

        guard method <= 1 else { throw PluginError.unavailable }
        let previous = try previousPosture(method: method, payload: payload)

        #if canImport(UIKit, _version: 9127.0.85)
        return try await watch(from: previous)
        #elseif canImport(UIKit)
        _ = try context.requireAttachedOwner()

        return PostureEncoding.rigid
        #else
        throw PluginError.unavailable
        #endif
    }

    /// The posture the caller already knows, which the watcher reports changes
    /// from. The status method carries none.
    private func previousPosture(method: UInt32, payload: NativeBuffer) throws -> Data? {
        guard method != 0 else {
            guard payload.count == 0 else { throw PluginError.invalidArgument }

            return nil
        }

        return try payload.withUnsafeBytes { bytes in
            let hinges = (bytes.count - PostureEncoding.headerBytes) / PostureEncoding.hingeBytes

            guard bytes.count >= PostureEncoding.headerBytes,
                  (bytes.count - PostureEncoding.headerBytes) % PostureEncoding.hingeBytes == 0,
                  hinges <= PostureEncoding.maxHinges else { throw PluginError.invalidArgument }

            return Data(bytes)
        }
    }

    #if canImport(UIKit, _version: 9127.0.85)
    /// Watches the window's hinges until they change.
    private func watch(from previous: Data?) async throws -> Data {
        let owner = try context.requireAttachedOwner()

        guard #available(iOS 27.1, *) else { return PostureEncoding.rigid }

        guard observer == nil else { throw PluginError.busy }
        let watcher = HingeWatcher(view: owner.view, previous: previous)
        observer = watcher
        defer { observer = nil }

        return try await watcher.next()
    }
    #endif

    public func close() {
        closed = true

        #if canImport(UIKit, _version: 9127.0.85)

        if #available(iOS 27.1, *), let watcher = observer as? HingeWatcher {
            watcher.finish(.failure(PluginError.cancelled))
        }

        #endif
    }
}

#if canImport(UIKit, _version: 9127.0.85)
/// Observes one view until its encoded posture differs from `previous`, or for
/// the first reading when there is no previous posture.
@available(iOS 27.1, *)
@MainActor
private final class HingeWatcher {
    private weak var view: UIView?
    private let previous: Data?
    private var interaction: UIHingeInteraction?
    private var geometry: NSKeyValueObservation?
    private var continuation: CheckedContinuation<Data, any Error>?
    private var hinge: UIHinge?
    private var sawHinge = false

    init(view: UIView, previous: Data?) {
        self.view = view
        self.previous = previous
    }

    func next() async throws -> Data {
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in

                guard !Task.isCancelled, let view else {
                    continuation.resume(throwing: PluginError.cancelled)

                    return
                }

                self.continuation = continuation
                // The handler runs with the initial state as soon as the
                // interaction joins the hierarchy, then on every hinge change.
                let interaction = UIHingeInteraction { [weak self] _, update in
                    self?.hinge = update.hinge
                    self?.sawHinge = true
                    self?.evaluate()
                }
                self.interaction = interaction
                view.addInteraction(interaction)
                // Opening, closing, and rotating resize the window; the fold's
                // region moves with it even when the hinge status does not.
                geometry = view.window?.windowScene?.observe(\.effectiveGeometry) { [weak self] _, _ in
                    MainActor.assumeIsolated { self?.evaluate() }
                }
            }
        } onCancel: {
            Task { @MainActor [weak self] in self?.finish(.failure(PluginError.cancelled)) }
        }
    }

    private func evaluate() {
        guard sawHinge, let view else { return }
        let current = PostureEncoding.encode(view: view, hinge: hinge)

        if current != previous { finish(.success(current)) }
    }

    func finish(_ result: Result<Data, any Error>) {
        guard let continuation else { return }
        self.continuation = nil
        geometry?.invalidate()
        geometry = nil

        if let interaction { view?.removeInteraction(interaction) }
        interaction = nil
        continuation.resume(with: result)
    }
}
#endif

/// The posture wire format shared with Rust and Kotlin: little-endian u32
/// words, a three-word header, and nine words per hinge.
enum PostureEncoding {
    static let maxHinges = 4
    static let headerBytes = 12
    static let hingeBytes = 36

    private static let unknownFlag: UInt32 = 0
    private static let falseFlag: UInt32 = 1
    private static let trueFlag: UInt32 = 2

    /// A device that does not fold.
    static let rigid = data([falseFlag, 0, 0])

    static func data(_ words: [UInt32]) -> Data {
        var result = Data(capacity: words.count * 4)

        for word in words {
            var little = word.littleEndian
            withUnsafeBytes(of: &little) { result.append(contentsOf: $0) }
        }

        return result
    }

    #if canImport(UIKit, _version: 9127.0.85)
    @available(iOS 27.1, *)
    @MainActor
    static func encode(view: UIView, hinge: UIHinge?) -> Data {
        // A nil hinge means this hierarchy gets no hinge updates.
        guard let hinge else { return data([unknownFlag, 0, 0]) }
        let fold: UInt32
        switch hinge.status {
        case .closed: fold = 1
        case .partiallyOpen: fold = 2
        case .fullyOpen: fold = 3
        default: fold = 0
        }
        // A closed hinge does not cross the window: the app is on the cover screen.
        guard fold != 1 else { return data([trueFlag, fold, 0]) }
        let degrees = UInt32(max(0, min(360, (hinge.angle * 180 / .pi).rounded())))
        let bounds = view.bounds
        let division = view.reservedRegions(kind: .division, options: .includeInactive).first
        var words: [UInt32] = [trueFlag, fold, 1, fold]

        if let division, bounds.width > 0, bounds.height > 0 {
            let frame = division.frame.intersection(bounds)
            let occluded = view.reservedRegions(kind: .occlusion).contains { $0.frame.intersects(frame) }
            words += [
                frame.height >= frame.width ? 1 : 2,
                occluded ? trueFlag : falseFlag,
                1,
                Float((frame.minX - bounds.minX) / bounds.width).bitPattern,
                Float((frame.minY - bounds.minY) / bounds.height).bitPattern,
                Float(frame.width / bounds.width).bitPattern,
                Float(frame.height / bounds.height).bitPattern
            ]
        } else {
            words += [0, unknownFlag, 0, 0, 0, 0, 0]
        }

        words.append(degrees)

        return data(words)
    }
    #endif
}
