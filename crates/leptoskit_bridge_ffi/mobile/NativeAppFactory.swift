import CLeptosKitBridge
import Foundation
import LeptosKitBridge

/// Creates the native owner after resolving platform-specific application directories.
@MainActor
enum NativeAppFactory {
    static func create(wasm: Data, identifier: String?) throws -> NativeAppLifecycle {
        let dataDirectory = try directory(.applicationSupportDirectory, identifier: identifier)
        let cacheDirectory = try directory(.cachesDirectory, identifier: identifier)
        let dataPath = Array(dataDirectory.path.utf8)
        let cachePath = Array(cacheDirectory.path.utf8)
        var error = lk_buffer()
        let raw = dataPath.withUnsafeBufferPointer { data in
            cachePath.withUnsafeBufferPointer { cache in
                var context = lk_app_context(
                    data_dir: data.baseAddress, data_dir_len: data.count,
                    cache_dir: cache.baseAddress, cache_dir_len: cache.count
                )

                return wasm.withUnsafeBytes {
                    leptoskit_app_create_with_context(
                        $0.bindMemory(to: UInt8.self).baseAddress, $0.count, &context, &error
                    )
                }
            }
        }

        guard let raw else { throw NativeAppError(message: try NativeBuffer(taking: error).utf8()) }

        return try NativeAppLifecycle(taking: raw)
    }
    /// macOS namespaces storage by bundle; iOS uses its existing sandbox directories.
    private static func directory(_ kind: FileManager.SearchPathDirectory, identifier: String?) throws -> URL {
        let manager = FileManager.default
        let base = try manager.url(
            for: kind, in: .userDomainMask, appropriateFor: nil, create: true
        )

        guard let identifier else { return base }
        let scoped = base.appendingPathComponent(identifier, isDirectory: true)
        try manager.createDirectory(at: scoped, withIntermediateDirectories: true)

        return scoped
    }
}
