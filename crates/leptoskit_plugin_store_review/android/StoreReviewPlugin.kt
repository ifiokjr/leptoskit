package dev.leptoskit.plugins.storereview

import android.app.Activity
import android.app.Application
import android.content.Context
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import com.google.android.play.core.review.ReviewManager
import com.google.android.play.core.review.ReviewManagerFactory
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
class StoreReviewPlugin internal constructor(
    context: Context,
    private val manager: ReviewManager,
) : NativePlugin {
    constructor(context: Context) : this(context, ReviewManagerFactory.create(context.applicationContext))

    override val name = "store-review"
    override val methods = intArrayOf(0, 1)
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

        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)

        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
        currentCoroutineContext().ensureActive()

        if (method == 0) {
            val value =
                if (pending != null) {
                    3
                } else if (activeOwner() != null) {
                    1
                } else {
                    2
                }

            return byteArrayOf(value.toByte(), 0, 0, 0)
        }

        if (pending != null) throw PluginException(PluginException.BUSY)

        if (activeOwner() == null) throw PluginException(PluginException.UNAVAILABLE)
        return suspendCancellableCoroutine { continuation ->
            val operation = Operation(continuation)
            pending = operation
            application?.registerActivityLifecycleCallbacks(lifecycle)
            observing = true
            continuation.invokeOnCancellation {
                executor.execute {
                    if (pending === operation) {

                        operation.cancelled = true
                        operation.continuation = null
                    }
                }
            }
            prepare(operation)
        }
    }

    private fun prepare(operation: Operation) {
        try {
            manager.requestReviewFlow().addOnCompleteListener(executor) { task ->

                if (pending !== operation) return@addOnCompleteListener
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
                        try {
                            manager.launchReviewFlow(owner, task.result).addOnCompleteListener(executor) { flow ->
                                val error =
                                    when {
                                        closed || operation.cancelled || flow.isCanceled -> {
                                            PluginException(PluginException.CANCELLED)
                                        }

                                        !flow.isSuccessful -> {
                                            PluginException(PluginException.NATIVE, flow.exception)
                                        }

                                        else -> {
                                            null
                                        }
                                    }
                                finish(operation, error)
                            }
                        } catch (error: RuntimeException) {
                            finish(operation, PluginException(PluginException.NATIVE, error))
                        }
                    }
                }
            }
        } catch (error: RuntimeException) {
            finish(operation, PluginException(PluginException.NATIVE, error))
        }
    }

    private fun activeOwner(): Activity? =
        activity.get()?.takeIf {
            !it.isFinishing && !it.isDestroyed &&
                it.window?.decorView?.isAttachedToWindow == true && it.hasWindowFocus()
        }

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

        if (continuation?.isActive == true) {
            if (error != null) {

                continuation.resumeWithException(error)
            } else {
                continuation.resume(byteArrayOf(1, 0, 0, 0))
            }
        }
    }

    override fun close() {
        checkMain()

        if (closed) return
        closed = true
        stopObserving()
        pending?.let {
            it.cancelled = true
            val continuation = it.continuation
            it.continuation = null

            if (continuation?.isActive == true) {
                continuation.resumeWithException(PluginException(PluginException.CANCELLED))
            }
        }
    }

    private fun stopObserving() {
        if (observing) application?.unregisterActivityLifecycleCallbacks(lifecycle)
        observing = false
    }

    private fun checkMain() {
        check(Looper.myLooper() == Looper.getMainLooper())
    }
}
