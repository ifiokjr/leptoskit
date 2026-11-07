package dev.leptoskit.plugins.pictureinpicture

import android.app.Activity
import android.app.Application
import android.app.PictureInPictureParams
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.PixelFormat
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.view.Gravity
import android.view.View
import android.view.WindowManager
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.suspendCancellableCoroutine
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.coroutines.resumeWithException

/**
 * Activity picture-in-picture and SYSTEM_ALERT_WINDOW overlays.
 *
 * Overlay permission is never inferred from the manifest declaration. Entering
 * overlay without a grant returns permission-denied. Picture-in-picture needs
 * `android:supportsPictureInPicture="true"` on the hosting activity.
 */
class PictureInPicturePlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "picture-in-picture"
    override val methods =
        intArrayOf(CAPABILITIES, SNAPSHOT, ENTER_PIP, ENTER_OVERLAY, OVERLAY_STATUS, REQUEST_OVERLAY)

    private val handler = Handler(Looper.getMainLooper())
    private var closed = false
    private var pipLease: CancellableContinuation<ByteArray>? = null
    private var overlayLease: CancellableContinuation<ByteArray>? = null
    private var overlayView: View? = null
    private var observing = false

    private val lifecycle =
        object : Application.ActivityLifecycleCallbacks {
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

        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)

        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        return when (method) {
            CAPABILITIES -> capabilities()
            SNAPSHOT -> snapshot()
            ENTER_PIP -> enterPip()
            ENTER_OVERLAY -> enterOverlay()
            OVERLAY_STATUS -> overlayStatus().toBytes()
            REQUEST_OVERLAY -> requestOverlay()
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }
    }

    override fun close() {
        check(Looper.myLooper() == Looper.getMainLooper())

        if (closed) return

        closed = true
        finishPip(PluginException.CANCELLED)
        finishOverlay(PluginException.CANCELLED)
        stopObserving()
    }

    private fun capabilities(): ByteArray {
        val pip = supportsPip()
        val overlay = Build.VERSION.SDK_INT >= Build.VERSION_CODES.O

        return words(WIRE_VERSION, if (pip) 1 else 0, if (overlay) 1 else 0)
    }

    private fun snapshot(): ByteArray {
        val session =
            when {
                pipLease != null -> SESSION_PIP
                overlayLease != null -> SESSION_OVERLAY
                else -> SESSION_NONE
            }

        return words(WIRE_VERSION, session)
    }

    private suspend fun enterPip(): ByteArray {
        if (pipLease != null || overlayLease != null) throw PluginException(PluginException.BUSY)

        if (!supportsPip()) throw PluginException(PluginException.UNAVAILABLE)

        val activity = requireActivity()

        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) {
            throw PluginException(PluginException.UNAVAILABLE)
        }

        val entered =
            try {
                activity.enterPictureInPictureMode(PictureInPictureParams.Builder().build())
            } catch (_: IllegalStateException) {
                throw PluginException(PluginException.UNAVAILABLE)
            }

        if (!entered) throw PluginException(PluginException.UNAVAILABLE)

        observe(activity)

        return suspendCancellableCoroutine { continuation ->
            pipLease = continuation
            continuation.invokeOnCancellation {
                runOnMain {
                    if (pipLease === continuation) {
                        pipLease = null
                        stopObservingIfIdle()
                    }
                }
            }
        }
    }

    private suspend fun enterOverlay(): ByteArray {
        if (pipLease != null || overlayLease != null) throw PluginException(PluginException.BUSY)

        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) {
            throw PluginException(PluginException.UNAVAILABLE)
        }

        if (!Settings.canDrawOverlays(context)) {
            throw PluginException(PluginException.PERMISSION_DENIED)
        }

        val activity = requireActivity()
        val manager =
            context.getSystemService(Context.WINDOW_SERVICE) as? WindowManager
                ?: throw PluginException(PluginException.UNAVAILABLE)

        val view = View(context).apply { setBackgroundColor(0x00000000) }
        val params =
            WindowManager.LayoutParams(
                OVERLAY_SIZE_PX,
                OVERLAY_SIZE_PX,
                WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
                WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or
                    WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
                PixelFormat.TRANSLUCENT,
            ).apply {
                gravity = Gravity.TOP or Gravity.START
                x = OVERLAY_MARGIN_PX
                y = OVERLAY_MARGIN_PX
            }

        try {
            manager.addView(view, params)
        } catch (_: WindowManager.BadTokenException) {
            throw PluginException(PluginException.PERMISSION_DENIED)
        } catch (_: IllegalStateException) {
            throw PluginException(PluginException.NATIVE)
        }

        overlayView = view
        observe(activity)

        return suspendCancellableCoroutine { continuation ->
            overlayLease = continuation
            continuation.invokeOnCancellation {
                runOnMain {
                    if (overlayLease === continuation) {
                        removeOverlay()
                        overlayLease = null
                        stopObservingIfIdle()
                    }
                }
            }
        }
    }

    private fun overlayStatus(): Int {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return OVERLAY_UNSUPPORTED

        return if (Settings.canDrawOverlays(context)) OVERLAY_GRANTED else OVERLAY_DENIED
    }

    private fun requestOverlay(): ByteArray {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) {
            throw PluginException(PluginException.UNAVAILABLE)
        }

        val status = overlayStatus()

        if (status == OVERLAY_GRANTED) return status.toBytes()

        val intent =
            Intent(
                Settings.ACTION_MANAGE_OVERLAY_PERMISSION,
                Uri.parse("package:${context.packageName}"),
            ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

        try {
            context.startActivity(intent)
        } catch (_: Exception) {
            throw PluginException(PluginException.UNAVAILABLE)
        }

        // The user returns later; report the status observed at launch time.
        return overlayStatus().toBytes()
    }

    private fun supportsPip(): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return false

        if (!context.packageManager.hasSystemFeature(PackageManager.FEATURE_PICTURE_IN_PICTURE)) {
            return false
        }

        val activity = context as? Activity ?: return false

        return activity.packageManager
            .getActivityInfo(activity.componentName, 0)
            .supportsPictureInPicture()
    }

    private fun requireActivity(): Activity {
        val activity = context as? Activity ?: throw PluginException(PluginException.UNAVAILABLE)

        if (activity.isDestroyed || activity.isFinishing) {
            throw PluginException(PluginException.UNAVAILABLE)
        }

        return activity
    }

    private fun observe(activity: Activity) {
        if (observing) return

        activity.application.registerActivityLifecycleCallbacks(lifecycle)
        observing = true
    }

    private fun stopObservingIfIdle() {
        if (pipLease == null && overlayLease == null) stopObserving()
    }

    private fun stopObserving() {
        if (!observing) return

        observing = false
        val activity = context as? Activity ?: return

        activity.application.unregisterActivityLifecycleCallbacks(lifecycle)
    }

    private fun finishPip(code: Int) {
        val continuation = pipLease ?: return

        pipLease = null
        continuation.resumeWithException(PluginException(code))
    }

    private fun finishOverlay(code: Int) {
        val continuation = overlayLease ?: return

        removeOverlay()
        overlayLease = null
        continuation.resumeWithException(PluginException(code))
    }

    private fun removeOverlay() {
        val view = overlayView ?: return
        overlayView = null

        val manager = context.getSystemService(Context.WINDOW_SERVICE) as? WindowManager ?: return

        try {
            manager.removeView(view)
        } catch (_: IllegalArgumentException) {
            // Already detached.
        }
    }

    private fun runOnMain(action: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) {
            action()
        } else {
            handler.post(action)
        }
    }

    private fun Int.toBytes(): ByteArray =
        ByteBuffer
            .allocate(4)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(this)
            .array()

    private fun words(vararg values: Int): ByteArray {
        val buffer = ByteBuffer.allocate(values.size * 4).order(ByteOrder.LITTLE_ENDIAN)

        for (value in values) buffer.putInt(value)

        return buffer.array()
    }

    private companion object {
        const val CAPABILITIES = 0
        const val SNAPSHOT = 1
        const val ENTER_PIP = 2
        const val ENTER_OVERLAY = 3
        const val OVERLAY_STATUS = 4
        const val REQUEST_OVERLAY = 5

        const val WIRE_VERSION = 1
        const val SESSION_NONE = 0
        const val SESSION_PIP = 1
        const val SESSION_OVERLAY = 2

        const val OVERLAY_UNSUPPORTED = 0
        const val OVERLAY_DENIED = 1
        const val OVERLAY_GRANTED = 2

        const val OVERLAY_SIZE_PX = 1
        const val OVERLAY_MARGIN_PX = 0
    }
}
