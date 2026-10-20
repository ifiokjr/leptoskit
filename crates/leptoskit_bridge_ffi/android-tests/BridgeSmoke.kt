package dev.leptoskit.bridge

import java.nio.ByteBuffer
import kotlin.coroutines.Continuation
import kotlin.coroutines.EmptyCoroutineContext
import kotlin.coroutines.startCoroutine

/** Real JNI/C ABI test, runnable on the JVM and in an Android host. */
fun main() {
    NativeBridge.load()
    val bridge = NativeBridge.create()
    check(bridge != 0L)
    try {
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
        try {
            NativeBridge.takeResult(bridge, call)
            error("A consumed call must fail")
        } catch (error: PluginException) {
            check(error.code == 8)
        }
        dispatcher.close()
        check(closed)
        dispatcher.close()
        val cancelled = NativeBridge.submit(bridge, 0, 0, byteArrayOf())
        checkNotNull(NativeBridge.nextRequest(bridge)).close()
        check(NativeBridge.cancel(bridge, cancelled) == 0)
        check(NativeBridge.complete(bridge, cancelled, 0, byteArrayOf()) == -8)
        check(NativeBridge.nextCancelled(bridge) == cancelled)
        check(NativeBridge.nextCancelled(bridge) == 0L)
        repeat(256) {
            check(NativeBridge.emit(bridge, 0, 1, byteArrayOf()) == 0)
        }
        check(NativeBridge.emit(bridge, 0, 1, byteArrayOf()) == -4)
        println("JNI bridge round trip, permissions, direct buffer ownership, and consumed reply passed")
    } finally {
        NativeBridge.destroy(bridge)
    }
}
