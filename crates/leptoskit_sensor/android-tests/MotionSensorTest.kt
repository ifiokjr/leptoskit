package dev.leptoskit.sensors

import android.content.Context
import android.hardware.Sensor
import android.os.Handler
import android.os.Looper
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.coroutines.resume
import kotlin.math.sqrt

/** Exercises real Android SensorManager registration on an emulator or physical device. */
@RunWith(AndroidJUnit4::class)
class MotionSensorTest {
    private val context = ApplicationProvider.getApplicationContext<Context>()

    @Test
    fun availabilityAndMalformedPayloadsUseTheNativeContract() =
        runBlocking {
            withContext(Dispatchers.Main) { checkAvailabilityAndMalformedPayloads() }
        }

    @Test
    fun accelerometerCancellationReleasesRegistrationAndReadingUsesSiUnits() =
        runBlocking {
            withContext(Dispatchers.Main) { checkAccelerometerReading() }
        }

    @Test
    fun boundedLightWaitReleasesItsSlotOnDeadlineCancellationAndClose() =
        runBlocking {
            withContext(Dispatchers.Main) { checkBoundedLightWait() }
        }

    /** Every probed sensor type answers availability and rejects bad arguments. */
    private suspend fun checkAvailabilityAndMalformedPayloads() {
        for (type in PROBED_TYPES) {
            val plugin = MotionSensorPlugin(context, "test", type)

            try {
                val available = plugin.invoke(AVAILABLE, ByteBuffer.allocate(0))

                assertEquals(BOOLEAN_REPLY_BYTES, available.size)
                assertTrue(available[0].toInt() in 0..1)

                expectError(PluginException.INVALID_ARGUMENT) { plugin.invoke(AVAILABLE, request(SAMPLE_INTERVAL_MS)) }
                expectError(PluginException.INVALID_ARGUMENT) { plugin.invoke(NEXT, request(MIN_INTERVAL_MS - 1)) }
                expectError(PluginException.INVALID_ARGUMENT) { plugin.invoke(NEXT, request(MAX_INTERVAL_MS + 1)) }
                expectError(PluginException.UNAVAILABLE) { plugin.invoke(UNKNOWN_METHOD, request(SAMPLE_INTERVAL_MS)) }
            } finally {
                plugin.close()
            }

            expectError(PluginException.CANCELLED) { plugin.invoke(AVAILABLE, ByteBuffer.allocate(0)) }
        }
    }

