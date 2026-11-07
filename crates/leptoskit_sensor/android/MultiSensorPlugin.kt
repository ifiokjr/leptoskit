package dev.leptoskit.sensors

import android.content.Context
import android.hardware.Sensor
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * The combined `sensors` plugin. Every request starts with a little-endian u32
 * sensor kind, followed by the request a single-kind [MotionSensorPlugin]
 * takes, and each kind is served by its own one.
 */
open class MultiSensorPlugin(
    context: Context,
    final override val name: String,
    lingerFloorMs: Long = STREAM_LINGER_MS,
) : NativePlugin {
    final override val methods = intArrayOf(AVAILABLE, NEXT, WATCH)

    /** One plugin per kind, in the kind's wire order. */
    private val channels = KIND_SENSOR_TYPES.map { MotionSensorPlugin(context, name, it, lingerFloorMs = lingerFloorMs) }

    /** Whether the stream of a kind holds its sensor, for tests. */
    fun streaming(kind: Int): Boolean = channels[kind].streaming

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (payload.remaining() < KIND_BYTES) throw PluginException(PluginException.INVALID_ARGUMENT)

        val kind = payload.order(ByteOrder.LITTLE_ENDIAN).int
        val channel = channels.getOrNull(kind) ?: throw PluginException(PluginException.INVALID_ARGUMENT)

        return channel.invoke(method, payload.slice().order(ByteOrder.LITTLE_ENDIAN))
    }

    override fun close() = channels.forEach { it.close() }

    companion object {
        /** Answers availability, one fresh sample, and a stream batch. */
        private const val AVAILABLE = 0
        private const val NEXT = 1
        private const val WATCH = 2

        /** The kind that prefixes every request. */
        private const val KIND_BYTES = 4

        /** Android sensor types, indexed by wire kind. */
        val KIND_SENSOR_TYPES =
            listOf(
                Sensor.TYPE_ACCELEROMETER,
                Sensor.TYPE_LINEAR_ACCELERATION,
                Sensor.TYPE_GYROSCOPE,
                Sensor.TYPE_MAGNETIC_FIELD,
                Sensor.TYPE_PRESSURE,
            )
    }
}
