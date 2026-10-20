package dev.leptoskit.plugins.documentpicker

import dev.leptoskit.bridge.PluginException
import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder

internal object DocumentEncoding {
    const val MAX_BYTES = 4 * 1024 * 1024

    fun validateMetadata(
        name: ByteArray,
        mime: ByteArray,
    ) {
        if (name.isEmpty() || name.size > 4096 || mime.size > 4096) throw PluginException(PluginException.TOO_LARGE)
    }

    suspend fun readBounded(
        stream: InputStream,
        limit: Int,
        checkCancellation: suspend () -> Unit,
    ): ByteArray {
        if (limit !in 0..MAX_BYTES) throw PluginException(PluginException.TOO_LARGE)
        val output = ByteArrayOutputStream(minOf(limit, 65536))
        val buffer = ByteArray(minOf(limit + 1, 65536))
        while (true) {
            checkCancellation()
            val count = stream.read(buffer, 0, minOf(buffer.size, limit - output.size() + 1))
            if (count < 0) break
            if (count == 0) throw PluginException(PluginException.NATIVE)
            if (count > limit - output.size()) throw PluginException(PluginException.TOO_LARGE)
            output.write(buffer, 0, count)
        }
        return output.toByteArray()
    }

    fun encode(
        name: ByteArray,
        mime: ByteArray,
        content: ByteArray,
    ): ByteArray {
        validateMetadata(name, mime)
        if (content.size > MAX_BYTES) throw PluginException(PluginException.TOO_LARGE)
        val reply = ByteBuffer.allocate(12 + name.size + mime.size + content.size).order(ByteOrder.LITTLE_ENDIAN)
        for (field in arrayOf(name, mime, content)) reply.putInt(field.size).put(field)
        return reply.array()
    }
}
