package dev.leptoskit.plugins.brightness

import android.app.Activity
import android.content.Context
import android.os.Looper
import android.view.WindowManager
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
    override val methods = intArrayOf(READ, WRITE, RESTORE)
    private var original: Float? = null
    private var closed = false

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper())

        if (closed) throw PluginException(PluginException.CANCELLED)

        requireWireArguments(method, payload)

        val activity = context as? Activity ?: throw PluginException(PluginException.UNAVAILABLE)
        val attributes = activity.window.attributes

        when (method) {
            READ -> return readLevel(attributes)
            WRITE -> applyLevel(payload, activity, attributes)
            RESTORE -> restore()
        }

        return byteArrayOf()
    }

    /** Only `write` carries an argument, and no method takes more than one u32. */
    private fun requireWireArguments(
        method: Int,
        payload: ByteBuffer,
    ) {
        if (method !in READ..RESTORE) throw PluginException(PluginException.UNAVAILABLE)

        val expectedBytes = if (method == WRITE) LEVEL_BYTES else 0

        if (payload.remaining() != expectedBytes) throw PluginException(PluginException.INVALID_ARGUMENT)
    }

    override fun close() {
        check(Looper.myLooper() == Looper.getMainLooper())

        if (closed) return

        closed = true
        restore()
    }

    /** Reads the window level as thousandths, or the sentinel when it inherits. */
    private fun readLevel(attributes: WindowManager.LayoutParams): ByteArray {
        val level = attributes.screenBrightness

        if (!level.isFinite() || (level != INHERITED && level !in 0f..1f)) throw PluginException(PluginException.NATIVE)

        val thousandths = if (level == INHERITED) INHERITED_LEVEL else (level * THOUSANDTHS_PER_UNIT).roundToInt()

        return ByteBuffer
            .allocate(LEVEL_BYTES)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(thousandths)
            .array()
    }

    /** Applies a thousandths level, remembering the window's baseline once. */
    private fun applyLevel(
        payload: ByteBuffer,
        activity: Activity,
        attributes: WindowManager.LayoutParams,
    ) {
        val level = payload.order(ByteOrder.LITTLE_ENDIAN).int

        if (level !in 0..THOUSANDTHS_PER_UNIT) throw PluginException(PluginException.INVALID_ARGUMENT)

        if (original == null) original = attributes.screenBrightness

        attributes.screenBrightness = level.toFloat() / THOUSANDTHS_PER_UNIT
        activity.window.attributes = attributes
    }

    /** Puts the saved level back and forgets it. Safe to repeat. */
    private fun restore() {
        val saved = original ?: return
        val activity = context as? Activity ?: return
        val attributes = activity.window.attributes

        attributes.screenBrightness = saved
        activity.window.attributes = attributes
        original = null
    }

    private companion object {
        /** Reports the window's current inherited or explicit level. */
        const val READ = 0

        /** Applies an explicit level and keeps the previous one for restore. */
        const val WRITE = 1

        /** Puts the level captured before the first override back. */
        const val RESTORE = 2

        /** Levels cross the wire as one little-endian u32. */
        const val LEVEL_BYTES = 4

        /** Brightness crosses the wire as thousandths of a unit fraction. */
        const val THOUSANDTHS_PER_UNIT = 1000

        /** The window attribute meaning "no explicit level yet". */
        const val INHERITED = -1f

        /** The same sentinel on the wire. */
        const val INHERITED_LEVEL = -1
    }
}
