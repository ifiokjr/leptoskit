package dev.leptoskit.plugins.sharing

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.os.Looper
import dev.leptoskit.bridge.NativeActivityResultHost
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Job
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import java.nio.ByteBuffer

/** Shares through Android's chooser. An empty reply is never a delivery receipt. */
class SharingPlugin(
    context: Context,
) : NativePlugin {
    override val name = "sharing"
    override val methods = intArrayOf(0)
    private val host = context as? NativeActivityResultHost
    private var job: Job? = null
    private var busy = false
    private var closed = false

    override fun close() {
        closed = true
        job?.cancel()
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Sharing requires the owning UI thread" }

        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method != 0) throw PluginException(PluginException.UNAVAILABLE)

        if (busy) throw PluginException(PluginException.BUSY)
        val content = ShareContent.decode(payload)
        val host = host ?: throw PluginException(PluginException.UNAVAILABLE)
        currentCoroutineContext().ensureActive()
        busy = true
        job = currentCoroutineContext()[Job]
        try {
            val intent =
                Intent(Intent.ACTION_SEND).apply {
                    type = "text/plain"
                    putExtra(Intent.EXTRA_TEXT, content.value)
                }
            val result = host.launchPluginActivity(Intent.createChooser(intent, null))
            currentCoroutineContext().ensureActive()

            if (closed) throw PluginException(PluginException.CANCELLED)
            // Android chooser result codes do not prove target selection or
            // delivery. RESULT_CANCELED also occurs after a valid handoff.
            if (result.resultCode != Activity.RESULT_OK && result.resultCode != Activity.RESULT_CANCELED) {
                throw PluginException(PluginException.NATIVE)
            }

            return byteArrayOf()

        } finally {
            busy = false
            job = null
        }
    }
}
