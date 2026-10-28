package dev.leptoskit.arena.bridge

import android.app.Activity
import android.content.Context
import android.os.Build
import android.util.Log
import android.view.Choreographer
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.suspendCancellableCoroutine
import org.json.JSONObject
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.coroutines.resume

/**
 * The arena's native module: launch scenario, logcat report, a Choreographer
 * frame clock, and the ArenaEcho methods. Method ids match `arena_bridge`'s
 * Rust contract.
 */
class ArenaPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "arena"
    override val methods = intArrayOf(0, 1, 2, 10, 11, 12, 13, 14)

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray =
        when (method) {
            0 -> scenario().toByteArray(Charsets.UTF_8)
            1 -> {
                bytes(payload).toString(Charsets.UTF_8).split('\n').forEach { Log.i(TAG, it) }
                byteArrayOf()
            }
            2 -> nextFrame()
            10 -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
                byteArrayOf()
            }
            11 -> {
                if (payload.remaining() != 4) throw PluginException(PluginException.INVALID_ARGUMENT)
                val value = payload.order(ByteOrder.LITTLE_ENDIAN).int
                ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(value + 1).array()
            }
            12 -> bytes(payload).toString(Charsets.UTF_8).toByteArray(Charsets.UTF_8)
            13 -> bytes(payload)
            14 -> JSONObject(bytes(payload).toString(Charsets.UTF_8)).toString().toByteArray(Charsets.UTF_8)
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }

    /** The `scenario` string extra the activity was started with. */
    private fun scenario(): String = (context as? Activity)?.intent?.getStringExtra("scenario") ?: ""

    private suspend fun nextFrame(): ByteArray {
        val timestamp =
            suspendCancellableCoroutine { continuation ->
                val callback = Choreographer.FrameCallback { continuation.resume(it) }
                Choreographer.getInstance().postFrameCallback(callback)
                continuation.invokeOnCancellation { Choreographer.getInstance().removeFrameCallback(callback) }
            }
        return ByteBuffer
            .allocate(16)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putLong(timestamp)
            .putLong((1_000_000_000.0 / refreshRate()).toLong())
            .array()
    }

    private fun refreshRate(): Float {
        val display =
            if (Build.VERSION.SDK_INT >= 30) {
                context.display
            } else {
                @Suppress("DEPRECATION")
                (context as? Activity)?.windowManager?.defaultDisplay
            }
        return display?.refreshRate?.takeIf { it > 0f } ?: 60f
    }

    private fun bytes(payload: ByteBuffer): ByteArray = ByteArray(payload.remaining()).also { payload.get(it) }

    private companion object {
        const val TAG = "ARENA"
    }
}
