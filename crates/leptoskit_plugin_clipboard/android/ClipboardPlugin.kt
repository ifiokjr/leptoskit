package dev.leptoskit.plugins.clipboard

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.os.Looper
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import dev.leptoskit.bridge.utf8
import java.nio.ByteBuffer

/** Activity/foreground restrictions are enforced by Android itself. */
class ClipboardPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "clipboard"
    override val methods = intArrayOf(0, 1, 2)

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Clipboard dispatch requires the UI thread" }
        val clipboard =
            context.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
                ?: throw PluginException(PluginException.UNAVAILABLE)
        try {
            return when (method) {
                0 -> {
                    requireEmpty(payload)
                    val clip = clipboard.primaryClip
                    val text =
                        if (clip != null && clip.itemCount > 0) {
                            clip
                                .getItemAt(0)
                                .coerceToText(context)
                                ?.toString()
                                .orEmpty()
                        } else {
                            ""
                        }
                    text.toByteArray(Charsets.UTF_8)
                }

                1 -> {
                    clipboard.setPrimaryClip(ClipData.newPlainText("", payload.utf8()))
                    byteArrayOf()
                }

                2 -> {
                    requireEmpty(payload)

                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                        clipboard.clearPrimaryClip()
                    } else {
                        clipboard.setPrimaryClip(ClipData.newPlainText("", ""))
                    }

                    byteArrayOf()
                }

                else -> {
                    throw PluginException(PluginException.UNAVAILABLE)
                }
            }
        } catch (error: SecurityException) {
            throw PluginException(PluginException.PERMISSION_DENIED, error)
        }
    }

    private fun requireEmpty(payload: ByteBuffer) {
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
    }
}
