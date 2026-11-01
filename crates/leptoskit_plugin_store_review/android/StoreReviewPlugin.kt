package dev.leptoskit.plugins.storereview

import android.app.Activity
import android.app.Application
import android.content.Context
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import com.google.android.play.core.review.ReviewInfo
import com.google.android.play.core.review.ReviewManager
import com.google.android.play.core.review.ReviewManagerFactory
import com.google.android.play.core.tasks.Task
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.suspendCancellableCoroutine
import java.lang.ref.WeakReference
import java.nio.ByteBuffer
import java.util.concurrent.Executor
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/** Google Play owns the UI. A completed flow does not reveal whether a dialog or review occurred. */
class StoreReviewPlugin(
    context: Context,
    private val manager: ReviewManager = ReviewManagerFactory.create(context.applicationContext),
) : NativePlugin {
    override val name = "store-review"
    override val methods = intArrayOf(AVAILABILITY, REQUEST)
    private val activity = WeakReference(context as? Activity)
    private val application = (context as? Activity)?.application
    private val handler = Handler(Looper.getMainLooper())
    private val executor = Executor { handler.post(it) }
    private var closed = false
    private var pending: Operation? = null
    private var observing = false

    private class Operation(
        var continuation: CancellableContinuation<ByteArray>?,
    ) {
        var cancelled = false
    }

    private val lifecycle =
        object : Application.ActivityLifecycleCallbacks {
            override fun onActivityDestroyed(owner: Activity) {
                if (owner === activity.get()) close()
            }

            override fun onActivityCreated(
                owner: Activity,
                state: Bundle?,
            ) {}

            override fun onActivityStarted(owner: Activity) {}

            override fun onActivityResumed(owner: Activity) {}

            override fun onActivityPaused(owner: Activity) {}

            override fun onActivityStopped(owner: Activity) {}

            override fun onActivitySaveInstanceState(
                owner: Activity,
                state: Bundle,
            ) {}
        }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        checkMain()
        requireRequest(method, payload)

        if (method == AVAILABILITY) return availability()

        if (pending != null) throw PluginException(PluginException.BUSY)

        if (activeOwner() == null) throw PluginException(PluginException.UNAVAILABLE)

        return startFlow()
    }

    /** Rejects a call this plugin can never answer, before any store work. */
    private suspend fun requireRequest(
        method: Int,
        payload: ByteBuffer,
    ) {
        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)

        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        currentCoroutineContext().ensureActive()
    }

    /** Reports whether this app could attempt a flow right now. */
    private fun availability(): ByteArray {
        val status =
            when {
                pending != null -> STATUS_BUSY
                activeOwner() != null -> STATUS_READY
                else -> STATUS_INACTIVE
            }

        return byteArrayOf(status.toByte(), 0, 0, 0)
    }

    /** Starts the flow for the active owner and keeps the slot until it ends. */
    private suspend fun startFlow(): ByteArray =
        suspendCancellableCoroutine { continuation ->
            val operation = Operation(continuation)

            pending = operation
            application?.registerActivityLifecycleCallbacks(lifecycle)
            observing = true
            continuation.invokeOnCancellation { cancel(operation) }
            prepare(operation)
        }

    /** Marks the operation cancelled, leaving the store flow to finish alone. */
    private fun cancel(operation: Operation) {
        executor.execute { markCancelled(operation) }
    }

    /** Runs on the main queue, where the pending slot is decided. */
    private fun markCancelled(operation: Operation) {
        if (pending !== operation) return

        operation.cancelled = true
        operation.continuation = null
    }

    /** Asks the store to prepare the flow, translating a synchronous refusal. */
    private fun prepare(operation: Operation) {
        try {
            manager.requestReviewFlow().addOnCompleteListener(executor) { task ->
                onPrepared(operation, task)
            }
        } catch (error: RuntimeException) {
            finish(operation, PluginException(PluginException.NATIVE, error))
        }
    }

    /** Maps the prepared flow to a failure, or presents it to the owner. */
    private fun onPrepared(
        operation: Operation,
        task: Task<ReviewInfo>,
    ) {
        if (pending !== operation) return

        val owner = activeOwner()

        when {
            closed || operation.cancelled || operation.continuation?.isActive != true -> {
                finish(operation, PluginException(PluginException.CANCELLED))
            }

            owner == null -> {
                finish(operation, PluginException(PluginException.UNAVAILABLE))
            }

            task.isCanceled -> {
                finish(operation, PluginException(PluginException.CANCELLED))
            }

            !task.isSuccessful -> {
                finish(operation, PluginException(PluginException.UNAVAILABLE, task.exception))
            }

            else -> {
                present(operation, owner, task.result)
            }
        }
    }

    /** Presents the store-owned UI, translating a synchronous refusal. */
    private fun present(
        operation: Operation,
        owner: Activity,
        info: ReviewInfo,
    ) {
        try {
            manager.launchReviewFlow(owner, info).addOnCompleteListener(executor) { flow ->
                finish(operation, flowFailure(operation, flow))
            }
        } catch (error: RuntimeException) {
            finish(operation, PluginException(PluginException.NATIVE, error))
        }
    }

    /** The failure a finished store flow reports, or null when it completed. */
    private fun flowFailure(
        operation: Operation,
        flow: Task<Void>,
    ): PluginException? =
        when {
            closed || operation.cancelled || flow.isCanceled -> PluginException(PluginException.CANCELLED)
            !flow.isSuccessful -> PluginException(PluginException.NATIVE, flow.exception)
            else -> null
        }

    private fun activeOwner(): Activity? =
        activity.get()?.takeIf {
            !it.isFinishing && !it.isDestroyed &&
                it.window?.decorView?.isAttachedToWindow == true && it.hasWindowFocus()
        }

    /** Answers the operation once, while it still owns the slot. */
    private fun finish(
        operation: Operation,
        error: PluginException?,
    ) {
        checkMain()

        if (pending !== operation) return

        pending = null
        stopObserving()

        val continuation = operation.continuation

        operation.continuation = null

        if (continuation?.isActive != true) return

        if (error != null) {
            continuation.resumeWithException(error)
        } else {
            continuation.resume(byteArrayOf(ACCEPTED.toByte(), 0, 0, 0))
        }
    }

    override fun close() {
        checkMain()

        if (closed) return

        closed = true
        stopObserving()

        val operation = pending ?: return

        operation.cancelled = true

        val continuation = operation.continuation

        operation.continuation = null

        if (continuation?.isActive == true) {
            continuation.resumeWithException(PluginException(PluginException.CANCELLED))
        }
    }

    private fun stopObserving() {
        if (observing) application?.unregisterActivityLifecycleCallbacks(lifecycle)

        observing = false
    }

    private fun checkMain() {
        check(Looper.myLooper() == Looper.getMainLooper())
    }

    private companion object {
        /** Method ids the plugin exposes. */
        const val AVAILABILITY = 0
        const val REQUEST = 1

        /** Availability statuses from the wire format. */
        const val STATUS_READY = 1
        const val STATUS_INACTIVE = 2
        const val STATUS_BUSY = 3

        /** The request acceptance the wire format defines. */
        const val ACCEPTED = 1
    }
}
