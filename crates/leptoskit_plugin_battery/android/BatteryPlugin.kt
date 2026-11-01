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

/** Four little-endian u32 fields: presence, charge, state, and power saving. */
private const val SNAPSHOT_BYTES = 16

/** Four bytes per wire field. */
private const val FIELD_BYTES = 4

/** One u32 boolean, as the optimisation reply carries it. */
private const val FLAG_BYTES = 4

/** Android requires an explicit receiver export flag from this API level. */
private const val EXPORT_REQUIRED_API = 33

/** Charge is carried in thousandths of full. */
private const val THOUSANDTHS = 1000.0

/** The reserved maximum stands in for a charge the gauge cannot measure. */
private const val UNKNOWN_CHARGE = -1

/** Tri-state encodings shared by every boolean field. */
private const val FLAG_UNKNOWN = 0
private const val FLAG_FALSE = 1
private const val FLAG_TRUE = 2

/** Wire codes for the charging state. */
private const val STATE_UNKNOWN = 0
private const val STATE_DISCHARGING = 1
private const val STATE_CHARGING = 2
private const val STATE_FULL = 3

/** The valid range of every validated field. */
private const val LAST_FLAG = 2
private const val LAST_STATE = 3
private const val FULL_CHARGE = 1000

/** Method identifiers. */
private const val SNAPSHOT = 0
private const val WAIT_FOR_CHANGE = 1
private const val OPTIMIZATION_ENABLED = 2

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
    override val methods = intArrayOf(SNAPSHOT, WAIT_FOR_CHANGE, OPTIMIZATION_ENABLED)

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper())

        if (closed) throw PluginException(PluginException.CANCELLED)

        return when (method) {
            SNAPSHOT -> currentSnapshot(payload)
            WAIT_FOR_CHANGE -> awaitChange(payload)
            OPTIMIZATION_ENABLED -> optimizationEnabled(payload)
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }
    }

    override fun close() {
        check(Looper.myLooper() == Looper.getMainLooper())
        closed = true
        val pending = waiter
        cleanup()
        pending?.cancel()
    }

    /** Answers one reading; the snapshot call takes no arguments. */
    private fun currentSnapshot(payload: ByteBuffer): ByteArray {
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        return snapshot()
    }

    /** Answers Android's app-specific battery optimisation policy as a u32 boolean. */
    private fun optimizationEnabled(payload: ByteBuffer): ByteArray {
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        val power =
            context.getSystemService(Context.POWER_SERVICE) as? PowerManager
                ?: throw PluginException(PluginException.UNAVAILABLE)
        val enabled = !power.isIgnoringBatteryOptimizations(context.packageName)

        return ByteBuffer
            .allocate(FLAG_BYTES)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(if (enabled) 1 else 0)
            .array()
    }

    /** Suspends until the OS reports a reading other than the caller's last one. */
    private suspend fun awaitChange(payload: ByteBuffer): ByteArray {
        val expected = validateSnapshot(payload)

        if (waiter?.isActive == false) cleanup()

        if (waiter != null) throw PluginException(PluginException.BUSY)

        return suspendCancellableCoroutine { continuation ->
            waiting(continuation, expected)
        }
    }

    /** Installs the receiver for one waiter, or fails it when the OS refuses. */
    private fun waiting(
        continuation: CancellableContinuation<ByteArray>,
        expected: ByteArray,
    ) {
        waiter = continuation
        previous = expected
        val observer = broadcastObserver()
        receiver = observer

        try {
            register(observer)
        } catch (error: Exception) {
            receiver = null
            cleanup()
            continuation.resumeWithException(error)

            return
        }

        continuation.invokeOnCancellation { release(continuation) }
        changed()
    }

    /** Reports every battery or power-save broadcast to the pending waiter. */
    private fun broadcastObserver(): BroadcastReceiver =
        object : BroadcastReceiver() {
            override fun onReceive(
                context: Context?,
                intent: Intent?,
            ) = changed()
        }

    /** Registers for every event that can change a reading, on any supported API. */
    private fun register(observer: BroadcastReceiver) {
        val filter =
            IntentFilter(Intent.ACTION_BATTERY_CHANGED).apply {
                addAction(PowerManager.ACTION_POWER_SAVE_MODE_CHANGED)
            }

        if (Build.VERSION.SDK_INT >= EXPORT_REQUIRED_API) {
            context.registerReceiver(observer, filter, Context.RECEIVER_NOT_EXPORTED)
        } else {
            context.registerReceiver(observer, filter)
        }
    }

    /**
     * Drops the receiver for a cancelled caller. The cancellation callback can
     * run while the receiver is still being registered, so it only acts when
     * this continuation is the one waiting.
     */
    private fun release(continuation: CancellableContinuation<ByteArray>) {
        val remove =
            Runnable {
                if (waiter === continuation) cleanup()
            }

        if (Looper.myLooper() == Looper.getMainLooper()) remove.run() else handler.post(remove)
    }

    /** Resumes the waiter once, and only when the reading actually differs. */
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

    /** Encodes the last battery broadcast; anything it omits stays unknown. */
    private fun snapshot(): ByteArray {
        val reading =
            context.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
                ?: throw PluginException(PluginException.UNAVAILABLE)

        return ByteBuffer
            .allocate(SNAPSHOT_BYTES)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(presence(reading))
            .putInt(charge(reading))
            .putInt(state(reading))
            .putInt(powerSave())
            .array()
    }

    /** Presence is its own field, so an absent battery is not an unknown one. */
    private fun presence(reading: Intent): Int =
        when {
            !reading.hasExtra(BatteryManager.EXTRA_PRESENT) -> FLAG_UNKNOWN
            reading.getBooleanExtra(BatteryManager.EXTRA_PRESENT, false) -> FLAG_TRUE
            else -> FLAG_FALSE
        }

    /** Charge in thousandths, or the reserved maximum when the gauge cannot say. */
    private fun charge(reading: Intent): Int {
        val level = reading.getIntExtra(BatteryManager.EXTRA_LEVEL, -1)
        val scale = reading.getIntExtra(BatteryManager.EXTRA_SCALE, -1)

        if (scale <= 0 || level !in 0..scale) return UNKNOWN_CHARGE

        return (level.toDouble() * THOUSANDTHS / scale).roundToInt()
    }

    /** Maps the Android charging status to the wire state. */
    private fun state(reading: Intent): Int =
        when (reading.getIntExtra(BatteryManager.EXTRA_STATUS, BatteryManager.BATTERY_STATUS_UNKNOWN)) {
            BatteryManager.BATTERY_STATUS_CHARGING -> STATE_CHARGING
            BatteryManager.BATTERY_STATUS_FULL -> STATE_FULL
            BatteryManager.BATTERY_STATUS_DISCHARGING, BatteryManager.BATTERY_STATUS_NOT_CHARGING -> STATE_DISCHARGING
            else -> STATE_UNKNOWN
        }

    /** System-wide power-save mode, unknown when the service is unavailable. */
    private fun powerSave(): Int {
        val power = context.getSystemService(Context.POWER_SERVICE) as? PowerManager ?: return FLAG_UNKNOWN

        return if (power.isPowerSaveMode) FLAG_TRUE else FLAG_FALSE
    }
}

/** Echoes the caller's last reading back, refusing fields the wire cannot carry. */
private fun validateSnapshot(payload: ByteBuffer): ByteArray {
    if (payload.remaining() != SNAPSHOT_BYTES) throw PluginException(PluginException.INVALID_ARGUMENT)

    val bytes = ByteArray(SNAPSHOT_BYTES).also { payload.get(it) }
    val fields = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
    val presence = fields.int
    val charge = fields.int
    val state = fields.int
    val saving = fields.int

    if (presence !in 0..LAST_FLAG ||
        (charge != UNKNOWN_CHARGE && charge !in 0..FULL_CHARGE) ||
        state !in 0..LAST_STATE ||
        saving !in 0..LAST_FLAG
    ) {
        throw PluginException(PluginException.INVALID_ARGUMENT)
    }

    return bytes
}