    /** An accelerator sample is gravity in m/s² and cancellation frees the slot. */
    private suspend fun CoroutineScope.checkAccelerometerReading() {
        val plugin = MotionSensorPlugin(context, "accelerometer", Sensor.TYPE_ACCELEROMETER)

        try {
            // The Android CI AVD provides an accelerometer; this assertion must not silently skip.
            assertEquals(1, plugin.invoke(AVAILABLE, ByteBuffer.allocate(0))[0].toInt())

            val waiting = startRead(plugin)

            expectError(PluginException.BUSY) { plugin.invoke(NEXT, request(SAMPLE_INTERVAL_MS)) }
            waiting.cancelAndJoin()
            drainMainHandler()

            val bytes = withTimeout(SAMPLE_TIMEOUT_MS) { plugin.invoke(NEXT, request(SAMPLE_INTERVAL_MS)) }

            assertEquals(VECTOR_REPLY_BYTES, bytes.size)

            val reading = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)

            assertTrue(reading.double >= 0.0)

            val x = reading.double
            val y = reading.double
            val z = reading.double

            assertTrue(x.isFinite() && y.isFinite() && z.isFinite())

            // At rest the emulator reports gravity (~9.8 m/s²), not Apple's unit-g scale.
            val magnitude = sqrt(x * x + y * y + z * z)

            assertTrue("Accelerometer magnitude $magnitude should include gravity in m/s²", magnitude in 8.0..12.0)

            val closing = startRead(plugin)

            plugin.close()
            closing.join()
            assertTrue(closing.isCancelled)
        } finally {
            plugin.close()
        }
    }

    /** A bounded light read answers with a sample or its deadline, then frees the slot. */
    private suspend fun CoroutineScope.checkBoundedLightWait() {
        val plugin = MotionSensorPlugin(context, "light_sensor", Sensor.TYPE_LIGHT, bounded = true)

        try {
            if (plugin.invoke(AVAILABLE, ByteBuffer.allocate(0))[0].toInt() == 0) {
                expectError(PluginException.UNAVAILABLE) { plugin.invoke(NEXT, boundedRequest()) }

                return
            }

            val waiting = startBoundedRead(plugin)

            expectError(PluginException.BUSY) { plugin.invoke(NEXT, boundedRequest()) }

            val reply = withTimeout(DEADLINE_TIMEOUT_MS) { waiting.await() }

            assertTrue(reply.contentEquals(TIMEOUT_REPLY) || reply.size == BOUNDED_LIGHT_REPLY_BYTES)

            val cancelled = startBoundedRead(plugin)

            cancelled.cancelAndJoin()
            drainMainHandler()

            val replacement = startBoundedRead(plugin)

            plugin.close()
            replacement.join()
            assertTrue(replacement.isCancelled)
        } finally {
            plugin.close()
        }
    }

    /** Starts a read that stays pending, so the next call observes the busy slot. */
    private fun CoroutineScope.startRead(plugin: MotionSensorPlugin): Deferred<ByteArray> =
        async(start = CoroutineStart.UNDISPATCHED) { plugin.invoke(NEXT, request(SAMPLE_INTERVAL_MS)) }

    /** Starts a bounded read that stays pending until its sample or deadline. */
    private fun CoroutineScope.startBoundedRead(plugin: MotionSensorPlugin): Deferred<ByteArray> =
        async(start = CoroutineStart.UNDISPATCHED) { plugin.invoke(NEXT, boundedRequest()) }

    /** Drains the cancellation cleanup the plugin posts to the main Handler. */
    private suspend fun drainMainHandler() {
        suspendCancellableCoroutine<Unit> { continuation ->
            Handler(Looper.getMainLooper()).post { resumeIfActive(continuation) }
        }
    }

    /** Answers the drain once the posted cleanup has run. */
    private fun resumeIfActive(continuation: CancellableContinuation<Unit>) {
        if (continuation.isActive) continuation.resume(Unit)
    }

    private fun request(interval: Int): ByteBuffer =
        ByteBuffer
            .allocate(REQUEST_BYTES)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(interval)
            .flip() as ByteBuffer

    private fun boundedRequest(): ByteBuffer =
        ByteBuffer
            .allocate(BOUNDED_REQUEST_BYTES)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(BOUNDED_INTERVAL_MS)
            .putInt(BOUNDED_TIMEOUT_MS)
            .flip() as ByteBuffer

    private suspend fun expectError(
        code: Int,
        action: suspend () -> ByteArray,
    ) {
        try {
            action()

            throw AssertionError("Expected native error $code")
        } catch (error: PluginException) {
            assertEquals(code, error.code)
        }
    }

    private companion object {
        /** Method ids the plugin exposes. */
        const val AVAILABLE = 0
        const val NEXT = 1

        /** A method the plugin never registered. */
        const val UNKNOWN_METHOD = 99

        /** Sensors the availability probe covers. */
        val PROBED_TYPES =
            intArrayOf(
                Sensor.TYPE_ACCELEROMETER,
                Sensor.TYPE_GYROSCOPE,
                Sensor.TYPE_MAGNETIC_FIELD,
                Sensor.TYPE_PRESSURE,
            )

        /** A request carries the interval, plus a deadline when bounded. */
        const val REQUEST_BYTES = 4
        const val BOUNDED_REQUEST_BYTES = 8

        /** The interval and deadline the bounded read asks for. */
        const val BOUNDED_INTERVAL_MS = 20
        const val BOUNDED_TIMEOUT_MS = 25

        /** Interval bounds the plugin enforces, in milliseconds. */
        const val MIN_INTERVAL_MS = 5
        const val MAX_INTERVAL_MS = 60_000

        /** A sampling interval that keeps the emulator responsive. */
        const val SAMPLE_INTERVAL_MS = 16

        /** A reply is either one u32 or a twelve-second-capable sample. */
        const val BOOLEAN_REPLY_BYTES = 4
        const val VECTOR_REPLY_BYTES = 32
        const val BOUNDED_LIGHT_REPLY_BYTES = 20

        /** The bounded reply that says no sample arrived before the deadline. */
        val TIMEOUT_REPLY = byteArrayOf(1, 0, 0, 0)

        /** How long a real device may take to deliver a sample. */
        const val SAMPLE_TIMEOUT_MS = 10_000L

        /** How long a bounded read may spend before its own deadline answers. */
        const val DEADLINE_TIMEOUT_MS = 5_000L
    }
}
