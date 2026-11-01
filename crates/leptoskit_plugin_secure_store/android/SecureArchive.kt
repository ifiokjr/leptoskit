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
            if (data.size !in MIN_BYTES..MAX_BYTES) throw PluginException(PluginException.NATIVE)
            val input = ByteBuffer.wrap(data).order(ByteOrder.LITTLE_ENDIAN)
            val count = input.int

            if (count !in 0..MAX_ENTRIES) throw PluginException(PluginException.NATIVE)
            val archive = SecureArchive()

            repeat(count) { archive.add(entry(data, input)) }

            if (input.hasRemaining()) throw PluginException(PluginException.NATIVE)

            return archive
        }

        /** Reads one entry at the cursor, leaving it past the entry. */
        private fun entry(
            data: ByteArray,
            input: ByteBuffer,
        ): SecureStorePlugin.Request {
            val start = input.position()
            repeat(FIELDS_PER_ENTRY) { skipField(input) }
            val entry = ByteBuffer.wrap(data, start, input.position() - start)

            return SecureStorePlugin.Request.decode(SECRET_METHOD, entry)
        }

        /** Moves the cursor past one length-prefixed field. */
        private fun skipField(input: ByteBuffer) {
            if (input.remaining() < 4) throw PluginException(PluginException.NATIVE)
            val length = input.int

            if (length < 0 || length > input.remaining()) throw PluginException(PluginException.NATIVE)
            input.position(input.position() + length)
        }

        /** Adds one entry, refusing a name pair the archive already holds. */
        private fun SecureArchive.add(request: SecureStorePlugin.Request) {
            val value = request.secret ?: throw PluginException(PluginException.NATIVE)

            if (get(request.service, request.key) != null) throw PluginException(PluginException.NATIVE)

            set(request.service, request.key, value)
        }

        /** Every entry carries a service, a key, and a value. */
        private const val FIELDS_PER_ENTRY = 3

        /** The method id whose request shape carries a secret. */
        private const val SECRET_METHOD = 1

        /** The largest archive this store will hold, and the smallest. */
        private const val MAX_BYTES = 2560
        private const val MIN_BYTES = 4

        /** How many entries one archive may hold. */
        private const val MAX_ENTRIES = 64
    }
}
