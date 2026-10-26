package dev.leptoskit.mobile

import android.app.Activity
import android.content.Context
import android.os.Handler
import android.os.Looper
import android.util.Log
import dev.leptoskit.bridge.NativeBridge

/** Thread-only JNI boundary for `leptoskit dev`; each call may block for the server's wait. */
internal object NativeDevelopment {
    @JvmStatic external fun open(
        url: ByteArray,
        running: ByteArray,
    ): Long

    /** A new guest, or null when nothing changed within the server's wait. */
    @JvmStatic external fun next(handle: Long): ByteArray?

    @JvmStatic external fun destroy(handle: Long)
}

/**
 * Follows the guest `leptoskit dev` serves to the Activity it launched, on a
 * dedicated thread, and delivers each new guest on the main thread. Only debug
 * builds accept development guests: Rust refuses to open one in a release build.
 */
internal class NativeDevelopmentGuests(
    url: String,
    running: ByteArray,
    private val onGuest: (ByteArray) -> Unit,
) : AutoCloseable {
    companion object {
        /** The intent extra `leptoskit dev` launches the Activity with. */
        private const val EXTRA = "leptoskit.dev_guest"
        private const val TAG = "LeptosKit"
        private const val RETRY_MILLIS = 1000L

        /** The guest URL the Activity hosting [context] was launched with, if any. */
        fun url(context: Context): String? = (context as? Activity)?.intent?.getStringExtra(EXTRA)
    }

    private val main = Handler(Looper.getMainLooper())

    @Volatile private var closed = false

    init {
        Thread({ follow(url, running) }, "leptoskit-dev-guests").apply {
            isDaemon = true
            start()
        }
    }

    private fun follow(
        url: String,
        running: ByteArray,
    ) {
        val handle =
            try {
                NativeBridge.load()
                NativeDevelopment.open(url.toByteArray(Charsets.UTF_8), running)
            } catch (error: IllegalStateException) {
                Log.w(TAG, "leptoskit dev: ${error.message}")

                return
            }

        try {
            while (!closed) poll(handle)
        } finally {
            NativeDevelopment.destroy(handle)
        }
    }

    private fun poll(handle: Long) {
        try {
            val guest = NativeDevelopment.next(handle) ?: return
            main.post { if (!closed) onGuest(guest) }
        } catch (error: IllegalStateException) {
            Log.w(TAG, "leptoskit dev: ${error.message}")
            Thread.sleep(RETRY_MILLIS)
        }
    }

    /** Stops delivery; the thread exits after its current request returns. */
    override fun close() {
        closed = true
    }
}
