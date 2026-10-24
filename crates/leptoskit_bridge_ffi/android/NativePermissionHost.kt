package dev.leptoskit.bridge

import android.app.Activity
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.suspendCancellableCoroutine
import java.util.concurrent.atomic.AtomicInteger

/** Optional host capability for native plugins that request Android runtime permissions. */
interface NativePermissionHost {
    suspend fun requestNativePermissions(permissions: Array<String>): IntArray
}

/**
 * One OS dialog at a time. Canceled requests retain their slot until the OS replies;
 * reusing the request code earlier could deliver an old result to a new caller.
 * All public methods must be called on the activity's main thread.
 */
class NativePermissionBroker(
    private val activity: Activity,
) {
    private class Pending(
        val requestCode: Int,
        val permissions: Array<String>,
        var continuation: CancellableContinuation<IntArray>?,
    )

    private var pending: Pending? = null
    private var closed = false
    private var active = false
    private val main = Handler(Looper.getMainLooper())

    suspend fun request(permissions: Array<String>): IntArray {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Permission requests require the UI thread" }

        requireRequestable(permissions)
        val requestCode = nextRequestCode.getAndUpdate { if (it <= LAST_REQUEST_CODE) it + 1 else it }

        if (requestCode > LAST_REQUEST_CODE) throw PluginException(PluginException.EXHAUSTED)

        return suspendCancellableCoroutine { continuation ->
            begin(Pending(requestCode, permissions.copyOf(), continuation))
        }
    }

    /** Preserve rejection precedence before allocating an OS request code or showing a dialog. */
    private fun requireRequestable(permissions: Array<String>) {
        if (closed || activity.isDestroyed || activity.isFinishing) throw PluginException(PluginException.CANCELLED)

        if (pending != null) throw PluginException(PluginException.BUSY)

        requireValidPermissions(permissions)

        if (!active) throw PluginException(PluginException.BUSY)
    }

    private fun requireValidPermissions(permissions: Array<String>) {
        if (permissions.isEmpty() || permissions.size > MAX_PERMISSIONS || permissions.any { it.isBlank() } ||
            permissions.distinct().size != permissions.size
        ) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
    }

    private fun begin(request: Pending) {
        val continuation = checkNotNull(request.continuation)
        pending = request
        continuation.invokeOnCancellation { main.post { retire(request) } }

        if (!continuation.isActive) {
            pending = null

            return
        }

        present(request, continuation)
    }

    private fun retire(request: Pending) {
        if (pending === request) request.continuation = null
    }

    private fun present(
        request: Pending,
        continuation: CancellableContinuation<IntArray>,
    ) {
        try {
            activity.requestPermissions(request.permissions, request.requestCode)
        } catch (error: SecurityException) {
            fail(continuation, PluginException.PERMISSION_DENIED, error)
        } catch (error: IllegalArgumentException) {
            fail(continuation, PluginException.INVALID_ARGUMENT, error)
        }
    }

    private fun fail(
        continuation: CancellableContinuation<IntArray>,
        code: Int,
        error: Exception,
    ) {
        pending = null
        continuation.resumeWith(Result.failure(PluginException(code, error)))
    }

    /** Returns true if this callback belongs to this broker. */
    fun onResult(
        requestCode: Int,
        permissions: Array<out String>,
        results: IntArray,
    ): Boolean {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Permission results require the UI thread" }

        val request = pending ?: return false

        if (requestCode != request.requestCode) return false
        pending = null

        val continuation = request.continuation ?: return true

        if (!continuation.isActive) return true
        val result =
            when {
                results.isEmpty() -> {
                    Result.failure(PluginException(PluginException.CANCELLED))
                }

                !request.permissions.contentEquals(permissions) || results.size != request.permissions.size -> {
                    Result.failure(PluginException(PluginException.INVALID_ARGUMENT))
                }

                else -> {
                    Result.success(results.copyOf())
                }
            }
        continuation.resumeWith(result)

        return true
    }

    /** Restore before starting the guest, so old Activity replies cannot match new requests. */
    fun restoreState(state: Bundle?) {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Permission restoration requires the UI thread" }
        val restored = state?.getInt(STATE_KEY, FIRST_REQUEST_CODE) ?: FIRST_REQUEST_CODE

        if (restored !in FIRST_REQUEST_CODE..LAST_REQUEST_CODE + 1) {
            throw IllegalArgumentException("Invalid permission request-code state")
        }

        nextRequestCode.updateAndGet { current -> maxOf(current, restored) }
    }

    /** Preserve the allocation watermark across process death as well as Activity recreation. */
    fun saveState(state: Bundle) {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Permission state saving requires the UI thread" }
        active = false
        state.putInt(STATE_KEY, nextRequestCode.get())
    }

    /** Match Activity onResume/onPause; never launch after its state has been saved. */
    fun setActive(active: Boolean) {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Permission lifecycle requires the UI thread" }
        this.active = active && !closed
    }

    fun close() {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Permission cleanup requires the UI thread" }
        closed = true
        pending?.continuation?.cancel()
        pending = null
    }

    private companion object {
        private const val MAX_PERMISSIONS = 16
        const val FIRST_REQUEST_CODE = 0x4C00
        const val LAST_REQUEST_CODE = 0xFFFF
        const val STATE_KEY = "leptoskit.permissions.next-request-code"
        val nextRequestCode = AtomicInteger(FIRST_REQUEST_CODE)
    }
}
