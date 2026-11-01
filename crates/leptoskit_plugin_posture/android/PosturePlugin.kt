package dev.leptoskit.plugins.posture

import android.app.Activity
import android.content.Context
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import androidx.window.layout.FoldingFeature
import androidx.window.layout.WindowInfoTracker
import androidx.window.layout.WindowLayoutInfo
import androidx.window.layout.WindowMetricsCalculator
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.onStart
import kotlinx.coroutines.withTimeoutOrNull
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.math.roundToInt

/**
 * Fold state from Jetpack WindowManager's folding features, which say where each
 * hinge crosses the activity's window, and the hinge angle sensor, which says how
 * far the device is open even when no hinge crosses the window (folded onto the
 * cover screen, or in split screen).
 */
class PosturePlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "posture"
    override val methods = intArrayOf(0, 1)
    private val sensors = context.getSystemService(Context.SENSOR_SERVICE) as? SensorManager
    private val hingeSensor = sensors?.getDefaultSensor(Sensor.TYPE_HINGE_ANGLE)
    private var lastAngle: Int? = null
    private var waiting = false
    private var closed = false

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)
        val activity = context as? Activity ?: throw PluginException(PluginException.UNAVAILABLE)

        if (activity.isDestroyed) throw PluginException(PluginException.UNAVAILABLE)
        val layouts = WindowInfoTracker.getOrCreate(activity).windowLayoutInfo(activity)

        if (method == 0) {
            if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
            val angle = lastAngle ?: withTimeoutOrNull(FIRST_ANGLE_TIMEOUT_MS) { angles().first() }

            return encode(activity, layouts.first(), angle)
        }

        val previous = previous(payload)

        if (waiting) throw PluginException(PluginException.BUSY)
        waiting = true
        try {
            // The first layout is the current one, so a change between the guest's
            // last snapshot and this call is reported immediately.
            return combine(layouts, angles().onStart { emit(lastAngle) }) { layout, angle ->
                encode(activity, layout, angle)
            }.first { !it.contentEquals(previous) }
        } finally {
            waiting = false
        }
    }

    override fun close() {
        closed = true
    }

    /** Whole-degree hinge angles, or a single null on devices without the sensor. */
    private fun angles(): Flow<Int?> {
        val sensor = hingeSensor ?: return flowOf(null)
        val manager = sensors ?: return flowOf(null)
        return callbackFlow {
            val listener =
                object : SensorEventListener {
                    override fun onSensorChanged(event: SensorEvent) {
                        val angle = event.values[0].roundToInt().coerceIn(0, 360)
                        lastAngle = angle
                        trySend(angle)
                    }

                    override fun onAccuracyChanged(
                        sensor: Sensor,
                        accuracy: Int,
                    ) {}
                }

            if (!manager.registerListener(listener, sensor, SensorManager.SENSOR_DELAY_UI)) {
                close(PluginException(PluginException.UNAVAILABLE))
            }

            awaitClose { manager.unregisterListener(listener) }
        }.distinctUntilChanged()
    }

    private fun previous(payload: ByteBuffer): ByteArray {
        val length = payload.remaining()
        val hinges = (length - HEADER_BYTES) / HINGE_BYTES

        if (length < HEADER_BYTES || (length - HEADER_BYTES) % HINGE_BYTES != 0 || hinges > MAX_HINGES) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        val bytes = ByteArray(length)
        payload.get(bytes)

        return bytes
    }

    private fun encode(
        activity: Activity,
        layout: WindowLayoutInfo,
        angle: Int?,
    ): ByteArray {
        val window = WindowMetricsCalculator.getOrCreate().computeCurrentWindowMetrics(activity).bounds
        val folds = layout.displayFeatures.filterIsInstance<FoldingFeature>().take(MAX_HINGES)
        val fold =
            when {
                folds.any { it.state == FoldingFeature.State.HALF_OPENED } -> HALF_OPEN
                folds.isNotEmpty() -> OPEN
                angle == null -> UNKNOWN
                angle < CLOSED_BELOW_DEGREES -> CLOSED
                angle < OPEN_FROM_DEGREES -> HALF_OPEN
                else -> OPEN
            }
        val foldable = hingeSensor != null || folds.isNotEmpty()
        val output = ByteBuffer.allocate(HEADER_BYTES + HINGE_BYTES * folds.size).order(ByteOrder.LITTLE_ENDIAN)

        output.putInt(if (foldable) TRUE else UNKNOWN_FLAG)
        output.putInt(fold)
        output.putInt(folds.size)
        val width = window.width().toFloat()
        val height = window.height().toFloat()

        for (feature in folds) {
            // Folding feature bounds are already in the window's coordinate space.
            val bounds = feature.bounds
            val x = (bounds.left / width).coerceIn(0f, 1f)

            val y = (bounds.top / height).coerceIn(0f, 1f)

            output.putInt(if (feature.state == FoldingFeature.State.HALF_OPENED) HALF_OPEN else OPEN)

            output.putInt(if (feature.orientation == FoldingFeature.Orientation.VERTICAL) VERTICAL else HORIZONTAL)

            output.putInt(if (feature.occlusionType == FoldingFeature.OcclusionType.FULL) TRUE else FALSE)
            output.putInt(1)
            output.putFloat(x)
            output.putFloat(y)
            output.putFloat((bounds.width() / width).coerceIn(0f, 1f - x))
            output.putFloat((bounds.height() / height).coerceIn(0f, 1f - y))
            // A single sensor cannot say which of several hinges it measures.
            output.putInt(if (folds.size == 1 && angle != null) angle else -1)
        }

        return output.array()
    }

    internal companion object {
        const val MAX_HINGES = 4
        const val HEADER_BYTES = 12
        const val HINGE_BYTES = 36
        const val FIRST_ANGLE_TIMEOUT_MS = 250L

        // The emulator's and AOSP's default posture boundaries.
        const val CLOSED_BELOW_DEGREES = 30
        const val OPEN_FROM_DEGREES = 150

        const val UNKNOWN = 0
        const val CLOSED = 1
        const val HALF_OPEN = 2
        const val OPEN = 3
        const val VERTICAL = 1
        const val HORIZONTAL = 2
        const val UNKNOWN_FLAG = 0
        const val FALSE = 1
        const val TRUE = 2
    }
}
