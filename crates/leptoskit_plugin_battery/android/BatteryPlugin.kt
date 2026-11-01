package dev.leptoskit.plugins.battery

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.BatteryManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.suspendCancellableCoroutine
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlin.math.roundToInt

/** One demand-driven waiter. OS notifications never accumulate in an event queue. */
class BatteryPlugin(
    context: Context,
) : NativePlugin {
    private val context = context.applicationContext
    private val handler = Handler(Looper.getMainLooper())
    private var waiter: CancellableContinuation<ByteArray>? = null
    private var previous = byteArrayOf()
    private var receiver: BroadcastReceiver? = null
    private var closed = false
    override val name = "battery"
    override val methods = intArrayOf(0, 1, 2)

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper())

        if (closed) throw PluginException(PluginException.CANCELLED)

        return when (method) {
            0 -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
                snapshot()
            }

            1 -> {
                val value = validateSnapshot(payload)

                if (waiter?.isActive == false) cleanup()

                if (waiter != null) throw PluginException(PluginException.BUSY)
                suspendCancellableCoroutine { continuation ->
                    waiter = continuation
                    previous = value
                    val observer =
                        object : BroadcastReceiver() {
                            override fun onReceive(
                                context: Context?,
                                intent: Intent?,
                            ) {
                                changed()
                            }
                        }
                    receiver = observer
                    val filter =
                        IntentFilter(Intent.ACTION_BATTERY_CHANGED).apply {
                            addAction(PowerManager.ACTION_POWER_SAVE_MODE_CHANGED)
                        }
                    try {
                        if (Build.VERSION.SDK_INT >= 33) {
                            context.registerReceiver(observer, filter, Context.RECEIVER_NOT_EXPORTED)
                        } else {
                            context.registerReceiver(observer, filter)
                        }
                    } catch (error: Exception) {
                        receiver = null
                        cleanup()

                        continuation.resumeWithException(error)

                        return@suspendCancellableCoroutine
                    }
                    continuation.invokeOnCancellation {
                        // Register first: an already-cancelled continuation runs this immediately.
                        val remove =
                            Runnable {
                                if (waiter === continuation) cleanup()
                            }

                        if (Looper.myLooper() == Looper.getMainLooper()) remove.run() else handler.post(remove)
                    }
                    changed()
                }
            }

            2 -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
                val power =
                    context.getSystemService(Context.POWER_SERVICE) as? PowerManager
                        ?: throw PluginException(PluginException.UNAVAILABLE)
                val enabled = !power.isIgnoringBatteryOptimizations(context.packageName)
                ByteBuffer
                    .allocate(4)
                    .order(ByteOrder.LITTLE_ENDIAN)
                    .putInt(if (enabled) 1 else 0)
                    .array()
            }

            else -> {
                throw PluginException(PluginException.UNAVAILABLE)
            }
        }
    }

    override fun close() {
        check(Looper.myLooper() == Looper.getMainLooper())
        closed = true
        val pending = waiter
        cleanup()
        pending?.cancel()
    }

    private fun changed() {
        val pending = waiter ?: return
        val value =
            try {
                snapshot()
            } catch (error: Exception) {
                cleanup()
                pending.resumeWithException(error)

                return
            }

        if (!value.contentEquals(previous)) {
            cleanup()
            pending.resume(value)
        }
    }

    private fun cleanup() {
        waiter = null
        previous = byteArrayOf()
        receiver?.let { context.unregisterReceiver(it) }
        receiver = null
    }

    private fun snapshot(): ByteArray {
        val reading =
            context.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
                ?: throw PluginException(PluginException.UNAVAILABLE)
        val present =
            if (!reading.hasExtra(BatteryManager.EXTRA_PRESENT)) {
                0
            } else if (reading.getBooleanExtra(BatteryManager.EXTRA_PRESENT, false)) {
                2
            } else {

                1
            }
        val level = reading.getIntExtra(BatteryManager.EXTRA_LEVEL, -1)
        val scale = reading.getIntExtra(BatteryManager.EXTRA_SCALE, -1)
        val charge = if (scale > 0 && level in 0..scale) (level.toDouble() * 1000 / scale).roundToInt() else -1
        val state =
            when (reading.getIntExtra(BatteryManager.EXTRA_STATUS, BatteryManager.BATTERY_STATUS_UNKNOWN)) {
                BatteryManager.BATTERY_STATUS_CHARGING -> 2
                BatteryManager.BATTERY_STATUS_FULL -> 3
                BatteryManager.BATTERY_STATUS_DISCHARGING, BatteryManager.BATTERY_STATUS_NOT_CHARGING -> 1
                else -> 0
            }
        val power = context.getSystemService(Context.POWER_SERVICE) as? PowerManager
        val saving =
            if (power == null) {
                0
            } else if (power.isPowerSaveMode) {
                2
            } else {
                1
            }

        return ByteBuffer
            .allocate(16)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(present)
            .putInt(charge)
            .putInt(state)
            .putInt(saving)
            .array()
    }
}

private fun validateSnapshot(payload: ByteBuffer): ByteArray {
    if (payload.remaining() != 16) throw PluginException(PluginException.INVALID_ARGUMENT)
    val bytes = ByteArray(16).also { payload.get(it) }
    val fields = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
    val presence = fields.int
    val charge = fields.int
    val state = fields.int
    val saving = fields.int

    if (presence !in 0..2 || (charge != -1 && charge !in 0..1000) || state !in 0..3 || saving !in 0..2) {

        throw PluginException(PluginException.INVALID_ARGUMENT)
    }

    return bytes
}
