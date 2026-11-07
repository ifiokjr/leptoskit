package dev.leptoskit.sensors

import android.app.Activity
import android.app.Application
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.os.Bundle
import android.os.Handler
import android.os.SystemClock
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/**
 * One plugin's stream of one sensor type, served from a [SampleRing].
 *
 * One request can be outstanding. After answering it the sensor keeps running
 * for a short grace period, so a reader that asks again promptly misses
 * nothing. Cancelling the pending request, closing the plugin, destroying the
 * activity or letting the grace period lapse stops it. While the activity is
 * paused the listener is removed and the reader keeps waiting.
 */
internal class SensorStream(
    private val manager: SensorManager?,
    private val sensorType: Int,
    private val handler: Handler,
    private val activity: Activity?,
    private val lingerFloorMs: Long,
) {
    private val ring = SampleRing()
    private var waiter: Waiter? = null
    private var listener: SensorEventListener? = null
    private var lifecycle: Application.ActivityLifecycleCallbacks? = null
    private var linger: Runnable? = null
    private var intervalMs = 0
    private var paused = false

    private class Waiter(
        val continuation: CancellableContinuation<ByteArray>,
        val cursor: Long,
    )

    /** Whether this stream holds the sensor, or would while paused. */
    val active: Boolean get() = listener != null || lifecycle != null

    /** Answers with every sample after [requested], waiting for one if needed. */
    suspend fun watch(
        intervalMs: Int,
        requested: Long,
    ): ByteArray {
        if (waiter != null) throw PluginException(PluginException.BUSY)

        // A cursor this stream never issued would wait forever.
        if (requested < 0 || requested > ring.lastSequence) throw PluginException(PluginException.INVALID_ARGUMENT)

        cancelLinger()
        start(intervalMs)

        // Zero asks for samples from now on, not ones a previous reader left.
        val cursor = if (requested == 0L) ring.lastSequence else requested

        ring.batch(cursor)?.let {
            scheduleLinger()

            return it
        }

        return suspendCancellableCoroutine { continuation ->
            waiter = Waiter(continuation, cursor)
            continuation.invokeOnCancellation {
                handler.post { if (waiter?.continuation === continuation) stop(null) }
            }
        }
    }

    /** Registers the listener, or re-registers it at a new interval. */
    private fun start(intervalMs: Int) {
        val sensorManager = manager ?: throw PluginException(PluginException.UNAVAILABLE)
        val sensor = sensorManager.getDefaultSensor(sensorType) ?: throw PluginException(PluginException.UNAVAILABLE)

        if (lifecycle == null) watchLifecycle()

        if (listener != null && this.intervalMs == intervalMs) return

        this.intervalMs = intervalMs

        if (!paused) register(sensorManager, sensor)
    }

    private fun register(
        sensorManager: SensorManager,
        sensor: Sensor,
    ) {
        unregister()

        val started = SystemClock.elapsedRealtimeNanos()
        val observer =
            object : SensorEventListener {
                override fun onAccuracyChanged(
                    sensor: Sensor?,
                    accuracy: Int,
                ) = Unit

                override fun onSensorChanged(event: SensorEvent) {
                    // A replaced listener, or a value cached from before this
                    // registration, is not part of the stream.
                    if (listener !== this || event.timestamp < started) return

                    receive(event)
                }
            }

        try {
            if (!sensorManager.registerListener(observer, sensor, intervalMs * MICROS_PER_MILLISECOND, handler)) {
                stop(PluginException(PluginException.UNAVAILABLE))

                throw PluginException(PluginException.UNAVAILABLE)
            }
        } catch (error: SecurityException) {
            stop(PluginException(PluginException.PERMISSION_DENIED, error))

            throw PluginException(PluginException.PERMISSION_DENIED, error)
        }

        listener = observer
    }

    private fun unregister() {
        listener?.let { manager?.unregisterListener(it) }
        listener = null
    }

    private fun receive(event: SensorEvent) {
        val bytes =
            try {
                encodeSensorEvent(sensorType, event.timestamp, event.values, false)
            } catch (error: PluginException) {
                stop(error)

                return
            }

        ring.append(bytes)

        val current = waiter ?: return
        val batch = ring.batch(current.cursor) ?: return

        waiter = null
        scheduleLinger()

        if (current.continuation.isActive) current.continuation.resume(batch)
    }

    /** Keeps the sensor running a little while for the reader's next request. */
    private fun scheduleLinger() {
        cancelLinger()

        val task =
            Runnable {
                linger = null

                if (waiter == null) stop(null)
            }

        linger = task
        handler.postDelayed(task, maxOf(lingerFloorMs, intervalMs.toLong() * LINGER_INTERVALS))
    }

    private fun cancelLinger() {
        linger?.let { handler.removeCallbacks(it) }
        linger = null
    }

    /** Pauses with the activity, resumes with it, and ends when it is destroyed. */
    private fun watchLifecycle() {
        val target = activity ?: return
        val callbacks =
            object : Application.ActivityLifecycleCallbacks {
                override fun onActivityPaused(current: Activity) {
                    if (current !== target) return

                    paused = true
                    unregister()
                }

                override fun onActivityResumed(current: Activity) {
                    if (current !== target || !paused) return

                    paused = false

                    val sensorManager = manager ?: return
                    val sensor = sensorManager.getDefaultSensor(sensorType) ?: return

                    try {
                        register(sensorManager, sensor)
                    } catch (error: PluginException) {
                        // register already ended the stream and told the reader.
                    }
                }

                override fun onActivityDestroyed(current: Activity) {
                    if (current === target) stop(PluginException(PluginException.CANCELLED))
                }

                override fun onActivityCreated(
                    current: Activity,
                    state: Bundle?,
                ) = Unit

                override fun onActivityStarted(current: Activity) = Unit

                override fun onActivityStopped(current: Activity) = Unit

                override fun onActivitySaveInstanceState(
                    current: Activity,
                    state: Bundle,
                ) = Unit
            }

        lifecycle = callbacks
        target.application.registerActivityLifecycleCallbacks(callbacks)
    }

    /**
     * Releases the sensor and ends any waiting request: with [error] when
     * given, otherwise as a cancellation.
     */
    fun stop(error: PluginException?) {
        cancelLinger()
        unregister()
        lifecycle?.let { activity?.application?.unregisterActivityLifecycleCallbacks(it) }
        lifecycle = null
        paused = false
        ring.clear()

        val current = waiter ?: return

        waiter = null

        if (!current.continuation.isActive) return

        if (error == null) current.continuation.cancel() else current.continuation.resumeWithException(error)
    }

    private companion object {
        /** SensorManager takes microseconds. */
        const val MICROS_PER_MILLISECOND = 1000

        /** The grace period covers at least this many sampling intervals. */
        const val LINGER_INTERVALS = 2L
    }
}
