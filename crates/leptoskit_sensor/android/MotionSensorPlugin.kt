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
import kotlin.math.floor

/** One fresh sensor event per call. Cancellation/close/background unregister the sensor. */
open class MotionSensorPlugin(
    context: Context,
    final override val name: String,
    private val sensorType: Int,
    private val bounded: Boolean = false,
    private val authorize: () -> Unit = {},
) : NativePlugin {
    final override val methods = intArrayOf(AVAILABLE, NEXT)
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

        val sensorManager = manager
        val sensor = sensorManager?.getDefaultSensor(sensorType)

        if (method == AVAILABLE) return availability(payload, sensor)

        val request = readRequest(payload)

        requireVisibleContent()
        authorize()

        return awaitEvent(request, sensorManager, sensor)
    }

    /** Answers whether a default sensor for this type is installed. */
    private fun availability(
        payload: ByteBuffer,
        sensor: Sensor?,
    ): ByteArray {
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        return byteArrayOf(if (sensor == null) 0 else 1, 0, 0, 0)
    }

    /** Reads the interval and, for bounded reads, the deadline that follows it. */
    private fun readRequest(payload: ByteBuffer): Request {
        if (payload.remaining() != expectedBytes()) throw PluginException(PluginException.INVALID_ARGUMENT)

        val interval = readInterval(payload)

        if (!bounded) return Request(interval, NO_DEADLINE)

        return Request(interval, readTimeout(payload))
    }

    /** Bounded requests carry a deadline after the interval. */
    private fun expectedBytes(): Int = if (bounded) BOUNDED_REQUEST_BYTES else REQUEST_BYTES

    private fun readInterval(payload: ByteBuffer): Int {
        val interval = payload.order(ByteOrder.LITTLE_ENDIAN).int

        if (interval !in MIN_INTERVAL_MS..MAX_INTERVAL_MS) throw PluginException(PluginException.INVALID_ARGUMENT)

        return interval
    }

    private fun readTimeout(payload: ByteBuffer): Int {
        val timeout = payload.int

        if (timeout !in MIN_TIMEOUT_MS..MAX_TIMEOUT_MS) throw PluginException(PluginException.INVALID_ARGUMENT)

        return timeout
    }

    /**
     * Bounded reads need visible content, not focus: a dialog over the
     * activity does not stop the app from being in use.
     */
    private fun requireVisibleContent() {
        if (!bounded || activity == null) return

        if (activity.window?.decorView?.isShown != true) throw PluginException(PluginException.UNAVAILABLE)
    }

    /** Registers one listener and resolves on the next fresh event. */
    private suspend fun awaitEvent(
        request: Request,
        sensorManager: SensorManager?,
        sensor: Sensor?,
    ): ByteArray {
        if (waiting != null) throw PluginException(PluginException.BUSY)

        if (sensorManager == null || sensor == null) throw PluginException(PluginException.UNAVAILABLE)

        return suspendCancellableCoroutine { continuation ->
            startWait(continuation, request, sensorManager, sensor)
        }
    }

    /** Ties one continuation to its listener, deadline and lifecycle callbacks. */
    private fun startWait(
        continuation: CancellableContinuation<ByteArray>,
        request: Request,
        sensorManager: SensorManager,
        sensor: Sensor,
    ) {
        waiting = continuation
        val started = SystemClock.elapsedRealtimeNanos()
        val observer = eventListener(continuation, started)

        listener = observer
        continuation.invokeOnCancellation {
            handler.post { if (waiting === continuation) cleanup() }
        }

        if (!continuation.isActive) {
            cleanup()

            return
        }

        if (bounded) scheduleDeadline(continuation, request.timeoutMs)

        listen(continuation, request, sensorManager, sensor, observer)
    }

    /** Starts delivery on the platform handler, translating its refusal modes. */
    private fun listen(
        continuation: CancellableContinuation<ByteArray>,
        request: Request,
        sensorManager: SensorManager,
        sensor: Sensor,
        observer: SensorEventListener,
    ) {
        try {
            val intervalMicros = request.intervalMs * MICROS_PER_MILLISECOND

            if (!sensorManager.registerListener(observer, sensor, intervalMicros, handler)) {
                fail(continuation, PluginException(PluginException.UNAVAILABLE))

                return
            }

            watchLifecycle(continuation)
        } catch (error: SecurityException) {
            fail(continuation, PluginException(PluginException.PERMISSION_DENIED, error))
        } catch (error: RuntimeException) {
            fail(continuation, PluginException(PluginException.NATIVE, error))
        }
    }

    /** Ignores events that belong to another wait or predate this one. */
    private fun eventListener(
        continuation: CancellableContinuation<ByteArray>,
        started: Long,
    ): SensorEventListener =
        object : SensorEventListener {
            override fun onAccuracyChanged(
                sensor: Sensor?,
                accuracy: Int,
            ) = Unit

            override fun onSensorChanged(event: SensorEvent) {
                if (waiting !== continuation || !continuation.isActive || event.timestamp < started) return

                onFreshEvent(continuation, event)
            }
        }

    /** Encodes the sample, or fails the wait when it cannot be trusted. */
    private fun onFreshEvent(
        continuation: CancellableContinuation<ByteArray>,
        event: SensorEvent,
    ) {
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

    /** Cancels the wait when the activity leaves the foreground. */
    private fun watchLifecycle(continuation: CancellableContinuation<ByteArray>) {
        val target = activity ?: return
        val callbacks = LifecycleWatcher(continuation, target)

        lifecycle = callbacks
        target.application.registerActivityLifecycleCallbacks(callbacks)
    }

    /** Cancels the wait when the activity it was started for pauses or is destroyed. */
    private inner class LifecycleWatcher(
        private val continuation: CancellableContinuation<ByteArray>,
        private val target: Activity,
    ) : Application.ActivityLifecycleCallbacks {
        override fun onActivityPaused(current: Activity) {
            if (current !== target || waiting !== continuation) return

            cleanup()
            continuation.cancel()
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

    /** Answers a bounded read with the timeout marker instead of waiting forever. */
    private fun scheduleDeadline(
        continuation: CancellableContinuation<ByteArray>,
        timeoutMs: Int,
    ) {
        val task =
            Runnable {
                if (waiting !== continuation) return@Runnable

                cleanup()

                if (continuation.isActive) continuation.resume(byteArrayOf(1, 0, 0, 0))
            }

        deadline = task
        handler.postDelayed(task, timeoutMs.toLong())
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

    /** One validated request: the interval, plus a deadline for bounded reads. */
    private data class Request(
        val intervalMs: Int,
        val timeoutMs: Int,
    )

    private companion object {
        /** Answers whether a default sensor for this type is installed. */
        const val AVAILABLE = 0

        /** Blocks for one fresh sample. */
        const val NEXT = 1

        /** A request carries the interval, plus a deadline when bounded. */
        const val REQUEST_BYTES = 4
        const val BOUNDED_REQUEST_BYTES = 8

        /** Accepted sampling interval, in milliseconds. */
        const val MIN_INTERVAL_MS = 5
        const val MAX_INTERVAL_MS = 60_000

        /** Accepted deadline, in milliseconds. */
        const val MIN_TIMEOUT_MS = 1
        const val MAX_TIMEOUT_MS = 120_000

        /** An unbounded read has no deadline to report. */
        const val NO_DEADLINE = 0

        /** SensorManager takes microseconds. */
        const val MICROS_PER_MILLISECOND = 1000
    }
}

/** Fixed, validated wire encoding; boot counters are not daily totals. */
internal fun encodeSensorEvent(
    type: Int,
    timestamp: Long,
    values: FloatArray,
    bounded: Boolean,
): ByteArray {
    val count = sampleCount(type)

    requireEncodable(type, timestamp, values, count)

    val bytes = allocate(type, count, bounded)

    if (type == Sensor.TYPE_STEP_COUNTER) {
        putStepCounter(bytes, values[0], timestamp)
    } else {
        putVector(bytes, timestamp, values, count)
    }

    return bytes.array()
}

/** Allocates the exact body, plus the status word a bounded read carries. */
private fun allocate(
    type: Int,
    count: Int,
    bounded: Boolean,
): ByteBuffer {
    val body = if (type == Sensor.TYPE_STEP_COUNTER) STEP_COUNTER_BYTES else (count + 1) * DOUBLE_BYTES
    val bytes = ByteBuffer.allocate(body + if (bounded) STATUS_BYTES else 0).order(ByteOrder.LITTLE_ENDIAN)

    if (bounded) bytes.putInt(SAMPLE_FOLLOWS)

    return bytes
}

/** Timestamp in seconds, then one double per reported component. */
private fun putVector(
    bytes: ByteBuffer,
    timestamp: Long,
    values: FloatArray,
    count: Int,
) {
    bytes.putDouble(seconds(timestamp))
    repeat(count) { bytes.putDouble(values[it].toDouble()) }
}

/** Pressure, ambient light and the step counter report a single component. */
private fun sampleCount(type: Int): Int =
    when (type) {
        Sensor.TYPE_PRESSURE, Sensor.TYPE_LIGHT, Sensor.TYPE_STEP_COUNTER -> 1
        else -> MOTION_AXES
    }

/** Rejects clocks, component counts and ranges the wire cannot represent. */
private fun requireEncodable(
    type: Int,
    timestamp: Long,
    values: FloatArray,
    count: Int,
) {
    if (timestamp < 0 || values.size < count || (0 until count).any { !values[it].isFinite() }) {
        throw PluginException(PluginException.NATIVE)
    }

    if (type == Sensor.TYPE_PRESSURE && values[0] <= 0) throw PluginException(PluginException.NATIVE)

    if ((type == Sensor.TYPE_LIGHT || type == Sensor.TYPE_STEP_COUNTER) && values[0] < 0) {
        throw PluginException(PluginException.NATIVE)
    }
}

/** Step counters report a whole count since boot, plus the sample time. */
private fun putStepCounter(
    bytes: ByteBuffer,
    value: Float,
    timestamp: Long,
) {
    val steps = value.toDouble()

    if (steps >= Long.MAX_VALUE.toDouble() || steps != floor(steps)) throw PluginException(PluginException.NATIVE)

    // Scope 1 marks the cumulative boot counter the pedometer expects; the
    // word after it is reserved and must stay zero.
    bytes
        .putInt(SCOPE_BOOT_COUNTER)
        .putInt(0)
        .putLong(steps.toLong())
        .putDouble(seconds(timestamp))
        .putDouble(0.0)
}

/** Platform clocks arrive in nanoseconds; the wire carries seconds. */
private fun seconds(nanos: Long): Double = nanos.toDouble() / NANOS_PER_SECOND

/** Android delivers sample timestamps in nanoseconds. */
private const val NANOS_PER_SECOND = 1_000_000_000.0

/** Components a motion sensor reports on each axis. */
private const val MOTION_AXES = 3

/** Bytes in a status code and in one f64. */
private const val STATUS_BYTES = 4
private const val DOUBLE_BYTES = 8

/** Byte size of a step-counter sample: scope, reserved, count and two doubles. */
private const val STEP_COUNTER_BYTES = 32

/** The status that announces a sample rather than a deadline. */
private const val SAMPLE_FOLLOWS = 0

/** The scope the pedometer reads as a cumulative counter since boot. */
private const val SCOPE_BOOT_COUNTER = 1
