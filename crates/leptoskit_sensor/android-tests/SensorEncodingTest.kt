package dev.leptoskit.sensors

import android.hardware.Sensor
import dev.leptoskit.bridge.PluginException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Synthetic protocol tests; no activity, permissions, hardware or user data. */
class SensorEncodingTest {
    @Test
    fun darknessAndCounterScopesAreNotInventedTotals() {
        val light =
            ByteBuffer
                .wrap(
                    encodeSensorEvent(Sensor.TYPE_LIGHT, 2_500_000_000, floatArrayOf(0f), true),
                ).order(ByteOrder.LITTLE_ENDIAN)
        assertEquals(20, light.remaining())
        assertEquals(0, light.int)
        assertEquals(2.5, light.double, 0.0)
        assertEquals(0.0, light.double, 0.0)
        val steps =
            ByteBuffer
                .wrap(
                    encodeSensorEvent(Sensor.TYPE_STEP_COUNTER, 2_500_000_000, floatArrayOf(13f), true),
                ).order(ByteOrder.LITTLE_ENDIAN)
        assertEquals(36, steps.remaining())
        assertEquals(0, steps.int)
        assertEquals(1, steps.int)
        assertEquals(0, steps.int)
        assertEquals(13L, steps.long)

        assertEquals(2.5, steps.double, 0.0)
        assertEquals(0.0, steps.double, 0.0)
    }

    @Test
    fun invalidValuesAreErrorsAndLegacyWireStaysUnchanged() {
        for (value in floatArrayOf(-1f, Float.NaN, Float.POSITIVE_INFINITY, 1.5f)) {
            assertThrows(PluginException::class.java) { encodeSensorEvent(Sensor.TYPE_STEP_COUNTER, 1, floatArrayOf(value), true) }
        }

        assertThrows(PluginException::class.java) { encodeSensorEvent(Sensor.TYPE_LIGHT, 1, floatArrayOf(-1f), true) }
        val vector = encodeSensorEvent(Sensor.TYPE_ACCELEROMETER, 1_000_000_000, floatArrayOf(-9.8f, 0f, 2f), false)
        assertEquals(32, vector.size)
        assertEquals(1.0, ByteBuffer.wrap(vector).order(ByteOrder.LITTLE_ENDIAN).double, 0.0)
    }
}
