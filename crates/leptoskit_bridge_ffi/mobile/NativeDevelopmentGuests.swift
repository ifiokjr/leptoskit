import CLeptosKitBridge
import Foundation
import LeptosKitBridge

/// Follows the guest `leptoskit dev` serves to the app it launched, on a dedicated
/// thread, and delivers each new guest on the main actor. Only debug builds accept
/// development guests: Rust refuses to open one in a release build.
final class NativeDevelopmentGuests: @unchecked Sendable {
    private static let retryDelay: TimeInterval = 1
    private let lock = NSLock()
    private var closed = false

    /// The guest URL `leptoskit dev` launched this process with, if any.
    static var url: String? { ProcessInfo.processInfo.environment["LEPTOSKIT_DEV_GUEST"] }

    init(url: String, running: Data, onGuest: @escaping @MainActor @Sendable (Data) -> Void) {
        let thread = Thread { [self] in follow(url: url, running: running, onGuest: onGuest) }
        thread.name = "leptoskit-dev-guests"
        thread.start()
    }

    /// Stops delivery; the thread exits after its current request returns.
    func close() {
        lock.lock()
        closed = true
        lock.unlock()
    }

    private var isClosed: Bool {
        lock.lock()
        defer { lock.unlock() }

        return closed
    }

    private func follow(url: String, running: Data, onGuest: @escaping @MainActor @Sendable (Data) -> Void) {
        var error = lk_buffer()
        let address = Array(url.utf8)
        let handle = address.withUnsafeBufferPointer { address in
            running.withUnsafeBytes { guest in
                lk_dev_open(
                    address.baseAddress, address.count,
                    guest.bindMemory(to: UInt8.self).baseAddress, guest.count, &error
                )
            }
        }

        guard let handle else {
            Self.report(NativeBuffer(taking: error))

            return
        }

        defer { lk_dev_destroy(handle) }

        while !isClosed {
            var wasm = lk_buffer()

            switch lk_dev_next(handle, &wasm) {
            case 1:
                let guest = NativeBuffer(taking: wasm).withUnsafeBytes { Data($0) }
                Task { @MainActor [weak self] in
                    if self?.isClosed == false { onGuest(guest) }
                }
            case 0:
                continue
            default:
                Self.report(NativeBuffer(taking: lk_dev_error(handle)))
                Thread.sleep(forTimeInterval: Self.retryDelay)
            }
        }
    }

    private static func report(_ diagnostic: NativeBuffer) {
        NSLog("leptoskit dev: %@", (try? diagnostic.utf8()) ?? "unreadable diagnostic")
    }
}
