package dev.leptoskit.plugins.screencapture

import android.annotation.TargetApi
import android.app.Activity

/** No photo-library access: the OS reports capture of this Activity after it occurs. */
@TargetApi(34)
internal class ScreenshotObservation(
    activity: Activity,
    captured: () -> Unit,
) {
    private val owner = activity
    private val callback = Activity.ScreenCaptureCallback { captured() }
    private var registered = false

    init {
        owner.registerScreenCaptureCallback(owner.mainExecutor, callback)
        registered = true
    }

    fun close() {
        if (!registered) return
        registered = false
        owner.unregisterScreenCaptureCallback(callback)
    }
}
