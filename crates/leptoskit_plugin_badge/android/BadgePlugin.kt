package dev.leptoskit.plugins.badge

import android.app.NotificationManager
import android.content.Context
import android.os.Build
import android.os.Looper
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Tracks the badge count the app last requested. On Android 8+, launcher badges
 * are driven by active notifications; this plugin stores the count and clears the
 * app's notifications when the count is zero. It never invents a notification.
 */
class BadgePlugin(
    context: Context,
) : NativePlugin {
    override val name = "badge"
    override val methods = intArrayOf(GET, SET, CLEAR)

    private val manager = context.getSystemService(NotificationManager::class.java)
    private var closed = false
    private var count = 0

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        checkMain()
        if (closed) throw PluginException(PluginException.CANCELLED)
        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)

        return when (method) {
            GET -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
                encode(count)
            }
            SET -> {
                set(decode(payload))
                byteArrayOf()
            }
            else -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
                set(0)
                byteArrayOf()
            }
        }
    }

    override fun close() {
        closed = true
    }

    private fun set(value: Int) {
        if (value !in 0..MAX_COUNT) throw PluginException(PluginException.INVALID_ARGUMENT)
        count = value
        if (value == 0 && Build.VERSION.SDK_INT >= 23) {
            manager?.cancelAll()
        }
    }

    private fun decode(payload: ByteBuffer): Int {
        if (payload.remaining() != 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        val value = payload.order(ByteOrder.LITTLE_ENDIAN).int
        if (value !in 0..MAX_COUNT) throw PluginException(PluginException.INVALID_ARGUMENT)
        return value
    }

    private fun encode(value: Int): ByteArray =
        ByteBuffer
            .allocate(4)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(value)
            .array()

    private fun checkMain() {
        check(Looper.myLooper() == Looper.getMainLooper())
    }

    private companion object {
        const val GET = 0
        const val SET = 1
        const val CLEAR = 2
        const val MAX_COUNT = 999_999
    }
}
