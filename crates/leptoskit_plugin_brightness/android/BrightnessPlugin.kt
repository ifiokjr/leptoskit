package dev.leptoskit.plugins.brightness

import android.app.Activity
import android.content.Context
import android.os.Looper
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.math.roundToInt

/** Uses only activity-local brightness; never requests WRITE_SETTINGS. */
class BrightnessPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "brightness"
    override val methods = intArrayOf(0, 1, 2)
    private var original: Float? = null
    private var closed = false

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper())
        if (closed) throw PluginException(PluginException.CANCELLED)
        if (method !in 0..2) throw PluginException(PluginException.UNAVAILABLE)
        if (payload.remaining() != if (method == 1) 4 else 0) throw PluginException(PluginException.INVALID_ARGUMENT)
        val activity = context as? Activity ?: throw PluginException(PluginException.UNAVAILABLE)
        val attributes = activity.window.attributes
        when (method) {
            0 -> {
                val level = attributes.screenBrightness
                if (!level.isFinite() || (level != -1f && level !in 0f..1f)) throw PluginException(PluginException.NATIVE)
                return ByteBuffer
                    .allocate(4)
                    .order(ByteOrder.LITTLE_ENDIAN)
                    .putInt(if (level == -1f) -1 else (level * 1000).roundToInt())
                    .array()
            }

            1 -> {
                val level = payload.order(ByteOrder.LITTLE_ENDIAN).int
                if (level !in 0..1000) throw PluginException(PluginException.INVALID_ARGUMENT)
                if (original == null) original = attributes.screenBrightness
                attributes.screenBrightness = level.toFloat() / 1000f
                activity.window.attributes = attributes
            }

            2 -> {
                restore()
            }
        }
        return byteArrayOf()
    }

    override fun close() {
        check(Looper.myLooper() == Looper.getMainLooper())
        if (closed) return
        closed = true
        restore()
    }

    private fun restore() {
        val saved = original ?: return
        val activity = context as? Activity ?: return
        val attributes = activity.window.attributes
        attributes.screenBrightness = saved
        activity.window.attributes = attributes
        original = null
    }
}
