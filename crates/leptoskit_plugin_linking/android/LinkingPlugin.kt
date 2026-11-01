package dev.leptoskit.plugins.linking

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.Settings
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.net.URI
import java.net.URISyntaxException
import java.nio.ByteBuffer
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction
import java.util.Locale

/** Outbound links only; incoming intents belong to the app host lifecycle. */
class LinkingPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "linking"
    override val methods = intArrayOf(OPEN, CAN_OPEN, OPEN_SETTINGS)

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray =
        when (method) {
            OPEN_SETTINGS -> openSettings(payload)
            OPEN, CAN_OPEN -> openOrQuery(method, payload)
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }

    /** Opens this application's system settings page, which takes no arguments. */
    private fun openSettings(payload: ByteBuffer): ByteArray {
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
        val application = Uri.fromParts("package", context.packageName, null)

        launch(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, application))

        return byteArrayOf()
    }

    /** Answers whether a handler exists, or hands the link to the system. */
    private fun openOrQuery(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        val link = validatedUri(decode(payload))
        val intent = Intent(Intent.ACTION_VIEW, link).addCategory(Intent.CATEGORY_BROWSABLE)

        if (method == CAN_OPEN) {
            // Android package visibility can hide handlers without matching <queries> declarations.
            return booleanReply(intent.resolveActivity(context.packageManager) != null)
        }

        launch(intent)

        return byteArrayOf()
    }

    /** Starts the intent, translating the launcher's two refusal modes. */
    private fun launch(intent: Intent) {
        if (context !is Activity) intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

        try {
            context.startActivity(intent)
        } catch (_: ActivityNotFoundException) {
            throw PluginException(PluginException.UNAVAILABLE)
        } catch (_: SecurityException) {
            throw PluginException(PluginException.PERMISSION_DENIED)
        }
    }

    /** A one-byte truth padded to the four bytes every boolean reply carries. */
    private fun booleanReply(value: Boolean): ByteArray = byteArrayOf(if (value) 1 else 0, 0, 0, 0)

    /** Decodes strict UTF-8, refusing payloads that cannot hold a URL. */
    private fun decode(payload: ByteBuffer): String {
        if (payload.remaining() !in 1..MAX_URL_BYTES) throw PluginException(PluginException.INVALID_ARGUMENT)

        return try {
            Charsets.UTF_8
                .newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(payload)
                .toString()
        } catch (_: CharacterCodingException) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
    }

    /**
     * Accepts an absolute http(s), mailto, tel or sms link. Whitespace,
     * backslashes, escaped controls and credentials are refused before the
     * launcher sees them.
     */
    internal fun validatedUri(value: String): Uri {
        requireEncoded(value)
        requireVisibleEscapes(value)
        val parsed = parseUri(value)

        when (parsed.scheme?.lowercase(Locale.ROOT)) {
            "http", "https" -> requireWebDestination(parsed)
            "mailto", "tel", "sms" -> requireContactDestination(parsed)
            else -> throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        return Uri.parse(value)
    }

    /** Limits the payload to printable ASCII, the only text a URL carries raw. */
    private fun requireEncoded(value: String) {
        if (value.isEmpty() || value.length > MAX_URL_BYTES) throw PluginException(PluginException.INVALID_ARGUMENT)

        val printable = value.all { it.code in FIRST_PRINTABLE..LAST_PRINTABLE && it != '\\' }

        if (!printable) throw PluginException(PluginException.INVALID_ARGUMENT)
    }

    /** Rejects escapes that would decode to a control character downstream. */
    private fun requireVisibleEscapes(value: String) {
        for (index in value.indices) {
            if (value[index] == '%' && !isVisibleEscape(value, index)) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }
        }
    }

    private fun isVisibleEscape(
        value: String,
        index: Int,
    ): Boolean {
        if (index + 2 >= value.length) return false

        val decoded = value.substring(index + 1, index + 3).toIntOrNull(HEX_RADIX) ?: return false

        return decoded >= FIRST_VISIBLE && decoded != DELETE
    }

    private fun parseUri(value: String): URI =
        try {
            URI(value)
        } catch (_: URISyntaxException) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

    /** A web link needs a host, and must not smuggle credentials past it. */
    private fun requireWebDestination(parsed: URI) {
        if (parsed.host.isNullOrEmpty() || parsed.rawUserInfo != null) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
    }

    /** Contact links carry their target after the colon, never as a host or path. */
    private fun requireContactDestination(parsed: URI) {
        val target = parsed.rawSchemeSpecificPart

        if (target.isNullOrEmpty() || target.startsWith("/") || target.startsWith("?")) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
    }

    private companion object {
        /** Hands a URL to the handler the system picks. */
        const val OPEN = 0

        /** Answers whether an installed handler claims the URL. */
        const val CAN_OPEN = 1

        /** Opens this application's page in the system settings. */
        const val OPEN_SETTINGS = 2

        /** The longest payload that can still be a usable URL. */
        const val MAX_URL_BYTES = 8192

        /** The printable ASCII range a URL may carry literally. */
        const val FIRST_PRINTABLE = 33
        const val LAST_PRINTABLE = 126

        /** Escapes at or below this code hide a control character. */
        const val FIRST_VISIBLE = 32

        /** The delete code never belongs in a link. */
        const val DELETE = 127

        /** Two hexadecimal digits decode one byte. */
        const val HEX_RADIX = 16
    }
}
