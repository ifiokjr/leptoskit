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
    override val methods = intArrayOf(0, 1, 2)

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (method !in 0..2) throw PluginException(PluginException.UNAVAILABLE)
        val intent =
            if (method == 2) {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
                Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null))
            } else {
                Intent(Intent.ACTION_VIEW, validatedUri(decode(payload))).addCategory(Intent.CATEGORY_BROWSABLE)
            }

        if (method == 1) {
            // Android package visibility can hide handlers without matching <queries> declarations.
            return byteArrayOf(if (intent.resolveActivity(context.packageManager) != null) 1 else 0, 0, 0, 0)
        }

        if (context !is Activity) intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        try {
            context.startActivity(intent)
        } catch (_: ActivityNotFoundException) {
            throw PluginException(PluginException.UNAVAILABLE)

        } catch (_: SecurityException) {
            throw PluginException(PluginException.PERMISSION_DENIED)
        }

        return byteArrayOf()
    }

    private fun decode(payload: ByteBuffer): String {
        if (payload.remaining() !in 1..8192) throw PluginException(PluginException.INVALID_ARGUMENT)

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

    internal fun validatedUri(value: String): Uri {
        if (value.isEmpty() || value.length > 8192 || value.any { it.code !in 33..126 || it == '\\' }) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        for (index in value.indices) {
            if (value[index] != '%') continue
            val decoded = if (index + 2 < value.length) value.substring(index + 1, index + 3).toIntOrNull(16) else null

            if (decoded == null || decoded < 32 || decoded == 127) throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        val parsed =
            try {

                URI(value)
            } catch (_: URISyntaxException) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }
        when (parsed.scheme?.lowercase(Locale.ROOT)) {
            "http", "https" -> {
                if (parsed.host.isNullOrEmpty() || parsed.rawUserInfo != null) {
                    throw PluginException(PluginException.INVALID_ARGUMENT)
                }
            }

            "mailto", "tel", "sms" -> {
                if (parsed.rawSchemeSpecificPart.isNullOrEmpty() || parsed.rawSchemeSpecificPart.startsWith("/") ||
                    parsed.rawSchemeSpecificPart.startsWith("?")
                ) {
                    throw PluginException(PluginException.INVALID_ARGUMENT)
                }
            }

            else -> {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }
        }

        return Uri.parse(value)
    }
}
