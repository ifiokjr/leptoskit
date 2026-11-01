package dev.leptoskit.plugins.location

import android.location.Location
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.leptoskit.bridge.PluginException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Synthetic inputs only: these tests never request permission or access user location. */
@RunWith(AndroidJUnit4::class)
class LocationPluginTest {
    @Test
    fun queuedLocationsCannotCrossPermissionOrPrecisionRevocation() {
        assertTrue(permitsLocationDelivery(true, true, true))
        assertTrue(permitsLocationDelivery(false, true, false))
        assertTrue(permitsLocationDelivery(false, true, true))
        assertTrue(!permitsLocationDelivery(true, true, false))
        assertTrue(!permitsLocationDelivery(false, false, false))
        assertTrue(!permitsLocationDelivery(true, false, true))
    }

    @Test
    fun freshnessIgnoresWallClockCorrectionsButRespectsExplicitCursor() {
        val options = LocationOptions(false, 1000, 1000, null)
        val location =
            Location("test").apply {
                time = 1 // Wall clock moved backwards; measurement is still fresh.
                elapsedRealtimeNanos = 1_900_000_000
            }
        assertTrue(acceptsLocation(location, options, 1_800_000_000, 2_000_000_000))
        assertTrue(!acceptsLocation(location, options.copy(after = 2.0), 1_800_000_000, 2_000_000_000))
        location.time = Long.MAX_VALUE
        location.elapsedRealtimeNanos = STALE_NANOS
        assertTrue(!acceptsLocation(location, options, 1_800_000_000, 2_000_000_000))

        location.elapsedRealtimeNanos = 1_700_000_000
        assertTrue(!acceptsLocation(location, options.copy(maximumAge = 0), 1_800_000_000, 2_000_000_000))
    }

    @Test
    fun syntheticCoordinatesPreserveUnknownFieldsAndUnits() {
        val location =
            Location("test").apply {
                time = 1_800_000_000_000
                latitude = 37.0
                longitude = -122.0
                accuracy = 5f
            }
        val bytes = encodeLocation(location)
        assertEquals(84, bytes.size)
        val input = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        assertEquals(0, input.int)
        assertEquals(0, input.int) // Optional values are absent, not fabricated zeros.
        assertEquals(0, input.int) // Unknown provider remains unknown.

        input.position(DOUBLES_OFFSET)
        assertEquals(location.time.toDouble(), input.double, 0.0)
        assertEquals(37.0, input.double, 0.0)
        assertEquals(-122.0, input.double, 0.0)
        assertEquals(5.0, input.double, 0.0)
        location.altitude = 0.0
        location.speed = 0f
        val withZero = ByteBuffer.wrap(encodeLocation(location)).order(ByteOrder.LITTLE_ENDIAN)

        assertEquals(9, withZero.getInt(4))
        location.latitude = MAX_LATITUDE + 1.0
        expectInvalid(PluginException.NATIVE) { encodeLocation(location) }
    }

    /** Rejects bounds the Rust crate would refuse, including a non-finite cursor. */
    @Test
    fun malformedBoundsAndPreviousTimestampsAreRejected() {
        fun payload(
            timeout: Int,
            previous: Double? = null,
        ): ByteBuffer {
            val bytes =
                ByteBuffer
                    .allocate(if (previous == null) BOUNDS_BYTES else PREVIOUS_BYTES)
                    .order(ByteOrder.LITTLE_ENDIAN)
                    .putInt(1)
                    .putInt(timeout)
                    .putInt(0)

            if (previous != null) bytes.putDouble(previous)
            bytes.rewind()

            return bytes
        }
        val valid = LocationOptions.decode(payload(1000), false)
        assertTrue(valid.highAccuracy)
        assertEquals(1000, valid.timeout)

        for (timeout in intArrayOf(0, MAX_TIMEOUT_MS + 1)) {
            expectInvalid(PluginException.INVALID_ARGUMENT) { LocationOptions.decode(payload(timeout), false) }
        }

        expectInvalid(PluginException.INVALID_ARGUMENT) { LocationOptions.decode(payload(1000, Double.NaN), true) }
        expectInvalid(PluginException.INVALID_ARGUMENT) { LocationOptions.decode(payload(1000), true) }
    }

    private companion object {
        /** Three bounds fields, or those plus one timestamp. */
        const val BOUNDS_BYTES = 12
        const val PREVIOUS_BYTES = 20

        /** Where the eight doubles start: five u32 fields precede them. */
        const val DOUBLES_OFFSET = 20

        /** A measurement older than the caller's request started. */
        const val STALE_NANOS = 900_000_000L

        /** Values the decoder must reject. */
        const val MAX_TIMEOUT_MS = 120_000
        const val MAX_LATITUDE = 90.0
    }

    private fun expectInvalid(
        code: Int,
        action: () -> Any,
    ) {
        try {
            action()
            throw AssertionError("Expected native failure $code")
        } catch (
            error: PluginException,
        ) {
            assertEquals(code, error.code)
        }
    }
}
