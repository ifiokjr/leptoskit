package dev.leptoskit.plugins.screencapture

import android.view.Window
import android.view.WindowManager
import dev.leptoskit.bridge.PluginException
import java.util.WeakHashMap

/** UI-thread only. All plugin instances sharing a window share its first baseline. */
internal object SecureWindowLeases {
    private data class Protection(
        val original: Boolean,
        var owners: Int,
    )

    private val windows = WeakHashMap<Window, Protection>()

    fun acquire(window: Window) {
        val existing = windows[window]
        if (existing != null) {
            if (existing.owners >= 64) throw PluginException(PluginException.BUSY)
            existing.owners += 1
            return
        }
        val original = window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE != 0
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        windows[window] = Protection(original, 1)
    }

    fun release(window: Window) {
        val protection = checkNotNull(windows[window])
        protection.owners -= 1
        if (protection.owners != 0) return
        windows.remove(window)
        // Restore only this bit; unrelated window flags may have changed meanwhile.
        if (protection.original) {
            window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        } else {
            window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
        }
    }
}
