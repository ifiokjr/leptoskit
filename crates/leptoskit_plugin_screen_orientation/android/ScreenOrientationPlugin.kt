package dev.leptoskit.plugins.screenorientation

import android.app.Activity
import android.app.Application
import android.content.Context
import android.content.pm.ActivityInfo
import android.content.res.Configuration
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.Surface
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.suspendCancellableCoroutine
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.WeakHashMap
import kotlin.coroutines.cancellation.CancellationException
import kotlin.coroutines.resumeWithException

/** One policy lease per owning Activity. The OS may ignore orientation requests. */
class ScreenOrientationPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "screen-orientation"
    override val methods = intArrayOf(0, 1)
    private val handler = Handler(Looper.getMainLooper())
    private var pending: CancellableContinuation<ByteArray>? = null
    private var original: Int? = null
    private var closed = false
    private var observing = false
    private val callbacks =
        object : Application.ActivityLifecycleCallbacks {
            override fun onActivityPaused(activity: Activity) {
                if (activity === context) finish(PluginException.CANCELLED)
            }

            override fun onActivityDestroyed(activity: Activity) {
                if (activity === context) close()
            }

            override fun onActivityCreated(
                activity: Activity,
                state: Bundle?,
            ) {}

            override fun onActivityStarted(activity: Activity) {}

            override fun onActivityResumed(activity: Activity) {}

            override fun onActivityStopped(activity: Activity) {}

            override fun onActivitySaveInstanceState(
                activity: Activity,
                state: Bundle,
            ) {}
        }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper())

        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method !in 0..1) throw PluginException(PluginException.UNAVAILABLE)

        if (payload.remaining() != if (method == 1) 4 else 0) throw PluginException(PluginException.INVALID_ARGUMENT)
        val activity = context as? Activity ?: throw PluginException(PluginException.UNAVAILABLE)

        if (activity.isDestroyed || activity.isFinishing) throw PluginException(PluginException.UNAVAILABLE)

        if (method == 0) {
            val orientation =
                when (activity.resources.configuration.orientation) {
                    Configuration.ORIENTATION_PORTRAIT -> 1
                    Configuration.ORIENTATION_LANDSCAPE -> 2
                    else -> 0
                }

            @Suppress("DEPRECATION")
            val display = if (Build.VERSION.SDK_INT >= 30) activity.display else activity.windowManager.defaultDisplay
            val angle =
                when (display?.rotation) {
                    Surface.ROTATION_0 -> 0
                    Surface.ROTATION_90 -> 90
                    Surface.ROTATION_180 -> 180
                    Surface.ROTATION_270 -> 270
                    else -> -1
                }

            return ByteBuffer
                .allocate(8)
                .order(ByteOrder.LITTLE_ENDIAN)
                .putInt(orientation)
                .putInt(angle)
                .array()
        }

        val requested = decodePolicy(payload.order(ByteOrder.LITTLE_ENDIAN).int)
        // Visible, not focused: apps commonly lock orientation at launch, before
        // the window gains focus, and a dialog may hold focus.
        if (activity.window?.decorView?.isShown != true) throw PluginException(PluginException.UNAVAILABLE)

        owners[activity]?.let { owner -> if (owner.pending?.isActive != true) owner.finish(PluginException.CANCELLED) }

        if (owners.containsKey(activity)) throw PluginException(PluginException.BUSY)
        var lease: CancellableContinuation<ByteArray>? = null
        try {
            return suspendCancellableCoroutine { continuation ->

                if (!continuation.isActive) return@suspendCancellableCoroutine
                lease = continuation
                original = activity.requestedOrientation
                pending = continuation
                owners[activity] = this
                activity.application.registerActivityLifecycleCallbacks(callbacks)
                observing = true
                try {
                    activity.requestedOrientation = requested
                } catch (_: SecurityException) {
                    finish(PluginException.PERMISSION_DENIED)
                } catch (_: IllegalStateException) {
                    finish(PluginException.UNAVAILABLE)
                }
                continuation.invokeOnCancellation {
                    val cleanup = Runnable { if (pending === continuation) finish(PluginException.CANCELLED) }

                    if (Looper.myLooper() == Looper.getMainLooper()) cleanup.run() else handler.post(cleanup)
                }
            }
        } catch (cancelled: CancellationException) {
            // Cancelled off the main thread, the posted cleanup can run after this call has
            // already completed: coroutine dispatch on the main looper may overtake it. The
            // resumed call is back on the main thread here, so restore the original policy
            // before the cancellation is observed.
            val continuation = lease

            if (continuation != null && pending === continuation && Looper.myLooper() == Looper.getMainLooper()) {
                finish(PluginException.CANCELLED)
            }

            throw cancelled
        }
    }

    override fun close() {
        check(Looper.myLooper() == Looper.getMainLooper())
        closed = true
        finish(PluginException.CANCELLED)
    }

    private fun finish(reason: Int) {
        val continuation = pending ?: return
        pending = null
        val activity = context as Activity
        owners.remove(activity)

        if (observing) {
            observing = false
            activity.application.unregisterActivityLifecycleCallbacks(callbacks)
        }

        var failure = reason

        val saved = original
        original = null
        try {
            if (saved != null && !activity.isDestroyed) activity.requestedOrientation = saved
        } catch (_: SecurityException) {
            failure = PluginException.PERMISSION_DENIED
        } catch (_: IllegalStateException) {
            failure = PluginException.NATIVE
        }

        if (continuation.isActive) {
            continuation.resumeWithException(PluginException(failure))
        } else if (failure != reason) {
            android.util.Log.e("LeptosKitOrientation", "Could not restore Activity orientation policy")
        }
    }

    internal companion object {
        private val owners = WeakHashMap<Activity, ScreenOrientationPlugin>()

        internal fun decodePolicy(value: Int): Int =
            when (value) {
                0 -> ActivityInfo.SCREEN_ORIENTATION_SENSOR_PORTRAIT
                1 -> ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
                2 -> ActivityInfo.SCREEN_ORIENTATION_PORTRAIT
                3 -> ActivityInfo.SCREEN_ORIENTATION_REVERSE_PORTRAIT
                4 -> ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE
                5 -> ActivityInfo.SCREEN_ORIENTATION_REVERSE_LANDSCAPE
                6 -> ActivityInfo.SCREEN_ORIENTATION_FULL_SENSOR
                else -> throw PluginException(PluginException.INVALID_ARGUMENT)
            }
    }
}
