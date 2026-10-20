package dev.leptoskit.bridge

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.suspendCancellableCoroutine
import java.util.concurrent.atomic.AtomicInteger

/** A result from a host-owned external activity; callers interpret its documented result codes. */
data class NativeActivityResult(
    val resultCode: Int,
    val data: Intent?,
)

/** Explicit presentation capability supplied by the owning Activity, never a global lookup. */
interface NativeActivityResultHost {
    suspend fun launchPluginActivity(intent: Intent): NativeActivityResult
}

/**
 * Owns one external presentation until its result arrives or this host is destroyed.
 * Cancellation retires the caller but retains the presentation slot: an external
 * Activity cannot generally be dismissed by cancelling a coroutine.
 * All public methods require the main thread.
 */
class NativeActivityResultBroker(
    private val activity: Activity,
) {
    private class Pending(
        val code: Int,
        var continuation: CancellableContinuation<NativeActivityResult>?,
    )

    private var pending: Pending? = null
    private var closed = false
    private var active = false
    private val main = Handler(Looper.getMainLooper())

    @Suppress("DEPRECATION")
    suspend fun launch(intent: Intent): NativeActivityResult {
        checkMain()
        if (closed || activity.isDestroyed || activity.isFinishing) throw PluginException(PluginException.CANCELLED)
        if (!active || pending != null) throw PluginException(PluginException.BUSY)
        val code = nextCode.getAndUpdate { if (it <= LAST_CODE) it + 1 else it }
        if (code > LAST_CODE) throw PluginException(PluginException.EXHAUSTED)
        return suspendCancellableCoroutine { continuation ->
            val request = Pending(code, continuation)
            pending = request
            continuation.invokeOnCancellation {
                main.post {
                    if (pending === request) request.continuation = null
                }
            }
            if (!continuation.isActive) {
                pending = null
            } else {
                try {
                    activity.startActivityForResult(Intent(intent), code)
                } catch (error: ActivityNotFoundException) {
                    pending = null
                    continuation.resumeWith(Result.failure(PluginException(PluginException.UNAVAILABLE, error)))
                } catch (error: SecurityException) {
                    pending = null
                    continuation.resumeWith(Result.failure(PluginException(PluginException.PERMISSION_DENIED, error)))
                } catch (error: IllegalArgumentException) {
                    pending = null
                    continuation.resumeWith(Result.failure(PluginException(PluginException.INVALID_ARGUMENT, error)))
                }
            }
        }
    }

    /** Returns true for an owned result, including a retired caller's late result. */
    fun onResult(
        requestCode: Int,
        resultCode: Int,
        data: Intent?,
    ): Boolean {
        checkMain()
        val request = pending ?: return false
        if (request.code != requestCode) return false
        pending = null
        val continuation = request.continuation ?: return true
        if (continuation.isActive) continuation.resumeWith(Result.success(NativeActivityResult(resultCode, data)))
        return true
    }

    /** Restore before guest execution so an earlier process's results cannot match new calls. */
    fun restoreState(state: Bundle?) {
        checkMain()
        val restored = state?.getInt(STATE_KEY, FIRST_CODE) ?: FIRST_CODE
        require(restored in FIRST_CODE..LAST_CODE + 1) { "Invalid activity-result request-code state" }
        nextCode.updateAndGet { current -> maxOf(current, restored) }
    }

    fun saveState(state: Bundle) {
        checkMain()
        active = false
        state.putInt(STATE_KEY, nextCode.get())
    }

    /** Pausing prevents new launches; it must not cancel the external Activity's pending result. */
    fun setActive(active: Boolean) {
        checkMain()
        this.active = active && !closed
    }

    fun close() {
        checkMain()
        closed = true
        active = false
        pending?.continuation?.cancel()
        pending = null
    }

    private fun checkMain() {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Activity results require the UI thread" }
    }

    private companion object {
        // Disjoint from NativePermissionBroker's 0x4C00..0xFFFF range.
        const val FIRST_CODE = 0x1000
        const val LAST_CODE = 0x4BFF
        const val STATE_KEY = "leptoskit.activity-results.next-request-code"
        val nextCode = AtomicInteger(FIRST_CODE)
    }
}
