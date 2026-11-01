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
        fun decode(payload: ByteBuffer): ShareContent {
            val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
            if (bytes.remaining() < 8) throw PluginException(PluginException.INVALID_ARGUMENT)
            val kind = bytes.int
            val size = bytes.int
            if (kind !in 0..1 || size != bytes.remaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

            if (size > if (kind == 0) 65536 else 8192) throw PluginException(PluginException.TOO_LARGE)
            val value =
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

            if (value.isEmpty() || '\u0000' in value) throw PluginException(PluginException.INVALID_ARGUMENT)

            if (kind == 1) validateUrl(value)

            return ShareContent(kind, value)
        }

        private fun validateUrl(value: String) {
            val uri =
                try {
                    URI(value)
                } catch (error: URISyntaxException) {
                    throw PluginException(PluginException.INVALID_ARGUMENT, error)
                }
            if (uri.scheme !in listOf("http", "https") || uri.host.isNullOrEmpty() ||
                uri.rawAuthority.isNullOrEmpty() || '@' in uri.rawAuthority ||
                uri.userInfo != null || uri.port !in -1..65535 || value.any { it.code !in 33..126 || it == '\\' }
            ) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }
        }
    }
}
