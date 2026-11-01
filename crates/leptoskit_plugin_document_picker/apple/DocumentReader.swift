import Foundation
import Darwin
import LeptosKitBridge
import UniformTypeIdentifiers

/// The caller retains its job until this bounded read finishes, including cancellation.
enum DocumentReader {
    static let maximum = 4 * 1024 * 1024

    /// The largest display name or media type a reply can carry.
    private static let metadataBytes = 4096

    /// One read never allocates more than this, however large the limit.
    private static let chunkBytes = 65536

    /// Three u32 lengths prefix the reply body.
    private static let headerBytes = 12

    static func read(
        _ url: URL,
        limit: Int,
        checkCancellation: () throws -> Void = Task.checkCancellation
    ) throws -> Data {
        guard (0...maximum).contains(limit) else { throw PluginError.tooLarge }
        try checkCancellation()
        let scoped = url.startAccessingSecurityScopedResource()

        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        let coordinator = NSFileCoordinator()
        var coordinationError: NSError?
        var result: Result<Data, Error> = .failure(PluginError.native)
        coordinator.coordinate(readingItemAt: url, options: [], error: &coordinationError) { coordinated in
            result = Result {
                try readCoordinated(coordinated, limit: limit, checkCancellation: checkCancellation)
            }
        }

        if let coordinationError { throw coordinationError }

        return try result.get()
    }

    /// Reads inside a file-coordination block, so another process cannot change
    /// the file between the resource check and the copy.
    static func readCoordinated(
        _ url: URL,
        limit: Int,
        checkCancellation: () throws -> Void = Task.checkCancellation
    ) throws -> Data {
        try checkCancellation()
        var url = url
        url.removeAllCachedResourceValues()
        let values = try url.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey, .contentTypeKey])

        guard values.isRegularFile == true else { throw PluginError.invalidArgument }

        if let size = values.fileSize, size > limit { throw PluginError.tooLarge }
        let name = Data(url.lastPathComponent.utf8)
        let mime = Data((values.contentType?.preferredMIMEType ?? "").utf8)

        guard !name.isEmpty, name.count <= metadataBytes, mime.count <= metadataBytes else {
            throw PluginError.tooLarge
        }

        let handle = try openRegularFile(url, limit: limit)
        defer { try? handle.close() }
        let content = try readContent(handle, limit: limit, checkCancellation: checkCancellation)
        var reply = Data(capacity: name.count + mime.count + content.count + headerBytes)

        for field in [name, mime, content] {
            var count = UInt32(field.count).littleEndian
            withUnsafeBytes(of: &count) { reply.append(contentsOf: $0) }
            reply.append(field)
        }

        return reply
    }

    /// Opens the file without following a link and validates the descriptor
    /// itself, so a path swapped after the resource check cannot be read.
    private static func openRegularFile(_ url: URL, limit: Int) throws -> FileHandle {
        let descriptor = url.withUnsafeFileSystemRepresentation { path in

            guard let path else { return Int32(-1) }
            return open(path, O_RDONLY | O_CLOEXEC | O_NOFOLLOW | O_NONBLOCK)
        }

        guard descriptor >= 0 else {
            throw errno == EACCES || errno == EPERM ? PluginError.permissionDenied : PluginError.native
        }

        let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
        var attributes = stat()
        // FileHandle owns the descriptor, including when validation throws.
        guard fstat(descriptor, &attributes) == 0 else { throw PluginError.native }

        guard attributes.st_mode & S_IFMT == S_IFREG else { throw PluginError.invalidArgument }

        guard attributes.st_size <= limit else { throw PluginError.tooLarge }

        return handle
    }

    /// Copies the descriptor in bounded chunks, checking cancellation between
    /// them so a cancelled job stops without waiting for the rest of the file.
    private static func readContent(
        _ handle: FileHandle,
        limit: Int,
        checkCancellation: () throws -> Void
    ) throws -> Data {
        var content = Data()

        while true {
            try checkCancellation()
            let chunk = try handle.read(upToCount: min(chunkBytes, limit - content.count + 1)) ?? Data()

            if chunk.isEmpty { break }

            guard chunk.count <= limit - content.count else { throw PluginError.tooLarge }
            content.append(chunk)
        }

        return content
    }
}
