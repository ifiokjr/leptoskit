package dev.leptoskit.plugins.screencapture

import android.Manifest
import android.app.Activity
import android.app.Application
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.Window
import android.view.WindowManager
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.suspendCancellableCoroutine
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.UUID
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/** Window-only protection. It never reads a screenshot or claims recording detection. */
class ScreenCapturePlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "screen-capture"
    override val methods = intArrayOf(0, 1, 2, 3)
    private val handler = Handler(Looper.getMainLooper())
    private val leases = mutableMapOf<CancellableContinuation<ByteArray>, Window>()
    private var closed = false
    private var observing = false
    private var wait: Pending? = null
    private var screenshot: ScreenshotObservation? = null

    private data class Pending(
        val id: UUID,
        val completion: CancellableContinuation<ByteArray>,
    )

    private val lifecycle =
        object : Application.ActivityLifecycleCallbacks {
            override fun onActivityStopped(activity: Activity) {
                if (activity === context) wait?.let { finishWait(it.id, PluginException(PluginException.CANCELLED)) }
                // Protection stays installed while stopped so recents snapshots remain secure.
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
        if (method !in 0..3) throw PluginException(PluginException.UNAVAILABLE)
        if (payload.remaining() != if (method == 2) 8 else 0) throw PluginException(PluginException.INVALID_ARGUMENT)
        val watch = if (method == 2) CaptureWatch.decode(payload) else null
        currentCoroutineContext().ensureActive()
        val activity = context as? Activity ?: throw PluginException(PluginException.UNAVAILABLE)
        if (activity.isDestroyed || activity.isFinishing) throw PluginException(PluginException.UNAVAILABLE)
        return when (method) {
            0 -> {
                fields(1, 1, 0, if (Build.VERSION.SDK_INT >= 34) 1 else 0)
            }

            1 -> {
                fields(1, 0, if (activity.window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE != 0) 2 else 1)
            }

            2 -> {
                if (checkNotNull(watch).kind == 0 || Build.VERSION.SDK_INT < 34) {
                    throw PluginException(PluginException.UNAVAILABLE)
                }
                if (activity.checkSelfPermission(Manifest.permission.DETECT_SCREEN_CAPTURE) != PackageManager.PERMISSION_GRANTED) {
                    throw PluginException(PluginException.PERMISSION_DENIED)
                }
                if (!activity.hasWindowFocus()) throw PluginException(PluginException.UNAVAILABLE)
                watchScreenshot(activity)
            }

            else -> {
                if (!activity.hasWindowFocus()) throw PluginException(PluginException.UNAVAILABLE)
                prevent(activity)
            }
        }
    }

    private suspend fun prevent(activity: Activity): ByteArray =
        suspendCancellableCoroutine { completion ->
            if (!completion.isActive) return@suspendCancellableCoroutine
            if (leases.size >= 64) {
                completion.resumeWithException(PluginException(PluginException.BUSY))
                return@suspendCancellableCoroutine
            }
            try {
                observeLifecycle(activity)
                val window = activity.window
                SecureWindowLeases.acquire(window)
                leases[completion] = window
                completion.invokeOnCancellation {
                    onMain {
                        leases.remove(completion)?.let { SecureWindowLeases.release(it) }
                        stopLifecycleIfIdle()
                    }
                }
            } catch (error: PluginException) {
                stopLifecycleIfIdle()
                completion.resumeWithException(error)
            } catch (error: RuntimeException) {
                stopLifecycleIfIdle()
                completion.resumeWithException(PluginException(PluginException.NATIVE, error))
            }
        }

    private suspend fun watchScreenshot(activity: Activity): ByteArray =
        suspendCancellableCoroutine { completion ->
            if (!completion.isActive) return@suspendCancellableCoroutine
            if (wait != null) {
                completion.resumeWithException(PluginException(PluginException.BUSY))
                return@suspendCancellableCoroutine
            }
            val id = UUID.randomUUID()
            wait = Pending(id, completion)
            try {
                observeLifecycle(activity)
                screenshot = ScreenshotObservation(activity) { finishWait(id, null) }
                completion.invokeOnCancellation { onMain { finishWait(id, PluginException(PluginException.CANCELLED)) } }
            } catch (error: SecurityException) {
                finishWait(id, PluginException(PluginException.PERMISSION_DENIED, error))
            } catch (error: RuntimeException) {
                finishWait(id, PluginException(PluginException.NATIVE, error))
            }
        }

    private fun finishWait(
        id: UUID,
        error: PluginException?,
    ) {
        val pending = wait ?: return
        if (pending.id != id) return
        wait = null
        screenshot?.close()
        screenshot = null
        stopLifecycleIfIdle()
        if (!pending.completion.isActive) return
        if (error == null) pending.completion.resume(fields(1, 1, 0)) else pending.completion.resumeWithException(error)
    }

    private fun observeLifecycle(activity: Activity) {
        if (observing) return
        activity.application.registerActivityLifecycleCallbacks(lifecycle)
        observing = true
    }

    private fun stopLifecycleIfIdle() {
        if (!observing || leases.isNotEmpty() || wait != null) return
        observing = false
        (context as Activity).application.unregisterActivityLifecycleCallbacks(lifecycle)
    }

    private fun onMain(block: () -> Unit) {
        if (Looper.myLooper() == handler.looper) block() else handler.post(block)
    }

    override fun close() {
        check(Looper.myLooper() == Looper.getMainLooper())
        if (closed) return
        closed = true
        wait?.let { finishWait(it.id, PluginException(PluginException.CANCELLED)) }
        val pending = leases.toMap()
        leases.clear()
        for ((completion, window) in pending) {
            SecureWindowLeases.release(window)
            if (completion.isActive) completion.resumeWithException(PluginException(PluginException.CANCELLED))
        }
        stopLifecycleIfIdle()
    }
}

internal data class CaptureWatch(
    val kind: Int,
    val previous: Int,
) {
    companion object {
        fun decode(payload: ByteBuffer): CaptureWatch {
            val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
            if (bytes.remaining() != 8) throw PluginException(PluginException.INVALID_ARGUMENT)
            val kind = bytes.int
            val previous = bytes.int
            if (kind !in 0..1 || previous !in 0..2 || (kind == 1 && previous != 0)) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }
            return CaptureWatch(kind, previous)
        }
    }
}

internal fun fields(vararg values: Int): ByteArray =
    ByteBuffer
        .allocate(values.size * 4)
        .order(ByteOrder.LITTLE_ENDIAN)
        .apply {
            values.forEach { putInt(it) }
        }.array()
