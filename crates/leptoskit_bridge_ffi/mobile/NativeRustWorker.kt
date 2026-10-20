package dev.leptoskit.mobile

import dev.leptoskit.bridge.NativeRequest
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.util.concurrent.atomic.AtomicLong

/**
 * Owns Rust work independently of the app. Close only after its coroutine has
 * finished: cancellation cannot interrupt synchronous native I/O.
 */
internal class NativeRustWorker private constructor(
    handle: Long,
) : AutoCloseable {
    private val worker = AtomicLong(handle)
    private val reply = AtomicLong(0)
    private val cancellationLock = Any()
    private var cancellation = NativeApp.workerCancellation(handle)

    companion object {
        fun retain(
            app: Long,
            request: NativeRequest,
        ): NativeRustWorker? =
            request.withNativeOwner { owner ->
                NativeApp
                    .worker(app, request.plugin, request.method, owner)
                    .takeIf { it != 0L }
                    ?.let(::NativeRustWorker)
            }
    }

    suspend fun execute() {
        withContext(Dispatchers.IO) {
            val handle = worker.getAndSet(0)
            check(handle != 0L) { "Worker already consumed" }
            // Store before returning from IO: prompt coroutine cancellation can
            // discard a withContext return value, but completion still closes us.
            reply.set(NativeApp.executeWorker(handle))
        }
    }

    fun complete(
        bridge: Long,
        id: Long,
    ): Int = NativeApp.completeWorker(bridge, id, reply.getAndSet(0))

    /** Called on the host thread before cancelling the coroutine. */
    fun cancel() {
        synchronized(cancellationLock) {
            NativeApp.cancelWorker(cancellation)
        }
    }

    override fun close() {
        NativeApp.releaseWorker(worker.getAndSet(0))
        NativeApp.releaseWorkerReply(reply.getAndSet(0))
        synchronized(cancellationLock) {
            NativeApp.releaseWorkerCancellation(cancellation)
            cancellation = 0
        }
    }
}
