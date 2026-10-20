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
) : NativePlugin {
    final override val methods = intArrayOf(0, 1)
    private val activity = context as? Activity
    private val manager = context.getSystemService(Context.SENSOR_SERVICE) as? SensorManager
    private val handler = Handler(Looper.getMainLooper())
    private var waiting: CancellableContinuation<ByteArray>? = null
    private var listener: SensorEventListener? = null
    private var lifecycle: Application.ActivityLifecycleCallbacks? = null
    private var closed = false

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
        if (payload.remaining() != 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        val interval = payload.order(ByteOrder.LITTLE_ENDIAN).int
        if (interval !in 5..60_000) throw PluginException(PluginException.INVALID_ARGUMENT)
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
                        val count = if (sensorType == Sensor.TYPE_PRESSURE) 1 else 3
                        if (event.values.size < count || (0 until count).any { !event.values[it].isFinite() } ||
                            (sensorType == Sensor.TYPE_PRESSURE && event.values[0] <= 0)
                        ) {
                            fail(continuation, PluginException(PluginException.NATIVE))
                            return
                        }
                        val bytes =
                            ByteBuffer
                                .allocate((count + 1) * 8)
                                .order(ByteOrder.LITTLE_ENDIAN)
                                .putDouble(event.timestamp.toDouble() / 1_000_000_000.0)
                        repeat(count) { bytes.putDouble(event.values[it].toDouble()) }
                        cleanup()
                        continuation.resume(bytes.array())
                    }
                }
            listener = observer
            continuation.invokeOnCancellation {
                handler.post { if (waiting === continuation) cleanup() }
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
        waiting = null
        listener?.let { manager?.unregisterListener(it) }
        listener = null
        lifecycle?.let { activity?.application?.unregisterActivityLifecycleCallbacks(it) }
        lifecycle = null
    }
}
