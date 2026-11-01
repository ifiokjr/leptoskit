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
    override val methods = intArrayOf(READ, WRITE, CLEAR)

    private companion object {
        /** Returns the clipboard's text. */
        const val READ = 0

        /** Replaces the clipboard's text. */
        const val WRITE = 1

        /** Empties the clipboard. */
        const val CLEAR = 2
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Clipboard dispatch requires the UI thread" }
        val clipboard =
            context.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
                ?: throw PluginException(PluginException.UNAVAILABLE)
        try {
            return dispatch(clipboard, method, payload)
        } catch (error: SecurityException) {
            throw PluginException(PluginException.PERMISSION_DENIED, error)
        }
    }

    /** Runs one clipboard call, which the platform may refuse. */
    private fun dispatch(
        clipboard: ClipboardManager,
        method: Int,
        payload: ByteBuffer,
    ): ByteArray =
        when (method) {
            READ -> {
                requireEmpty(payload)

                readText(clipboard).toByteArray(Charsets.UTF_8)
            }

            WRITE -> {
                clipboard.setPrimaryClip(ClipData.newPlainText("", payload.utf8()))

                byteArrayOf()
            }

            CLEAR -> {
                requireEmpty(payload)
                clear(clipboard)

                byteArrayOf()
            }

            else -> {
                throw PluginException(PluginException.UNAVAILABLE)
            }
        }

    /** The first item's text, or empty when the clipboard holds nothing. */
    private fun readText(clipboard: ClipboardManager): String {
        val clip = clipboard.primaryClip ?: return ""

        if (clip.itemCount == 0) return ""

        return clip
            .getItemAt(0)
            .coerceToText(context)
            ?.toString()
            .orEmpty()
    }

    /** Empties the clipboard, the way its API level allows. */
    private fun clear(clipboard: ClipboardManager) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            clipboard.clearPrimaryClip()
        } else {
            clipboard.setPrimaryClip(ClipData.newPlainText("", ""))
        }
    }

    private fun requireEmpty(payload: ByteBuffer) {
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
    }
}
