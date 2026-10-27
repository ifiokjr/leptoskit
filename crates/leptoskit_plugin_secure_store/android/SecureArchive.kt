package dev.leptoskit.plugins.securestore

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** A single encrypted record bounds both dynamic identities and their payloads. */
internal class SecureArchive {
    private class Entry(
        val service: String,
        val key: String,
        val value: ByteArray,
    )

    private val entries = mutableListOf<Entry>()

    fun get(
        service: String,
        key: String,
    ): ByteArray? = entries.firstOrNull { it.service == service && it.key == key }?.value

    fun remove(
        service: String,
        key: String,
    ) {
        entries.removeAll {
            val matches = it.service == service && it.key == key
            if (matches) it.value.fill(0)
            matches
        }
    }

    fun set(
        service: String,
        key: String,
        value: ByteArray,
    ) {
        remove(service, key)
        entries.add(Entry(service, key, value))
    }

    fun clear() {
        entries.forEach { it.value.fill(0) }
    }

    fun encode(
        maxBytes: Int,
        maxEntries: Int,
    ): ByteArray {
        val size = 4 + entries.sumOf { 12 + it.service.length + it.key.length + it.value.size }
        if (entries.size > maxEntries || size > maxBytes) throw PluginException(PluginException.TOO_LARGE)
        val output = ByteBuffer.allocate(size).order(ByteOrder.LITTLE_ENDIAN)

        fun field(value: ByteArray) {
            output.putInt(value.size).put(value)
        }
        output.putInt(entries.size)
        entries.forEach {
            field(it.service.toByteArray(Charsets.US_ASCII))
            field(it.key.toByteArray(Charsets.US_ASCII))
            field(it.value)
        }
        return output.array()
    }

    companion object {
        fun decode(data: ByteArray): SecureArchive {
            if (data.size > 2560 || data.size < 4) throw PluginException(PluginException.NATIVE)
            val input = ByteBuffer.wrap(data).order(ByteOrder.LITTLE_ENDIAN)
            val count = input.int
            if (count !in 0..64) throw PluginException(PluginException.NATIVE)
            val archive = SecureArchive()
            repeat(count) {
                val start = input.position()
                repeat(3) {
                    if (input.remaining() < 4) throw PluginException(PluginException.NATIVE)
                    val length = input.int
                    if (length < 0 || length > input.remaining()) throw PluginException(PluginException.NATIVE)
                    input.position(input.position() + length)
                }
                val entry = ByteBuffer.wrap(data, start, input.position() - start)
                val request = SecureStorePlugin.Request.decode(1, entry)
                val value = request.secret ?: throw PluginException(PluginException.NATIVE)
                if (archive.get(request.service, request.key) != null) throw PluginException(PluginException.NATIVE)
                archive.set(request.service, request.key, value)
            }
            if (input.hasRemaining()) throw PluginException(PluginException.NATIVE)
            return archive
        }
    }
}
