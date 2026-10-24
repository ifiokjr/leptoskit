package dev.leptoskit.bridge

import java.nio.ByteBuffer
import kotlin.coroutines.Continuation
import kotlin.coroutines.EmptyCoroutineContext
import kotlin.coroutines.startCoroutine

private const val REMAINING_JOB_SLOTS = 127
private const val EVENT_QUEUE_CAPACITY = 256

/** Real JNI/C ABI test, runnable on the JVM and in an Android host. */
fun main() {
    NativeBridge.load()
    val bridge = NativeBridge.create()
    check(bridge != 0L)
    try {
        verifyRoundTrip(bridge)
        verifyCancelledBudget(bridge)
        verifyByteBudget()
        verifyEventCapacity(bridge)
        println("JNI bridge round trip, permissions, direct buffer ownership, and consumed reply passed")
    } finally {
        NativeBridge.destroy(bridge)
    }
}

private fun verifyRoundTrip(bridge: Long) {
    val plugin = NativeBridge.register(bridge, "echo".toByteArray(), intArrayOf(0))
    check(plugin == 0L)
    check(NativeBridge.submit(bridge, 0, 0, byteArrayOf()) == -2L)
    check(NativeBridge.grant(bridge, 0, 0, true) == 0)
    val call = NativeBridge.submit(bridge, 0, 0, "hello".toByteArray())
    check(call > 0)
    check(NativeBridge.takeResult(bridge, call) == null)
    val request = checkNotNull(NativeBridge.nextRequest(bridge))

    check(request.bytes().isDirect)
    check(request.bytes().isReadOnly)

    val dispatcher = NativeDispatcher()
    var closed = false
    dispatcher.install(
        0,
        object : NativePlugin {
            override val name = "echo"
            override val methods = intArrayOf(0)

            override fun close() {
                closed = true
            }

            override suspend fun invoke(
                method: Int,
                payload: ByteBuffer,
            ): ByteArray = payload.utf8().toByteArray()
        },
    )
    var response: ByteArray? = null
    val operation: suspend () -> ByteArray = { dispatcher.dispatch(request) }
    operation.startCoroutine(
        object : Continuation<ByteArray> {
            override val context = EmptyCoroutineContext

            override fun resumeWith(result: Result<ByteArray>) {
                response = result.getOrThrow()
            }
        },
    )
    check(runCatching { request.bytes() }.isFailure)
    check(NativeBridge.complete(bridge, call, 0, checkNotNull(response)) == 0)
    check(checkNotNull(NativeBridge.takeResult(bridge, call)).decodeToString() == "hello")
    verifyConsumedCall(bridge, call)

    dispatcher.close()
    check(closed)
    dispatcher.close()
}

private fun verifyConsumedCall(
    bridge: Long,
    call: Long,
) {
    try {
        NativeBridge.takeResult(bridge, call)
        error("A consumed call must fail")
    } catch (error: PluginException) {
        check(error.code == 8)
    }
}

private fun verifyCancelledBudget(bridge: Long) {
    val cancelled = NativeBridge.submit(bridge, 0, 0, byteArrayOf())
    val cancelledRequest = checkNotNull(NativeBridge.nextRequest(bridge))
    val budget = NativeJobBudget()
    budget.acquire(cancelled, cancelledRequest.bytes().remaining())
    repeat(REMAINING_JOB_SLOTS) { budget.acquire(1000L + it, 0) }
    check(NativeBridge.cancel(bridge, cancelled) == 0)
    check(NativeBridge.complete(bridge, cancelled, 0, byteArrayOf()) == -8)
    check(NativeBridge.nextCancelled(bridge) == cancelled)
    check(NativeBridge.nextCancelled(bridge) == 0L)
    // Cancellation frees a bridge slot, but native work can still be using
    // its request. It must not free the separate host reservation.
    check(!budget.canTakeRequest)
    check(runCatching { budget.acquire(2000, 0) }.isFailure)
    cancelledRequest.close()
    budget.release(cancelled)

    check(budget.canTakeRequest)
}

private fun verifyByteBudget() {
    val byteBudget = NativeJobBudget()
    repeat(4) { byteBudget.acquire(it.toLong(), NativeJobBudget.MAXIMUM_PAYLOAD_BYTES) }
    check(!byteBudget.canTakeRequest)
    byteBudget.release(0)
    byteBudget.release(0)
    byteBudget.acquire(4, NativeJobBudget.MAXIMUM_PAYLOAD_BYTES)
    check(!byteBudget.canTakeRequest)
}

private fun verifyEventCapacity(bridge: Long) {
    repeat(EVENT_QUEUE_CAPACITY) {
        check(NativeBridge.emit(bridge, 0, 1, byteArrayOf()) == 0)
    }
    check(NativeBridge.emit(bridge, 0, 1, byteArrayOf()) == -4)
}
