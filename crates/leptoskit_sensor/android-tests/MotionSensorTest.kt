package dev.leptoskit.sensors

import android.content.Context
import android.hardware.Sensor
import android.os.Handler
import android.os.Looper
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CoroutineStart
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

/** Exercises real Android SensorManager registration on an emulator or physical device. */
@RunWith(AndroidJUnit4::class)
class MotionSensorTest {
    private val context = ApplicationProvider.getApplicationContext<Context>()

    @Test
    fun availabilityAndMalformedPayloadsUseTheNativeContract() =
        runBlocking {
            withContext(Dispatchers.Main) {
                for (type in intArrayOf(
                    Sensor.TYPE_ACCELEROMETER,
                    Sensor.TYPE_GYROSCOPE,
                    Sensor.TYPE_MAGNETIC_FIELD,
                    Sensor.TYPE_PRESSURE,
                )) {
                    val plugin = MotionSensorPlugin(context, "test", type)
                    try {
                        val available = plugin.invoke(0, ByteBuffer.allocate(0))
                        assertEquals(4, available.size)
                        assertTrue(available[0].toInt() in 0..1)
                        expectError(PluginException.INVALID_ARGUMENT) { plugin.invoke(0, request(16)) }
                        expectError(PluginException.INVALID_ARGUMENT) { plugin.invoke(1, request(4)) }
                        expectError(PluginException.INVALID_ARGUMENT) { plugin.invoke(1, request(60_001)) }
                        expectError(PluginException.UNAVAILABLE) { plugin.invoke(99, request(16)) }
                    } finally {
                        plugin.close()
                    }
                    expectError(PluginException.CANCELLED) { plugin.invoke(0, ByteBuffer.allocate(0)) }
                }
            }
        }

    @Test
    fun accelerometerCancellationReleasesRegistrationAndReadingUsesSiUnits() =
        runBlocking {
            withContext(Dispatchers.Main) {
                val plugin = MotionSensorPlugin(context, "accelerometer", Sensor.TYPE_ACCELEROMETER)
                try {
                    // The Android CI AVD provides an accelerometer; this assertion must not silently skip.
                    assertEquals(1, plugin.invoke(0, ByteBuffer.allocate(0))[0].toInt())
                    val waiting = async(start = CoroutineStart.UNDISPATCHED) { plugin.invoke(1, request(16)) }
                    expectError(PluginException.BUSY) { plugin.invoke(1, request(16)) }
                    waiting.cancelAndJoin()
                    // Drain the cancellation cleanup posted to the main Handler.
                    suspendCancellableCoroutine<Unit> { continuation ->
                        Handler(Looper.getMainLooper()).post { if (continuation.isActive) continuation.resume(Unit) }
                    }
                    val bytes = withTimeout(10_000) { plugin.invoke(1, request(16)) }
                    assertEquals(32, bytes.size)
                    val reading = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
                    assertTrue(reading.double >= 0.0)
                    val x = reading.double
                    val y = reading.double
                    val z = reading.double
                    assertTrue(x.isFinite() && y.isFinite() && z.isFinite())
                    // At rest the emulator reports gravity (~9.8 m/s²), not Apple's unit-g scale.
                    val magnitude = kotlin.math.sqrt(x * x + y * y + z * z)
                    assertTrue("Accelerometer magnitude $magnitude should include gravity in m/s²", magnitude in 8.0..12.0)
                    val closing = async(start = CoroutineStart.UNDISPATCHED) { plugin.invoke(1, request(16)) }
                    plugin.close()
                    closing.join()
                    assertTrue(closing.isCancelled)
                } finally {
                    plugin.close()
                }
            }
        }

    @Test
    fun boundedLightWaitReleasesItsSlotOnDeadlineCancellationAndClose() =
        runBlocking {
            withContext(Dispatchers.Main) {
                val plugin = MotionSensorPlugin(context, "light_sensor", Sensor.TYPE_LIGHT, bounded = true)

                fun boundedRequest() =
                    ByteBuffer
                        .allocate(8)
                        .order(ByteOrder.LITTLE_ENDIAN)
                        .putInt(20)
                        .putInt(25)
                        .flip() as ByteBuffer
                try {
                    if (plugin.invoke(0, ByteBuffer.allocate(0))[0].toInt() == 0) {
                        expectError(PluginException.UNAVAILABLE) { plugin.invoke(1, boundedRequest()) }
                        return@withContext
                    }
                    val waiting = async(start = CoroutineStart.UNDISPATCHED) { plugin.invoke(1, boundedRequest()) }
                    expectError(PluginException.BUSY) { plugin.invoke(1, boundedRequest()) }
                    val reply = withTimeout(5_000) { waiting.await() }
                    assertTrue(reply.contentEquals(byteArrayOf(1, 0, 0, 0)) || reply.size == 20)
                    val cancelled = async(start = CoroutineStart.UNDISPATCHED) { plugin.invoke(1, boundedRequest()) }
                    cancelled.cancelAndJoin()
                    suspendCancellableCoroutine<Unit> { continuation ->
                        Handler(Looper.getMainLooper()).post { if (continuation.isActive) continuation.resume(Unit) }
                    }
                    val replacement = async(start = CoroutineStart.UNDISPATCHED) { plugin.invoke(1, boundedRequest()) }
                    plugin.close()
                    replacement.join()
                    assertTrue(replacement.isCancelled)
                } finally {
                    plugin.close()
                }
            }
        }

    private fun request(interval: Int): ByteBuffer =
        ByteBuffer
            .allocate(4)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(interval)
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
}
