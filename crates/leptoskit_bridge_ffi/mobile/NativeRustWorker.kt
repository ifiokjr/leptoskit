package dev.leptoskit.mobile

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.util.concurrent.atomic.AtomicLong

/**
 * Owns Rust work independently of the app. Close only after its coroutine has
 * finished: cancellation cannot interrupt synchronous native I/O.
 */
internal class NativeRustWorker private constructor(
    handle: Long,
) : NativeRustOperation {
    private val worker = AtomicLong(handle)
    private val reply = AtomicLong(0)
    private val cancellationLock = Any()
    private var cancellation = NativeApp.workerCancellation(handle)

    companion object {
        /** At most four blocking Rust operations at once, matching the Apple host. */
        private val executor = Dispatchers.IO.limitedParallelism(4)

        /** Owns the worker the guest thread resolved for a request, if it has one. */
        fun adopt(handle: Long): NativeRustWorker? = handle.takeIf { it != 0L }?.let(::NativeRustWorker)
    }

    override suspend fun execute() {
        withContext(executor) {
            val handle = worker.getAndSet(0)
            check(handle != 0L) { "Worker already consumed" }
            // Store before returning from IO: prompt coroutine cancellation can
            // discard a withContext return value, but completion still closes us.
            reply.set(NativeApp.executeWorker(handle))
        }
    }

    override fun complete(
        guest: Long,
        id: Long,
    ): Int = NativeApp.guestCompleteWorker(guest, id, reply.getAndSet(0))

    /** Called on the host thread before cancelling the coroutine. */
    override fun cancel() {
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
