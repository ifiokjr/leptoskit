package dev.leptoskit.plugins.network

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.os.Handler
import android.os.Looper
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.suspendCancellableCoroutine
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

class NetworkPlugin(
    context: Context,
) : NativePlugin {
    override val name = "network"
    override val methods = intArrayOf(0, 1)
    private val manager = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
    private val handler = Handler(Looper.getMainLooper())
    private var waiting: CancellableContinuation<ByteArray>? = null
    private var callback: ConnectivityManager.NetworkCallback? = null
    private var closed = false

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)

        if (method == 0) {
            if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

            return snapshot()
        }

        val previous = previous(payload)

        if (waiting != null) throw PluginException(PluginException.BUSY)
        val connectivity = manager ?: throw PluginException(PluginException.UNAVAILABLE)
        return suspendCancellableCoroutine { continuation ->
            waiting = continuation
            val observer =
                object : ConnectivityManager.NetworkCallback() {
                    override fun onAvailable(network: Network) = changed()

                    override fun onLost(network: Network) = changed()

                    override fun onCapabilitiesChanged(
                        network: Network,
                        capabilities: NetworkCapabilities,
                    ) = changed()

                    private fun changed() {
                        handler.post { update(continuation, previous) }
                    }
                }
            continuation.invokeOnCancellation {
                if (Looper.myLooper() == handler.looper) {
                    if (waiting === continuation) cleanup()
                } else {
                    handler.post {
                        if (waiting === continuation) cleanup()
                    }
                }
            }
            try {
                connectivity.registerDefaultNetworkCallback(observer)
                callback = observer
                // Register first, then sample, so changes cannot fall between them.
                update(continuation, previous)
            } catch (error: SecurityException) {

                cleanup()

                if (continuation.isActive) continuation.resumeWithException(PluginException(PluginException.PERMISSION_DENIED, error))
            } catch (error: RuntimeException) {
                cleanup()

                if (continuation.isActive) continuation.resumeWithException(PluginException(PluginException.NATIVE, error))
            }
        }
    }

    override fun close() {
        closed = true
        val continuation = waiting
        cleanup()
        continuation?.cancel()
    }

    private fun previous(payload: ByteBuffer): ByteArray {
        if (payload.remaining() != 32) throw PluginException(PluginException.INVALID_ARGUMENT)
        val bytes = ByteArray(32)
        payload.get(bytes)
        val input = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        val fields = IntArray(8) { input.int }

        if (fields[0] !in 0..7 || (1..4).any { fields[it] !in 0..2 } || fields[7] !in 0..4) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        return bytes
    }

    private fun update(
        continuation: CancellableContinuation<ByteArray>,
        previous: ByteArray,
    ) {
        if (waiting !== continuation || !continuation.isActive) return
        try {
            val current = snapshot()

            if (!previous.contentEquals(current)) {
                cleanup()
                continuation.resume(current)
            }
        } catch (error: PluginException) {
            cleanup()

            continuation.resumeWithException(error)
        }
    }

    private fun cleanup() {
        val observer = callback
        callback = null
        waiting = null

        if (observer != null) manager?.unregisterNetworkCallback(observer)
    }

    private fun snapshot(): ByteArray {
        val connectivity = manager ?: throw PluginException(PluginException.UNAVAILABLE)
        val capabilities =
            try {
                connectivity.activeNetwork?.let(connectivity::getNetworkCapabilities)
            } catch (error: SecurityException) {
                throw PluginException(PluginException.PERMISSION_DENIED, error)
            }
        val connection =
            when {

                capabilities == null -> 1
                capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN) -> 6
                capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> 2
                capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) -> 3
                capabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) -> 4
                capabilities.hasTransport(NetworkCapabilities.TRANSPORT_BLUETOOTH) -> 5
                else -> 7
            }
        val validated =
            capabilities != null && capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) &&
                capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
        val expensive =
            if (capabilities ==
                null
            ) {
                0
            } else if (capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED)) {
                1
            } else {
                2
            }

        // Restrict-background state is app-specific, not a general low-data-mode flag.
        return ByteBuffer
            .allocate(32)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(connection)
            .putInt(if (capabilities == null) 1 else 2)
            .putInt(if (validated) 2 else 1)
            .putInt(expensive)
            .putInt(0)
            .putInt(-1)
            .putInt(-1)
            .putInt(0)
            .array()
    }
}
