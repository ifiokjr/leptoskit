package dev.leptoskit.gallery

import android.app.Activity
import android.os.Build
import android.os.Bundle
import android.view.WindowInsets
import android.widget.FrameLayout
import dev.leptoskit.widgets.WidgetHost
import java.io.File

/** Displays the same Rust screen functions used by every gallery host. */
class GalleryActivity : Activity() {
    private lateinit var host: WidgetHost
    private lateinit var container: FrameLayout
    private var session = 0L

    /** System bars and cutouts drawn over the container, in pixels. */
    private var safeArea = SafeArea(0, 0, 0, 0)

    private data class SafeArea(
        val top: Int,
        val right: Int,
        val bottom: Int,
        val left: Int,
    )

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
        // Edge-to-edge windows draw the screen under the status and navigation
        // bars; the shared screens pad themselves by these insets.
        container.setOnApplyWindowInsetsListener { _, insets ->
            val changed = safeArea(insets)
            if (changed != safeArea) {
                safeArea = changed
                container.post {
                    if (!isFinishing && !isDestroyed) updateViewport()
                }
            }
            insets
        }
    }

    private fun safeArea(insets: WindowInsets): SafeArea =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            val covered = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout())
            SafeArea(covered.top, covered.right, covered.bottom, covered.left)
        } else {
            @Suppress("DEPRECATION")
            SafeArea(
                insets.systemWindowInsetTop,
                insets.systemWindowInsetRight,
                insets.systemWindowInsetBottom,
                insets.systemWindowInsetLeft,
            )
        }

    private fun updateViewport() {
        val width = container.width
        val height = container.height

        if (width > 0 && height > 0) {
            val density = host.density
            if (session == 0L) {
                session =
                    nativeCreate(
                        host,
                        width / density,
                        height / density,
                        intent.getStringExtra("screen") ?: "counter",
                        File(filesDir, "gallery-assets").path,
                    )
            }
            mountRoot(
                nativeResize(
                    session,
                    width / density,
                    height / density,
                    safeArea.top / density,
                    safeArea.right / density,
                    safeArea.bottom / density,
                    safeArea.left / density,
                ),
            )
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

    /** Lays the screen out in a viewport and safe area, in dp; returns its root. */
    private external fun nativeResize(
        session: Long,
        width: Float,
        height: Float,
        top: Float,
        right: Float,
        bottom: Float,
        left: Float,
    ): Long

    private external fun nativeDestroy(session: Long)
}
