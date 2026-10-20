package dev.leptoskit.sensors

import android.app.Activity
import android.app.Application
import android.content.Context
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.suspendCancellableCoroutine
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/** One fresh sensor event per call. Cancellation/close/background unregister the sensor. */
open class MotionSensorPlugin(
    context: Context,
    final override val name: String,
    private val sensorType: Int,
    private val bounded: Boolean = false,
    private val authorize: () -> Unit = {},
) : NativePlugin {
    final override val methods = intArrayOf(0, 1)
    private val activity = context as? Activity
    private val manager = context.getSystemService(Context.SENSOR_SERVICE) as? SensorManager
    private val handler = Handler(Looper.getMainLooper())
    private var waiting: CancellableContinuation<ByteArray>? = null
    private var listener: SensorEventListener? = null
    private var lifecycle: Application.ActivityLifecycleCallbacks? = null
    private var closed = false
    private var deadline: Runnable? = null

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)
        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)
        val sensors = manager
        val sensor = sensors?.getDefaultSensor(sensorType)
        if (method == 0) {
            if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
            return byteArrayOf(if (sensor == null) 0 else 1, 0, 0, 0)
        }
        if (payload.remaining() != if (bounded) 8 else 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        val interval = payload.order(ByteOrder.LITTLE_ENDIAN).int
        if (interval !in 5..60_000) throw PluginException(PluginException.INVALID_ARGUMENT)
        val timeout = if (bounded) payload.int else 0
        if (bounded && timeout !in 1..120_000) throw PluginException(PluginException.INVALID_ARGUMENT)
        if (bounded && activity != null && !activity.hasWindowFocus()) throw PluginException(PluginException.BUSY)
        authorize()
        if (waiting != null) throw PluginException(PluginException.BUSY)
        if (sensors == null || sensor == null) throw PluginException(PluginException.UNAVAILABLE)
        return suspendCancellableCoroutine { continuation ->
            waiting = continuation
            val started = SystemClock.elapsedRealtimeNanos()
            val observer =
                object : SensorEventListener {
                    override fun onAccuracyChanged(
                        sensor: Sensor?,
                        accuracy: Int,
                    ) = Unit

                    override fun onSensorChanged(event: SensorEvent) {
                        if (waiting !== continuation || !continuation.isActive || event.timestamp < started) return
                        val bytes =
                            try {
                                authorize()
                                encodeSensorEvent(sensorType, event.timestamp, event.values, bounded)
                            } catch (error: PluginException) {
                                fail(continuation, error)
                                return
                            }
                        cleanup()
                        continuation.resume(bytes)
                    }
                }
            listener = observer
            continuation.invokeOnCancellation {
                handler.post { if (waiting === continuation) cleanup() }
            }
            if (!continuation.isActive) {
                cleanup()
                return@suspendCancellableCoroutine
            }
            if (bounded) {
                val task =
                    Runnable {
                        if (waiting === continuation) {
                            cleanup()
                            if (continuation.isActive) continuation.resume(byteArrayOf(1, 0, 0, 0))
                        }
                    }
                deadline = task
                handler.postDelayed(task, timeout.toLong())
            }
            try {
                if (!sensors.registerListener(observer, sensor, interval * 1000, handler)) {
                    fail(continuation, PluginException(PluginException.UNAVAILABLE))
                } else if (activity != null) {
                    val callbacks =
                        object : Application.ActivityLifecycleCallbacks {
                            override fun onActivityPaused(current: Activity) {
                                if (current === activity && waiting === continuation) {
                                    cleanup()
                                    continuation.cancel()
                                }
                            }

                            override fun onActivityCreated(
                                current: Activity,
                                state: Bundle?,
                            ) = Unit

                            override fun onActivityStarted(current: Activity) = Unit

                            override fun onActivityResumed(current: Activity) = Unit

                            override fun onActivityStopped(current: Activity) = Unit

                            override fun onActivitySaveInstanceState(
                                current: Activity,
                                state: Bundle,
                            ) = Unit

                            override fun onActivityDestroyed(current: Activity) = onActivityPaused(current)
                        }
                    lifecycle = callbacks
                    activity.application.registerActivityLifecycleCallbacks(callbacks)
                }
            } catch (error: SecurityException) {
                fail(continuation, PluginException(PluginException.PERMISSION_DENIED, error))
            } catch (error: RuntimeException) {
                fail(continuation, PluginException(PluginException.NATIVE, error))
            }
        }
    }

    override fun close() {
        closed = true
        val continuation = waiting
        cleanup()
        continuation?.cancel()
    }

    private fun fail(
        continuation: CancellableContinuation<ByteArray>,
        error: PluginException,
    ) {
        if (waiting !== continuation) return
        cleanup()
        if (continuation.isActive) continuation.resumeWithException(error)
    }

    private fun cleanup() {
        deadline?.let { handler.removeCallbacks(it) }
        deadline = null
        waiting = null
        listener?.let { manager?.unregisterListener(it) }
        listener = null
        lifecycle?.let { activity?.application?.unregisterActivityLifecycleCallbacks(it) }
        lifecycle = null
    }
}

/** Fixed, validated wire encoding; boot counters are not daily totals. */
internal fun encodeSensorEvent(
    type: Int,
    timestamp: Long,
    values: FloatArray,
    bounded: Boolean,
): ByteArray {
    val scalar = type == Sensor.TYPE_PRESSURE || type == Sensor.TYPE_LIGHT || type == Sensor.TYPE_STEP_COUNTER
    val count = if (scalar) 1 else 3
    if (timestamp < 0 || values.size < count || (0 until count).any { !values[it].isFinite() } ||
        (type == Sensor.TYPE_PRESSURE && values[0] <= 0) ||
        ((type == Sensor.TYPE_LIGHT || type == Sensor.TYPE_STEP_COUNTER) && values[0] < 0)
    ) {
        throw PluginException(PluginException.NATIVE)
    }
    val size = if (type == Sensor.TYPE_STEP_COUNTER) 32 else (count + 1) * 8
    val bytes = ByteBuffer.allocate(size + if (bounded) 4 else 0).order(ByteOrder.LITTLE_ENDIAN)
    if (bounded) bytes.putInt(0)
    if (type == Sensor.TYPE_STEP_COUNTER) {
        val steps = values[0].toDouble()
        if (steps >= Long.MAX_VALUE.toDouble() || steps != kotlin.math.floor(steps)) throw PluginException(PluginException.NATIVE)
        bytes
            .putInt(1)
            .putInt(0)
            .putLong(steps.toLong())
            .putDouble(timestamp.toDouble() / 1_000_000_000.0)
            .putDouble(0.0)
    } else {
        bytes.putDouble(timestamp.toDouble() / 1_000_000_000.0)
        repeat(count) { bytes.putDouble(values[it].toDouble()) }
    }
    return bytes.array()
}
