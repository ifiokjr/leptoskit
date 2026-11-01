package dev.leptoskit.plugins.keepawake

import android.app.Activity
import android.app.Application
import android.content.Context
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.WindowManager
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.suspendCancellableCoroutine
import java.nio.ByteBuffer
import kotlin.coroutines.resumeWithException

/** Every live invocation owns one reference; cancellation releases that reference. */
class KeepAwakePlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "keep-awake"
    override val methods = intArrayOf(0)
    private val pending = mutableSetOf<CancellableContinuation<ByteArray>>()
    private val handler = Handler(Looper.getMainLooper())
    private var closed = false
    private var original = false
    private var observing = false

    private companion object {
        /** The only method: takes another lease. */
        const val STATUS = 0

        /** How many leases one app may hold at once. */
        const val MAX_LEASES = 64
    }

    private val callbacks =
        object : Application.ActivityLifecycleCallbacks {
            override fun onActivityStopped(activity: Activity) {
                if (activity === context) finishAll()
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

            override fun onActivityPaused(activity: Activity) {}

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

        if (method != STATUS) throw PluginException(PluginException.UNAVAILABLE)

        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        if (pending.size >= MAX_LEASES) throw PluginException(PluginException.BUSY)
        val activity = visibleActivity()

        return suspendCancellableCoroutine { continuation ->
            if (!continuation.isActive) return@suspendCancellableCoroutine

            hold(activity)
            pending.add(continuation)
            releaseOnCancel(continuation)
        }
    }

    /** The attached activity, when it is visible enough to keep the screen on. */
    private fun visibleActivity(): Activity {
        val activity = context as? Activity ?: throw PluginException(PluginException.UNAVAILABLE)

        // Visible, not focused: a lease can start at launch, before the window
        // gains focus, or while a dialog has it.
        if (activity.isDestroyed || activity.isFinishing || activity.window?.decorView?.isShown != true) {
            throw PluginException(PluginException.UNAVAILABLE)
        }

        return activity
    }

    /** Keeps the screen on for the first lease, and watches the activity for it. */
    private fun hold(activity: Activity) {
        if (pending.isNotEmpty()) return
        original = activity.window.attributes.flags and WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON != 0
        activity.window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        activity.application.registerActivityLifecycleCallbacks(callbacks)
        observing = true
    }

    /** Drops the lease when its caller cancels, on the main thread. */
    private fun releaseOnCancel(continuation: CancellableContinuation<ByteArray>) {
        continuation.invokeOnCancellation {
            val remove =
                Runnable {
                    pending.remove(continuation)

                    if (pending.isEmpty()) stop()
                }

            if (Looper.myLooper() == Looper.getMainLooper()) remove.run() else handler.post(remove)
        }
    }

    override fun close() {
        check(Looper.myLooper() == Looper.getMainLooper())
        closed = true
        finishAll()
    }

    private fun finishAll() {
        val continuations = pending.toList()
        pending.clear()
        stop()

        for (continuation in continuations) continuation.resumeWithException(PluginException(PluginException.CANCELLED))
    }

    private fun stop() {
        if (!observing) return
        observing = false

        val activity = context as? Activity ?: return
        activity.application.unregisterActivityLifecycleCallbacks(callbacks)

        if (!original) activity.window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
    }
}
