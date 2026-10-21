package dev.leptoskit.gallery

import android.app.Activity
import android.os.Bundle
import android.widget.FrameLayout
import dev.leptoskit.widgets.WidgetHost
import java.io.File

/** Displays the same Rust screen functions used by every gallery host. */
class GalleryActivity : Activity() {
    private lateinit var host: WidgetHost
    private lateinit var container: FrameLayout
    private var session = 0L

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        System.loadLibrary("gallery")
        host = WidgetHost(this)
        container = FrameLayout(this)
        setContentView(container)
        // Layout passes that keep the size (mounting, text changes) need no resize.
        container.addOnLayoutChangeListener { _, left, top, right, bottom, oldLeft, oldTop, oldRight, oldBottom ->
            if (right - left != oldRight - oldLeft || bottom - top != oldBottom - oldTop) {
                container.post {
                    if (!isFinishing && !isDestroyed) updateViewport()
                }
            }
        }
    }

    private fun updateViewport() {
        val width = container.width
        val height = container.height

        if (width > 0 && height > 0) {
            if (session == 0L) {
                session =
                    nativeCreate(
                        host,
                        width / host.density,
                        height / host.density,
                        intent.getStringExtra("screen") ?: "counter",
                        File(filesDir, "gallery-assets").path,
                    )
                mountRoot(nativeRoot(session))
            } else {
                mountRoot(nativeResize(session, width / host.density, height / host.density))
            }
        }
    }

    /** JNI calls this after a complete event-driven root replacement. */
    fun mountRoot(id: Long) {
        host.mount(id, container)
    }

    override fun onStart() {
        super.onStart()
        if (::host.isInitialized) host.setForeground(true)
    }

    override fun onStop() {
        if (::host.isInitialized) host.setForeground(false)
        super.onStop()
    }

    override fun onDestroy() {
        if (session != 0L) {
            val previous = session
            session = 0L
            nativeDestroy(previous)
        } else if (::host.isInitialized) {
            host.close()
        }

        super.onDestroy()
    }

    private external fun nativeCreate(
        host: WidgetHost,
        width: Float,
        height: Float,
        screen: String,
        assetDir: String,
    ): Long

    private external fun nativeRoot(session: Long): Long

    private external fun nativeResize(
        session: Long,
        width: Float,
        height: Float,
    ): Long

    private external fun nativeDestroy(session: Long)
}
