package dev.leptoskit.mobile

import android.database.ContentObserver
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.view.Choreographer
import android.view.View

/** One UI-thread display subscription per host; idle/background players have none. */
internal class NativeRiveAnimator(
    private val root: View,
    private val hasScenes: () -> Boolean,
    private val tick: (Double, Float, Int) -> Boolean,
    private val fail: (Exception) -> Unit,
) : Choreographer.FrameCallback {
    private val display = Choreographer.getInstance()
    private var open = true
    private var scheduled = false
    private var ticking = false
    private var observing = false
    private var previous = 0L
    private var deadline = 0L
    private val preference = Settings.Global.getUriFor(Settings.Global.ANIMATOR_DURATION_SCALE)
    private val observer =
        object : ContentObserver(Handler(Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean) {
                requestFrame()
            }
        }

    fun requestFrame() {
        if (!open) return
        if (!active()) {
            stop()

            return
        }
        // Uploads may synchronously notify the native view hierarchy. Never
        // query the Rust owner while its display callback is still borrowing it.
        if (scheduled || ticking) return
        if (!hasScenes()) {
            detach()

            return
        }

        if (!observing) {
            root.context.contentResolver.registerContentObserver(preference, false, observer)
            observing = true
        }

        scheduled = true
        display.postFrameCallback(this)
    }

    private fun active(): Boolean =
        root.isAttachedToWindow && root.isShown && root.hasWindowFocus() && root.windowVisibility == View.VISIBLE

    override fun doFrame(frameTimeNanos: Long) {
        scheduled = false

        if (!open || !active()) {
            stop()

            return
        }

        // A fixed cadence limits raster work on 90/120 Hz displays without
        // accumulating time while paused. The tolerance avoids halving 60 Hz.
        if (deadline != 0L && frameTimeNanos + 500_000L < deadline) {
            requestFrame()

            return
        }
        val elapsed = if (previous == 0L) 0.0 else (frameTimeNanos - previous).coerceAtLeast(0) / 1_000_000_000.0

        previous = frameTimeNanos
        deadline = if (deadline == 0L) frameTimeNanos + 16_666_667L else maxOf(deadline + 16_666_667L, frameTimeNanos + 1_000_000L)
        val pending =
            try {
                ticking = true
                val reduced = Settings.Global.getFloat(root.context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f

                tick(elapsed, root.resources.displayMetrics.density, if (reduced) 3 else 1)
            } catch (error: Exception) {
                stop()
                fail(error)

                return
            } finally {
                ticking = false
            }

        if (pending) requestFrame() else stop()
    }

    fun stop() {
        if (scheduled) display.removeFrameCallback(this)
        scheduled = false
        previous = 0
        deadline = 0
    }

    fun detach() {
        stop()

        if (observing) root.context.contentResolver.unregisterContentObserver(observer)
        observing = false
    }

    fun close() {
        if (!open) return
        open = false
        detach()
    }
}
