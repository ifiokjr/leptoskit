package dev.leptoskit.plugins.webauth

import android.app.Activity
import android.content.Context
import android.net.Uri
import android.os.Looper
import androidx.browser.auth.AuthTabIntent
import androidx.browser.customtabs.CustomTabsClient
import dev.leptoskit.bridge.NativeActivityResultHost
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import java.nio.ByteBuffer

/**
 * Sign-in in the system browser's Auth Tab, never a web view in the app. Auth Tab
 * returns the redirect to this app's private-use scheme as the launching activity's
 * result, so no other app can receive it. Browsers without Auth Tab answer UNAVAILABLE.
 */
class WebAuthPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "web-auth"
    override val methods = intArrayOf(0, 1)
    private val host = context as? NativeActivityResultHost
    private var begun: Pair<Int, String>? = null
    private var next = 0
    private var busy = false
    private var closed = false

    override fun close() {
        closed = true
        begun = null
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Web auth requires the owning UI thread" }

        if (closed) throw PluginException(PluginException.CANCELLED)

        return when (method) {
            0 -> begin(payload)
            1 -> authenticate(WebAuthRequest.decode(payload))
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }
    }

    private fun begin(payload: ByteBuffer): ByteArray {
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        if (busy) throw PluginException(PluginException.BUSY)
        val returnTo = returnAddress(context.packageName) ?: throw PluginException(PluginException.UNAVAILABLE)
        next += 1
        begun = next to returnTo

        return encodeSession(next, returnTo)
    }

    private suspend fun authenticate(request: WebAuthRequest): ByteArray {
        if (busy) throw PluginException(PluginException.BUSY)
        val (session, returnTo) = begun ?: throw PluginException(PluginException.INVALID_CALL)

        if (session != request.session) throw PluginException(PluginException.INVALID_CALL)
        begun = null
        val host = host ?: throw PluginException(PluginException.UNAVAILABLE)
        val browser = CustomTabsClient.getPackageName(context, null) ?: throw PluginException(PluginException.UNAVAILABLE)

        if (!CustomTabsClient.isAuthTabSupported(context, browser)) throw PluginException(PluginException.UNAVAILABLE)
        currentCoroutineContext().ensureActive()

        busy = true
        try {
            val intent = AuthTabIntent.Builder().build().intent
            intent.setPackage(browser)
            intent.data = Uri.parse(request.url.toString())
            intent.putExtra(AuthTabIntent.EXTRA_REDIRECT_SCHEME, Uri.parse(returnTo).scheme)
            val result = host.launchPluginActivity(intent)
            currentCoroutineContext().ensureActive()

            if (closed) throw PluginException(PluginException.CANCELLED)
            val address = result.data?.data?.toString()

            return when {
                result.resultCode == Activity.RESULT_OK && address != null && cameBack(address, returnTo) -> {
                    address.toByteArray(Charsets.UTF_8)
                }

                result.resultCode == Activity.RESULT_CANCELED -> {
                    throw PluginException(PluginException.CANCELLED)
                }

                else -> {
                    throw PluginException(PluginException.NATIVE)
                }
            }
        } finally {
            busy = false
        }
    }
}
