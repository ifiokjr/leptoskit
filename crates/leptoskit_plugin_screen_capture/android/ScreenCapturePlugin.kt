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
    override val methods = intArrayOf(STATUS, PROTECTION, SCREENSHOT, PREVENT)
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
                if (activity !== context) return

                wait?.let { finishWait(it.id, PluginException(PluginException.CANCELLED)) }
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

    /**
     * Answers one call on the UI thread, because every window and lifecycle call
     * needs it. Protection and capture watches are registered here and released
     * when their caller drops them.
     */
    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper())

        requireOpen(method)
        val watch = readWatch(method, payload)
        currentCoroutineContext().ensureActive()
        val activity = requireActivity()

        return answer(method, activity, watch)
    }

    /** Rejects a closed plugin or a method this host does not answer. */
    private fun requireOpen(method: Int) {
        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)
    }

    /** Answers the call, now that its payload and the activity are checked. */
    private suspend fun answer(
        method: Int,
        activity: Activity,
        watch: CaptureWatch?,
    ): ByteArray {
        return when (method) {
            STATUS -> statusAnswer()
            PROTECTION -> protectionAnswer(activity)
            SCREENSHOT -> watchScreenshot(activity, checkNotNull(watch))
            else -> prevent(activity)
        }
    }

    /** Reports the wire version and whether the OS can report a capture. */
    private fun statusAnswer(): ByteArray {
        return fields(WIRE_VERSION, SUPPORTED, 0, if (supportsCaptureDetection()) 1 else 0)
    }

    /** Reports whether the window already carries the secure flag. */
    private fun protectionAnswer(activity: Activity): ByteArray {
        return fields(WIRE_VERSION, 0, if (isProtected(activity)) SECURE_ON else SECURE_OFF)
    }

    /** Reads the watch a screenshot request carries; other methods carry nothing. */
    private fun readWatch(
        method: Int,
        payload: ByteBuffer,
    ): CaptureWatch? {
        val expected = if (method == SCREENSHOT) CaptureWatch.BYTES else 0

        if (payload.remaining() != expected) throw PluginException(PluginException.INVALID_ARGUMENT)

        if (method != SCREENSHOT) return null

        return CaptureWatch.decode(payload)
    }

    /** The live activity every window operation attaches to. */
    private fun requireActivity(): Activity {
        val activity = context as? Activity ?: throw PluginException(PluginException.UNAVAILABLE)

        if (activity.isDestroyed || activity.isFinishing) throw PluginException(PluginException.UNAVAILABLE)

        return activity
    }

    /** Screen-capture detection arrived in API 34. */
    private fun supportsCaptureDetection(): Boolean {
        return Build.VERSION.SDK_INT >= DETECT_CAPTURE_API
    }

    /** Whether the window already carries the secure flag. */
    private fun isProtected(activity: Activity): Boolean {
        return activity.window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE != 0
    }

    private suspend fun prevent(activity: Activity): ByteArray =
        suspendCancellableCoroutine { completion ->

            if (!completion.isActive) return@suspendCancellableCoroutine

            if (leases.size >= MAX_LEASES) {
                completion.resumeWithException(PluginException(PluginException.BUSY))

                return@suspendCancellableCoroutine
            }

            lease(activity, completion)
        }

    /** Installs the secure flag and remembers the lease until the caller drops it. */
    private fun lease(
        activity: Activity,
        completion: CancellableContinuation<ByteArray>,
    ) {
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

    /** Rejects a watch the OS cannot serve before it registers anything. */
    private suspend fun watchScreenshot(
        activity: Activity,
        watch: CaptureWatch,
    ): ByteArray {
        if (watch.kind != CaptureWatch.OBSERVE || !supportsCaptureDetection()) {
            throw PluginException(PluginException.UNAVAILABLE)
        }

        if (activity.checkSelfPermission(Manifest.permission.DETECT_SCREEN_CAPTURE) != PackageManager.PERMISSION_GRANTED) {
            throw PluginException(PluginException.PERMISSION_DENIED)
        }

        // Visible, not focused: a dialog or system prompt over the activity still
        // leaves its content on screen to be captured.
        if (activity.window?.decorView?.isShown != true) throw PluginException(PluginException.UNAVAILABLE)

        return awaitCapture(activity)
    }

    private suspend fun awaitCapture(activity: Activity): ByteArray =
        suspendCancellableCoroutine { completion ->

            if (!completion.isActive) return@suspendCancellableCoroutine

            if (wait != null) {
                completion.resumeWithException(PluginException(PluginException.BUSY))

                return@suspendCancellableCoroutine
            }

            val id = UUID.randomUUID()
            wait = Pending(id, completion)
            observe(activity, id, completion)
        }

    /** Registers the observation and answers early if it cannot be installed. */
    private fun observe(
        activity: Activity,
        id: UUID,
        completion: CancellableContinuation<ByteArray>,
    ) {
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

    /** Answers the wait this id belongs to, if it is still the current one. */
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

        if (error == null) pending.completion.resume(fields(WIRE_VERSION, SUPPORTED, 0)) else {
            pending.completion.resumeWithException(error)
        }
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

    private companion object {
        /** Methods the Rust crate calls. */
        const val STATUS = 0
        const val PROTECTION = 1
        const val SCREENSHOT = 2
        const val PREVENT = 3

        /** Wire version and the secure-flag answers the Rust side decodes. */
        const val WIRE_VERSION = 1
        const val SUPPORTED = 1
        const val SECURE_OFF = 1
        const val SECURE_ON = 2

        /** The first API level that reports screen captures. */
        const val DETECT_CAPTURE_API = 34

        /** How many windows may hold protection at once. */
        const val MAX_LEASES = 64
    }
}

internal data class CaptureWatch(
    val kind: Int,
    val previous: Int,
) {
    companion object {
        /** A watch either observes captures or reports the previous outcome. */
        const val SCREENSHOT = 0
        const val OBSERVE = 1

        /** The outcome the previous watch ended with. */
        const val NONE = 0
        const val TAKEN = 1
        const val DENIED = 2

        /** A screenshot request carries a kind and a previous outcome. */
        const val BYTES = 8

        /** Rejects a kind or outcome the Rust crate does not define. */
        fun decode(payload: ByteBuffer): CaptureWatch {
            val bytes = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)

            if (bytes.remaining() != BYTES) throw PluginException(PluginException.INVALID_ARGUMENT)
            val kind = bytes.int
            val previous = bytes.int

            if (kind !in SCREENSHOT..OBSERVE) throw PluginException(PluginException.INVALID_ARGUMENT)

            if (previous !in NONE..DENIED) throw PluginException(PluginException.INVALID_ARGUMENT)

            if (kind == OBSERVE && previous != NONE) throw PluginException(PluginException.INVALID_ARGUMENT)

            return CaptureWatch(kind, previous)
        }
    }
}

internal fun fields(vararg values: Int): ByteArray {
    val output = ByteBuffer.allocate(values.size * FIELD_BYTES).order(ByteOrder.LITTLE_ENDIAN)

    for (value in values) output.putInt(value)

    return output.array()
}

/** Every wire integer is one little-endian u32. */
private const val FIELD_BYTES = 4
