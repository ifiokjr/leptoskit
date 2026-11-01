package dev.leptoskit.mobile

import kotlin.coroutines.Continuation
import kotlin.coroutines.resume
import kotlin.coroutines.suspendCoroutine

internal interface NativeRustOperation : AutoCloseable {
    suspend fun execute()

    /** Hands the reply to the guest thread; it is consumed on every path. */
    fun complete(
        guest: Long,
        id: Long,
    ): Int

    fun cancel()
}

/**
 * A retained Rust future. Cancellation requests cleanup but does not resume the
 * coroutine: its host capacity remains reserved until the native callback.
 */
internal class NativeRustAsync private constructor(
    private var handle: Long,
) : NativeRustOperation {
    private val lock = Any()
    private var cancellation = NativeApp.asyncCancellation(handle)
    private var reply = 0L

    companion object {
        /** Owns the async job the guest thread resolved for a request, if it has one. */
        fun adopt(handle: Long): NativeRustAsync? = handle.takeIf { it != 0L }?.let(::NativeRustAsync)
    }

    override suspend fun execute() {
        suspendCoroutine { continuation ->
            val value =
                synchronized(lock) {
                    check(handle != 0L) { "Async job already consumed" }
                    handle.also { handle = 0 }
                }
            // start may invoke completion before it returns. Never hold lock.
            NativeApp.startAsync(value, Completion(this, continuation))
        }
    }

    override fun complete(
        guest: Long,
        id: Long,
    ): Int {
        val value = synchronized(lock) { reply.also { reply = 0 } }

        return NativeApp.guestCompleteWorker(guest, id, value)
    }

    override fun cancel() {
        synchronized(lock) { NativeApp.cancelAsync(cancellation) }
    }

    /** Called only after the host coroutine actually completes. */
    override fun close() {
        synchronized(lock) {
            NativeApp.releaseAsync(handle)
            handle = 0
            NativeApp.releaseWorkerReply(reply)
            reply = 0
            NativeApp.releaseAsyncCancellation(cancellation)
            cancellation = 0
        }
    }

    /**
     * JNI holds a global reference until exactly one Rust callback. The stored
     * reply remains owned even when coroutine cancellation discards resumption.
     */
    class Completion internal constructor(
        private val owner: NativeRustAsync,
        private val continuation: Continuation<Unit>,
    ) {
        @Suppress("unused") // Called by JNI.
        fun completed(reply: Long) {
            synchronized(owner.lock) { owner.reply = reply }
            continuation.resume(Unit)
        }
    }
}
