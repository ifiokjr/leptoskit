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

/** Reads the OS connectivity state and waits for it to change. */
class NetworkPlugin(
    context: Context,
) : NativePlugin {
    override val name = "network"
    override val methods = intArrayOf(SNAPSHOT, WAIT_FOR_CHANGE)
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

        return when (method) {
            SNAPSHOT -> currentSnapshot(payload)
            WAIT_FOR_CHANGE -> awaitChange(payload)
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }
    }

    override fun close() {
        closed = true
        val continuation = waiting
        cleanup()
        continuation?.cancel()
    }

    /** Answers one reading; the snapshot call takes no arguments. */
    private fun currentSnapshot(payload: ByteBuffer): ByteArray {
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        return snapshot()
    }

    /** Suspends until the OS reports a reading other than the caller's last one. */
    private suspend fun awaitChange(payload: ByteBuffer): ByteArray {
        val previous = previous(payload)

        if (waiting != null) throw PluginException(PluginException.BUSY)

        val connectivity = manager ?: throw PluginException(PluginException.UNAVAILABLE)

        return suspendCancellableCoroutine { continuation ->
            waiting = continuation
            continuation.invokeOnCancellation { release(continuation) }
            val observer = changeObserver(continuation, previous)

            register(connectivity, observer, continuation, previous)
        }
    }

    /** Repeats the comparison on the main looper for every relevant OS event. */
    private fun changeObserver(
        continuation: CancellableContinuation<ByteArray>,
        previous: ByteArray,
    ): ConnectivityManager.NetworkCallback =
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

    /** Registers the callback, then samples, so no change can fall between the two. */
    private fun register(
        connectivity: ConnectivityManager,
        observer: ConnectivityManager.NetworkCallback,
        continuation: CancellableContinuation<ByteArray>,
        previous: ByteArray,
    ) {
        try {
            connectivity.registerDefaultNetworkCallback(observer)
            callback = observer
            update(continuation, previous)
        } catch (error: SecurityException) {
            fail(continuation, PluginException.PERMISSION_DENIED, error)
        } catch (error: RuntimeException) {
            fail(continuation, PluginException.NATIVE, error)
        }
    }

    /** Releases a failed registration and tells the caller why it failed. */
    private fun fail(
        continuation: CancellableContinuation<ByteArray>,
        code: Int,
        error: Throwable,
    ) {
        cleanup()

        if (continuation.isActive) continuation.resumeWithException(PluginException(code, error))
    }

    /** Drops the observer for a cancelled caller, on whichever thread cancelled it. */
    private fun release(continuation: CancellableContinuation<ByteArray>) {
        if (Looper.myLooper() == handler.looper) {
            releaseIfWaiting(continuation)
        } else {
            handler.post { releaseIfWaiting(continuation) }
        }
    }

    private fun releaseIfWaiting(continuation: CancellableContinuation<ByteArray>) {
        if (waiting === continuation) cleanup()
    }

    /**
     * Resumes the caller only when the reading differs. A caller can cancel
     * between the post and this call, so the identity check comes first.
     */
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

    /** Decodes the caller's last reading and refuses fields the wire cannot carry. */
    private fun previous(payload: ByteBuffer): ByteArray {
        if (payload.remaining() != SNAPSHOT_BYTES) throw PluginException(PluginException.INVALID_ARGUMENT)

        val bytes = ByteArray(SNAPSHOT_BYTES)
        payload.get(bytes)
        val input = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        val fields = IntArray(FIELD_COUNT) { input.int }

        if (!isValid(fields)) throw PluginException(PluginException.INVALID_ARGUMENT)

        return bytes
    }

    /** Mirrors the wire contract: known transport, tri-state flags, known estimate class. */
    private fun isValid(fields: IntArray): Boolean =
        fields[CONNECTION_FIELD] in 0..LAST_CONNECTION &&
            (FIRST_FLAG_FIELD..LAST_FLAG_FIELD).all { fields[it] in 0..FLAG_TRUE } &&
            fields[EFFECTIVE_TYPE_FIELD] in 0..LAST_EFFECTIVE_TYPE

    /** Encodes a reading; anything the OS does not report stays unknown. */
    private fun snapshot(): ByteArray {
        val connectivity = manager ?: throw PluginException(PluginException.UNAVAILABLE)
        val capabilities = activeCapabilities(connectivity)
        val connected = if (capabilities == null) FLAG_FALSE else FLAG_TRUE

        return ByteBuffer
            .allocate(SNAPSHOT_BYTES)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(connectionType(capabilities))
            .putInt(connected)
            .putInt(if (validated(capabilities)) FLAG_TRUE else FLAG_FALSE)
            .putInt(costType(capabilities))
            // Restrict-background state is app-specific, not a general low-data-mode flag.
            .putInt(FLAG_UNKNOWN)
            .putInt(UNKNOWN_ESTIMATE)
            .putInt(UNKNOWN_ESTIMATE)
            .putInt(EFFECTIVE_UNKNOWN)
            .array()
    }

    /** Reads the active network's capabilities, if this caller may see them. */
    private fun activeCapabilities(connectivity: ConnectivityManager): NetworkCapabilities? =
        try {
            connectivity.activeNetwork?.let(connectivity::getNetworkCapabilities)
        } catch (error: SecurityException) {
            throw PluginException(PluginException.PERMISSION_DENIED, error)
        }

    /** Maps the reported transport to the wire code, preferring the overlay transport. */
    private fun connectionType(capabilities: NetworkCapabilities?): Int =
        when {
            capabilities == null -> NO_CONNECTION
            capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN) -> VPN
            capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> WIFI
            capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) -> CELLULAR
            capabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) -> ETHERNET
            capabilities.hasTransport(NetworkCapabilities.TRANSPORT_BLUETOOTH) -> BLUETOOTH
            else -> OTHER
        }

    /** True only when the OS itself has validated internet access. */
    private fun validated(capabilities: NetworkCapabilities?): Boolean =
        capabilities != null &&
            capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) &&
            capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)

    /** Metered state as a tri-state; a present network always reports one of the two. */
    private fun costType(capabilities: NetworkCapabilities?): Int =
        when {
            capabilities == null -> FLAG_UNKNOWN
            capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED) -> FLAG_FALSE
            else -> FLAG_TRUE
        }

    private companion object {
        /** Answers the current reading without blocking. */
        const val SNAPSHOT = 0

        /** Waits until the OS reports a different reading. */
        const val WAIT_FOR_CHANGE = 1

        /** Eight little-endian u32 fields, as the wire snapshot defines them. */
        const val SNAPSHOT_BYTES = 32
        const val FIELD_COUNT = 8

        /** Field offsets inside the reading. */
        const val CONNECTION_FIELD = 0
        const val FIRST_FLAG_FIELD = 1
        const val LAST_FLAG_FIELD = 4
        const val EFFECTIVE_TYPE_FIELD = 7

        /** The highest code each field may carry. */
        const val LAST_CONNECTION = 7
        const val LAST_EFFECTIVE_TYPE = 4

        /** Tri-state encodings shared by every boolean field. */
        const val FLAG_UNKNOWN = 0
        const val FLAG_FALSE = 1
        const val FLAG_TRUE = 2

        /** Numeric estimates use the reserved maximum for unknown. */
        const val UNKNOWN_ESTIMATE = -1

        /** No estimate of the browser performance class. */
        const val EFFECTIVE_UNKNOWN = 0

        /** Connection codes, mapped from the transports Android reports. */
        const val NO_CONNECTION = 1
        const val WIFI = 2
        const val CELLULAR = 3
        const val ETHERNET = 4
        const val BLUETOOTH = 5
        const val VPN = 6
        const val OTHER = 7
    }
}
