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

    @Test
    fun streamsDeliverOrderedBatchesAndStopWhenTheReaderLeaves() =
        runBlocking {
            withContext(Dispatchers.Main) { checkAccelerometerStream() }
        }

    @Test
    fun theCombinedPluginRoutesKindsAndBoundedSensorsDoNotStream() =
        runBlocking {
            withContext(Dispatchers.Main) { checkCombinedPlugin() }
        }

    /** A stream answers after its cursor, refuses a second reader, and stops on cancel or lapse. */
    private suspend fun CoroutineScope.checkAccelerometerStream() {
        val plugin = MotionSensorPlugin(context, "accelerometer", Sensor.TYPE_ACCELEROMETER, lingerFloorMs = SHORT_LINGER_MS)

        try {
            assertEquals(1, plugin.invoke(AVAILABLE, ByteBuffer.allocate(0))[0].toInt())
            expectError(PluginException.INVALID_ARGUMENT) { plugin.invoke(WATCH, request(SAMPLE_INTERVAL_MS)) }
            expectError(PluginException.INVALID_ARGUMENT) { plugin.invoke(WATCH, watchRequest(SAMPLE_INTERVAL_MS, 1)) }

            val first = batch(withTimeout(SAMPLE_TIMEOUT_MS) { plugin.invoke(WATCH, watchRequest(SAMPLE_INTERVAL_MS, 0)) })

            assertTrue(first.cursor >= first.samples.size)
            assertTrue(plugin.streaming)

            // The next request picks up exactly where the last one ended.
            val second = batch(withTimeout(SAMPLE_TIMEOUT_MS) { plugin.invoke(WATCH, watchRequest(SAMPLE_INTERVAL_MS, first.cursor)) })

            assertTrue(second.cursor >= first.cursor + second.samples.size)
            assertTrue(second.samples.first()[0] >= first.samples.last()[0])

            for (sample in first.samples + second.samples) {
                val magnitude = sqrt(sample[1] * sample[1] + sample[2] * sample[2] + sample[3] * sample[3])

                assertTrue("Accelerometer magnitude $magnitude should include gravity in m/s²", magnitude in 8.0..12.0)
            }

            // A cursor the stream never issued is refused rather than waited on.
            expectError(PluginException.INVALID_ARGUMENT) {
                plugin.invoke(WATCH, watchRequest(SAMPLE_INTERVAL_MS, second.cursor + CURSOR_GAP))
            }

            // Cancelling a pending request is how a reader says it is done.
            val waiting = async(start = CoroutineStart.UNDISPATCHED) { plugin.invoke(WATCH, watchRequest(SLOW_INTERVAL_MS, 0)) }

            expectError(PluginException.BUSY) { plugin.invoke(WATCH, watchRequest(SAMPLE_INTERVAL_MS, 0)) }
            waiting.cancelAndJoin()
            drainMainHandler()
            assertTrue(!plugin.streaming)

            // A reader that stops asking lets the grace period lapse.
            withTimeout(SAMPLE_TIMEOUT_MS) { plugin.invoke(WATCH, watchRequest(SAMPLE_INTERVAL_MS, 0)) }
            assertTrue(plugin.streaming)
            kotlinx.coroutines.delay(SHORT_LINGER_MS * LINGER_WAIT_FACTOR)
            assertTrue(!plugin.streaming)
        } finally {
            plugin.close()
        }
    }

    /** Kinds route to their own sensors; light and steps keep their one-shot contract. */
    private suspend fun CoroutineScope.checkCombinedPlugin() {
        val plugin = MultiSensorPlugin(context, "sensors")

        try {
            assertEquals(1, plugin.invoke(AVAILABLE, kind(0))[0].toInt())
            expectError(PluginException.INVALID_ARGUMENT) { plugin.invoke(AVAILABLE, kind(UNKNOWN_KIND)) }
            expectError(PluginException.INVALID_ARGUMENT) { plugin.invoke(AVAILABLE, ByteBuffer.allocate(2)) }

            for (code in MultiSensorPlugin.KIND_SENSOR_TYPES.indices) {
                assertTrue(plugin.invoke(AVAILABLE, kind(code))[0].toInt() in 0..1)
            }

            val reading = withTimeout(SAMPLE_TIMEOUT_MS) { plugin.invoke(NEXT, kind(0, request(SAMPLE_INTERVAL_MS))) }

            assertEquals(VECTOR_REPLY_BYTES, reading.size)

            val stream = async(start = CoroutineStart.UNDISPATCHED) { plugin.invoke(WATCH, kind(0, watchRequest(SAMPLE_INTERVAL_MS, 0))) }
            val reply = batch(withTimeout(SAMPLE_TIMEOUT_MS) { stream.await() })

            assertTrue(reply.samples.isNotEmpty())
            assertTrue(plugin.streaming(0))

            val closing =
                async(start = CoroutineStart.UNDISPATCHED) { plugin.invoke(WATCH, kind(0, watchRequest(SLOW_INTERVAL_MS, reply.cursor))) }

            plugin.close()
            closing.join()
            assertTrue(closing.isCancelled)
            assertTrue(!plugin.streaming(0))
        } finally {
            plugin.close()
        }

        val light = MotionSensorPlugin(context, "light_sensor", Sensor.TYPE_LIGHT, bounded = true)

        try {
            assertTrue(WATCH !in light.methods)
            expectError(PluginException.UNAVAILABLE) { light.invoke(WATCH, watchRequest(SAMPLE_INTERVAL_MS, 0)) }
        } finally {
            light.close()
        }
    }

    /** A decoded watch reply: cursor, dropped count and each sample's doubles. */
    private class Batch(
        val cursor: Long,
        val dropped: Int,
        val samples: List<DoubleArray>,
    )

    private fun batch(bytes: ByteArray): Batch {
        val reply = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        val cursor = reply.long
        val dropped = reply.int
        val count = reply.int

        assertTrue(count in 1..SampleRing.CAPACITY)
        assertEquals(BATCH_HEADER_BYTES + count * VECTOR_REPLY_BYTES, bytes.size)

        return Batch(cursor, dropped, List(count) { DoubleArray(VECTOR_DOUBLES) { reply.double } })
    }

    private fun watchRequest(
        interval: Int,
        cursor: Long,
    ): ByteBuffer =
        ByteBuffer
            .allocate(WATCH_REQUEST_BYTES)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(interval)
            .putLong(cursor)
            .flip() as ByteBuffer

    /** Prefixes a request with the combined plugin's sensor kind. */
    private fun kind(
        code: Int,
        rest: ByteBuffer = ByteBuffer.allocate(0),
    ): ByteBuffer =
        ByteBuffer
            .allocate(KIND_BYTES + rest.remaining())
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(code)
            .put(rest)
            .flip() as ByteBuffer

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

        /** Streams samples after a cursor. */
        const val WATCH = 2

        /** A watch carries the interval and a u64 cursor; replies start with a 16-byte header. */
        const val WATCH_REQUEST_BYTES = 12
        const val BATCH_HEADER_BYTES = 16

        /** Doubles in a vector sample: timestamp, then three axes. */
        const val VECTOR_DOUBLES = 4

        /** The combined plugin's kind prefix, and a kind it does not know. */
        const val KIND_BYTES = 4
        const val UNKNOWN_KIND = 9

        /** A slow interval that keeps a request pending while the test acts on it. */
        const val SLOW_INTERVAL_MS = 60_000

        /** A grace period short enough to watch lapse, and how long to wait for it. */
        const val SHORT_LINGER_MS = 100L

        /** How far past the newest sample an unissued cursor sits. */
        const val CURSOR_GAP = 1_000_000L
        const val LINGER_WAIT_FACTOR = 10L

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
