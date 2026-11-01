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
import android.util.Log
import android.view.Display
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
    override val methods = intArrayOf(SNAPSHOT, HOLD)
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

        requireWireArguments(method, payload)

        val activity = requireActivity()

        if (method == SNAPSHOT) return snapshot(activity)

        return hold(activity, payload)
    }

    /** A snapshot takes no argument; a hold takes exactly one policy id. */
    private fun requireWireArguments(
        method: Int,
        payload: ByteBuffer,
    ) {
        if (method !in SNAPSHOT..HOLD) throw PluginException(PluginException.UNAVAILABLE)

        val expected = if (method == HOLD) POLICY_BYTES else 0

        if (payload.remaining() != expected) throw PluginException(PluginException.INVALID_ARGUMENT)
    }

    /** The owning activity, while the host still presents it. */
    private fun requireActivity(): Activity {
        val activity = context as? Activity ?: throw PluginException(PluginException.UNAVAILABLE)

        if (activity.isDestroyed || activity.isFinishing) throw PluginException(PluginException.UNAVAILABLE)

        return activity
    }

    /** Reports the configured orientation and the display rotation in degrees. */
    private fun snapshot(activity: Activity): ByteArray {
        val orientation =
            when (activity.resources.configuration.orientation) {
                Configuration.ORIENTATION_PORTRAIT -> ORIENTATION_PORTRAIT
                Configuration.ORIENTATION_LANDSCAPE -> ORIENTATION_LANDSCAPE
                else -> ORIENTATION_UNKNOWN
            }
        val angle = rotationDegrees(activity)

        return ByteBuffer
            .allocate(SNAPSHOT_BYTES)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(orientation)
            .putInt(angle)
            .array()
    }

    /** The display's rotation from its natural orientation, or -1 when unknown. */
    private fun rotationDegrees(activity: Activity): Int {
        val rotation = activityDisplay(activity)?.rotation

        return when (rotation) {
            Surface.ROTATION_0 -> DEGREES_0
            Surface.ROTATION_90 -> DEGREES_90
            Surface.ROTATION_180 -> DEGREES_180
            Surface.ROTATION_270 -> DEGREES_270
            else -> UNKNOWN_ROTATION
        }
    }

    /** `Activity.display` needs API 30; older hosts read the window manager. */
    @Suppress("DEPRECATION")
    private fun activityDisplay(activity: Activity): Display? =
        if (Build.VERSION.SDK_INT >= DISPLAY_API) activity.display else activity.windowManager.defaultDisplay

    /** Takes over this activity's policy until the call settles or is cancelled. */
    private suspend fun hold(
        activity: Activity,
        payload: ByteBuffer,
    ): ByteArray {
        val requested = decodePolicy(payload.order(ByteOrder.LITTLE_ENDIAN).int)

        // Visible, not focused: apps commonly lock orientation at launch, before
        // the window gains focus, and a dialog may hold focus.
        if (activity.window?.decorView?.isShown != true) throw PluginException(PluginException.UNAVAILABLE)

        releaseStaleLease(activity)

        if (owners.containsKey(activity)) throw PluginException(PluginException.BUSY)

        return acquireLease(activity, requested)
    }

    /** Drops a lease whose call has already completed. */
    private fun releaseStaleLease(activity: Activity) {
        val owner = owners[activity] ?: return

        if (owner.pending?.isActive == true) return

        owner.finish(PluginException.CANCELLED)
    }

    /** Applies the policy and keeps the lease alive until the call settles. */
    private suspend fun acquireLease(
        activity: Activity,
        requested: Int,
    ): ByteArray {
        var lease: CancellableContinuation<ByteArray>? = null

        try {
            return suspendCancellableCoroutine { continuation ->
                lease = continuation
                beginLease(continuation, activity, requested)
            }
        } catch (cancelled: CancellationException) {
            restoreAfterLateCancellation(lease)

            throw cancelled
        }
    }

    /** Records the lease, applies the policy and wires cancellation. */
    private fun beginLease(
        continuation: CancellableContinuation<ByteArray>,
        activity: Activity,
        requested: Int,
    ) {
        if (!continuation.isActive) return

        original = activity.requestedOrientation
        pending = continuation
        owners[activity] = this
        activity.application.registerActivityLifecycleCallbacks(callbacks)
        observing = true

        apply(activity, requested)
        continuation.invokeOnCancellation { scheduleCleanup(continuation) }
    }

    /** Runs the cancelled lease's cleanup here on the main thread, else posted. */
    private fun scheduleCleanup(continuation: CancellableContinuation<ByteArray>) {
        val cleanup = Runnable { if (pending === continuation) finish(PluginException.CANCELLED) }

        if (Looper.myLooper() == Looper.getMainLooper()) cleanup.run() else handler.post(cleanup)
    }

    /** Applies the requested policy, translating the platform's refusals. */
    private fun apply(
        activity: Activity,
        requested: Int,
    ) {
        try {
            activity.requestedOrientation = requested
        } catch (_: SecurityException) {
            finish(PluginException.PERMISSION_DENIED)
        } catch (_: IllegalStateException) {
            finish(PluginException.UNAVAILABLE)
        }
    }

    /**
     * Cancelled off the main thread, the posted cleanup can run after this call
     * has already completed: coroutine dispatch on the main looper may overtake
     * it. The resumed call is back on the main thread here, so restore the
     * original policy before the cancellation is observed.
     */
    private fun restoreAfterLateCancellation(lease: CancellableContinuation<ByteArray>?) {
        if (lease == null) return

        if (pending !== lease) return

        if (Looper.myLooper() != Looper.getMainLooper()) return

        finish(PluginException.CANCELLED)
    }

    override fun close() {
        check(Looper.myLooper() == Looper.getMainLooper())

        closed = true
        finish(PluginException.CANCELLED)
    }

    /** Ends the lease, putting back the policy the activity had before it. */
    private fun finish(reason: Int) {
        val continuation = pending ?: return
        val activity = context as Activity

        pending = null
        owners.remove(activity)
        stopObserving(activity)

        val failure = restore(activity, reason)

        if (continuation.isActive) {
            continuation.resumeWithException(PluginException(failure))
        } else if (failure != reason) {
            Log.e(TAG, "Could not restore Activity orientation policy")
        }
    }

    /** Stops watching an activity that no longer holds a lease. */
    private fun stopObserving(activity: Activity) {
        if (!observing) return

        observing = false
        activity.application.unregisterActivityLifecycleCallbacks(callbacks)
    }

    /** Puts the saved policy back, reporting the failure that should surface. */
    private fun restore(
        activity: Activity,
        fallback: Int,
    ): Int {
        val saved = original

        original = null

        if (saved == null || activity.isDestroyed) return fallback

        return try {
            activity.requestedOrientation = saved
            fallback
        } catch (_: SecurityException) {
            PluginException.PERMISSION_DENIED
        } catch (_: IllegalStateException) {
            PluginException.NATIVE
        }
    }

    internal companion object {
        private val owners = WeakHashMap<Activity, ScreenOrientationPlugin>()

        /** Log tag for a policy that could not be put back. */
        private const val TAG = "LeptosKitOrientation"

        /** Method ids the plugin exposes. */
        const val SNAPSHOT = 0
        const val HOLD = 1

        /** A hold request carries one little-endian policy id. */
        const val POLICY_BYTES = 4

        /** A snapshot reports the orientation and the rotation angle, two u32s. */
        const val SNAPSHOT_BYTES = 8

        /** Snapshot orientation codes shared with the Rust wire format. */
        internal const val ORIENTATION_UNKNOWN = 0
        internal const val ORIENTATION_PORTRAIT = 1
        internal const val ORIENTATION_LANDSCAPE = 2

        /** Rotation angles in degrees, and the sentinel for an unknown one. */
        const val DEGREES_0 = 0
        const val DEGREES_90 = 90
        const val DEGREES_180 = 180
        const val DEGREES_270 = 270
        const val UNKNOWN_ROTATION = -1

        /** `Activity.display` is available from API 30. */
        const val DISPLAY_API = 30

        /** Policy ids from the wire format. */
        internal const val POLICY_SENSOR_PORTRAIT = 0
        internal const val POLICY_SENSOR_LANDSCAPE = 1
        internal const val POLICY_PORTRAIT = 2
        internal const val POLICY_REVERSE_PORTRAIT = 3
        internal const val POLICY_LANDSCAPE = 4
        internal const val POLICY_REVERSE_LANDSCAPE = 5
        internal const val POLICY_FULL_SENSOR = 6

        /** Maps a wire policy id to the platform's requested orientation. */
        internal fun decodePolicy(value: Int): Int =
            when (value) {
                POLICY_SENSOR_PORTRAIT -> ActivityInfo.SCREEN_ORIENTATION_SENSOR_PORTRAIT
                POLICY_SENSOR_LANDSCAPE -> ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
                POLICY_PORTRAIT -> ActivityInfo.SCREEN_ORIENTATION_PORTRAIT
                POLICY_REVERSE_PORTRAIT -> ActivityInfo.SCREEN_ORIENTATION_REVERSE_PORTRAIT
                POLICY_LANDSCAPE -> ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE
                POLICY_REVERSE_LANDSCAPE -> ActivityInfo.SCREEN_ORIENTATION_REVERSE_LANDSCAPE
                POLICY_FULL_SENSOR -> ActivityInfo.SCREEN_ORIENTATION_FULL_SENSOR
                else -> throw PluginException(PluginException.INVALID_ARGUMENT)
            }
    }
}
