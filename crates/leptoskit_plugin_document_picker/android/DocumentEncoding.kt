package dev.leptoskit.plugins.documentpicker

import dev.leptoskit.bridge.PluginException
import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder

internal object DocumentEncoding {
    /** The largest document a caller may ask for. */
    const val MAX_BYTES = 4 * 1024 * 1024

    /** The largest display name or media type the reply can carry. */
    private const val MAX_METADATA_BYTES = 4096

    /** One read never allocates more than this, however large the limit. */
    private const val CHUNK_BYTES = 65536

    /** Three u32 lengths prefix the reply body. */
    private const val HEADER_BYTES = 12

    fun validateMetadata(
        name: ByteArray,
        mime: ByteArray,
    ) {
        if (name.isEmpty() || name.size > MAX_METADATA_BYTES || mime.size > MAX_METADATA_BYTES) {
            throw PluginException(PluginException.TOO_LARGE)
        }
    }

    suspend fun readBounded(
        stream: InputStream,
        limit: Int,
        checkCancellation: suspend () -> Unit,
    ): ByteArray {
        if (limit !in 0..MAX_BYTES) throw PluginException(PluginException.TOO_LARGE)

        val output = ByteArrayOutputStream(minOf(limit, CHUNK_BYTES))
        val buffer = ByteArray(minOf(limit + 1, CHUNK_BYTES))
        var more = true

        while (more) {
            more = copyChunk(stream, buffer, output, limit, checkCancellation)
        }

        return output.toByteArray()
    }

    /**
     * Appends one chunk and reports whether the stream has more. The caller
     * checks cancellation between chunks, so a dropped guest call stops the
     * copy at a chunk boundary instead of running to completion.
     */
    private suspend fun copyChunk(
        stream: InputStream,
        buffer: ByteArray,
        output: ByteArrayOutputStream,
        limit: Int,
        checkCancellation: suspend () -> Unit,
    ): Boolean {
        checkCancellation()
        val count = stream.read(buffer, 0, minOf(buffer.size, limit - output.size() + 1))

        if (count < 0) return false

        if (count == 0) throw PluginException(PluginException.NATIVE)

        if (count > limit - output.size()) throw PluginException(PluginException.TOO_LARGE)

        output.write(buffer, 0, count)

        return true
    }

    fun encode(
        name: ByteArray,
        mime: ByteArray,
        content: ByteArray,
    ): ByteArray {
        validateMetadata(name, mime)

        if (content.size > MAX_BYTES) throw PluginException(PluginException.TOO_LARGE)
        val reply =
            ByteBuffer
                .allocate(HEADER_BYTES + name.size + mime.size + content.size)
                .order(ByteOrder.LITTLE_ENDIAN)

        for (field in arrayOf(name, mime, content)) reply.putInt(field.size).put(field)

        return reply.array()
    }
}
