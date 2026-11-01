package dev.leptoskit.plugins.sharing

import dev.leptoskit.bridge.PluginException
import java.net.URI
import java.net.URISyntaxException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction

internal data class ShareContent(
    val kind: Int,
    val value: String,
) {
    companion object {
        /** Decodes a framed text or URL payload, refusing unknown kinds. */
        fun decode(payload: ByteBuffer): ShareContent {
            val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
            val (kind, size) = header(bytes)

            if (size != bytes.remaining()) throw invalid()

            if (size > limit(kind)) throw PluginException(PluginException.TOO_LARGE)

            return ShareContent(kind, value(kind, bytes))
        }

        /** Reads the kind and value length, refusing an unknown kind or truncated framing. */
        private fun header(bytes: ByteBuffer): Pair<Int, Int> {
            if (bytes.remaining() < HEADER_BYTES) throw invalid()

            val kind = bytes.int
            val size = bytes.int

            if (kind != TEXT && kind != URL) throw invalid()

            return kind to size
        }

        /** Decodes the value, then applies the rule its kind carries. */
        private fun value(
            kind: Int,
            bytes: ByteBuffer,
        ): String {
            val value = text(bytes)

            if (value.isEmpty() || '\u0000' in value) throw invalid()

            if (kind == URL) validateUrl(value)

            return value
        }

        /** The largest value each kind may carry. */
        private fun limit(kind: Int): Int = if (kind == TEXT) MAX_TEXT_BYTES else MAX_URL_BYTES

        /** Decodes strict UTF-8, refusing a payload that is not text. */
        private fun text(bytes: ByteBuffer): String =
            try {
                Charsets.UTF_8
                    .newDecoder()
                    .onMalformedInput(CodingErrorAction.REPORT)
                    .onUnmappableCharacter(CodingErrorAction.REPORT)
                    .decode(bytes)
                    .toString()
            } catch (error: CharacterCodingException) {
                throw PluginException(PluginException.INVALID_ARGUMENT, error)
            }

        /** Accepts an absolute http(s) URL with a visible-ASCII body and no credentials. */
        private fun validateUrl(value: String) {
            val uri = parse(value)

            if (uri.scheme !in WEB_SCHEMES) throw invalid()

            requireDestination(uri)
            requireVisible(value)
        }

        /** A URL needs a host, and must not smuggle credentials past its authority. */
        private fun requireDestination(uri: URI) {
            if (uri.host.isNullOrEmpty() || uri.rawAuthority.isNullOrEmpty()) throw invalid()

            if ('@' in uri.rawAuthority || uri.userInfo != null) throw invalid()

            if (uri.port !in -1..MAX_PORT) throw invalid()
        }

        /** Only printable ASCII may appear literally, so no control hides in the value. */
        private fun requireVisible(value: String) {
            if (value.any { it.code !in FIRST_PRINTABLE..LAST_PRINTABLE || it == '\\' }) throw invalid()
        }

        private fun parse(value: String): URI =
            try {
                URI(value)
            } catch (error: URISyntaxException) {
                throw PluginException(PluginException.INVALID_ARGUMENT, error)
            }

        private fun invalid() = PluginException(PluginException.INVALID_ARGUMENT)

        /** Two u32 fields frame the value. */
        private const val HEADER_BYTES = 8

        /** Kind codes, matching the wire contract. */
        private const val TEXT = 0
        private const val URL = 1

        /** The largest value each kind may carry. */
        private const val MAX_TEXT_BYTES = 65536
        private const val MAX_URL_BYTES = 8192

        /** Only printable ASCII may appear literally in a shared URL. */
        private const val FIRST_PRINTABLE = 33
        private const val LAST_PRINTABLE = 126

        /** The highest port a URL may name. */
        private const val MAX_PORT = 65535

        private val WEB_SCHEMES = setOf("http", "https")
    }
}
