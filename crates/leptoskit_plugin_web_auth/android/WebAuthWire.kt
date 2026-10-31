package dev.leptoskit.plugins.webauth

import dev.leptoskit.bridge.PluginException
import java.net.URI
import java.net.URISyntaxException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction
import java.util.Locale

/** The path every return address has; the server checks the same one. */
internal const val RETURN_PATH = "/auth/native/return"

/** This app's private-use return address (RFC 8252 §7.1), or null for an ID that can't be a scheme. */
internal fun returnAddress(packageName: String): String? {
    val scheme = packageName.lowercase(Locale.ROOT)
    val segments = scheme.split('.')
    val valid =
        segments.size >= 2 &&
            segments.all { segment ->
                segment.isNotEmpty() && segment[0] in 'a'..'z' && segment.all { it in 'a'..'z' || it in '0'..'9' }
            }
    return if (valid) "$scheme:$RETURN_PATH" else null
}

/** BEGIN's answer: the session's little-endian ID, then its return address. */
internal fun encodeSession(
    id: Int,
    returnTo: String,
): ByteArray {
    val address = returnTo.toByteArray(Charsets.UTF_8)
    return ByteBuffer
        .allocate(4 + address.size)
        .order(ByteOrder.LITTLE_ENDIAN)
        .putInt(id)
        .put(address)
        .array()
}

/** An AUTHENTICATE request: the session, and the provider's page. */
internal class WebAuthRequest(
    val session: Int,
    val url: URI,
) {
    companion object {
        fun decode(payload: ByteBuffer): WebAuthRequest {
            val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
            if (bytes.remaining() !in 5..(4 + 8192)) throw PluginException(PluginException.INVALID_ARGUMENT)
            val session = bytes.int
            val text =
                try {
                    Charsets.UTF_8
                        .newDecoder()
                        .onMalformedInput(CodingErrorAction.REPORT)
                        .onUnmappableCharacter(CodingErrorAction.REPORT)
                        .decode(bytes)
                        .toString()
                } catch (_: CharacterCodingException) {
                    throw PluginException(PluginException.INVALID_ARGUMENT)
                }
            return WebAuthRequest(session, providerPage(text))
        }

        /** `https`, or `http` on this device for development; no credentials. */
        fun providerPage(text: String): URI {
            if (text.any { it.code !in 33..126 }) throw PluginException(PluginException.INVALID_ARGUMENT)
            val url =
                try {
                    URI(text)
                } catch (_: URISyntaxException) {
                    throw PluginException(PluginException.INVALID_ARGUMENT)
                }
            val host = url.host ?: throw PluginException(PluginException.INVALID_ARGUMENT)
            val local = host == "localhost" || host == "127.0.0.1" || host == "[::1]"
            val scheme = url.scheme?.lowercase(Locale.ROOT)
            if (url.rawUserInfo != null || !(scheme == "https" || (scheme == "http" && local))) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }
            return url
        }
    }
}

/** Whether the browser came back to [returnTo] itself, with only a query. */
internal fun cameBack(
    address: String,
    returnTo: String,
): Boolean {
    val back =
        try {
            URI(address)
        } catch (_: URISyntaxException) {
            return false
        }
    val expected = URI(returnTo)
    return back.scheme.equals(expected.scheme, ignoreCase = true) &&
        back.rawAuthority == expected.rawAuthority &&
        back.rawPath == expected.rawPath &&
        back.rawFragment == null
}
