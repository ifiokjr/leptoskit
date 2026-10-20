package dev.leptoskit.bridge

import java.nio.ByteBuffer
import java.nio.charset.CodingErrorAction

/** Native failure codes shared with Rust and Swift. */
class PluginException
    @JvmOverloads
    constructor(
        val code: Int,
        cause: Throwable? = null,
    ) : Exception("Native plugin error $code", cause) {
        companion object {
            const val UNAVAILABLE = 1
            const val PERMISSION_DENIED = 2
            const val INVALID_ARGUMENT = 3
            const val BUSY = 4
            const val TOO_LARGE = 5
            const val CANCELLED = 6
            const val NATIVE = 7
            const val INVALID_CALL = 8
            const val EXHAUSTED = 9
        }
    }

/**
 * Invoked on the host's UI dispatcher. Suspend for permission dialogs and I/O.
 * The borrowed payload must not escape invoke; copy it if a detached job needs it.
 */
interface NativePlugin {
    val name: String
    val methods: IntArray

    suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray

    /** Stop subscriptions and cancel native work before unloading the guest. */
    fun close() {}
}

/** Rust-owned immutable request bytes; close exactly once after dispatch. */
class NativeRequest(
    val id: Long,
    val plugin: Int,
    val method: Int,
    private val payload: ByteBuffer,
    private var owner: Long,
) : AutoCloseable {
    fun bytes(): ByteBuffer {
        check(owner != 0L) { "Request is closed" }
        return payload.asReadOnlyBuffer()
    }

    /** Lends the Rust owner to an in-module JNI call that retains its Bytes. */
    internal fun <T> withNativeOwner(block: (Long) -> T): T {
        check(owner != 0L) { "Request is closed" }
        return block(owner)
    }

    override fun close() {
        if (owner != 0L) {
            NativeBridge.release(owner)
            owner = 0
        }
    }
}

/** Explicit loading leaves library ownership with the embedding Android app. */
object NativeBridge {
    fun load() = System.loadLibrary("leptoskit_bridge_jni")

    @JvmStatic external fun create(): Long

    @JvmStatic external fun destroy(bridge: Long)

    @JvmStatic external fun register(
        bridge: Long,
        name: ByteArray,
        methods: IntArray,
    ): Long

    @JvmStatic external fun grant(
        bridge: Long,
        plugin: Int,
        method: Int,
        allowed: Boolean,
    ): Int

    @JvmStatic external fun nextRequest(bridge: Long): NativeRequest?

    @JvmStatic external fun submit(
        bridge: Long,
        plugin: Int,
        method: Int,
        payload: ByteArray,
    ): Long

    @JvmStatic external fun takeResult(
        bridge: Long,
        id: Long,
    ): ByteArray?

    @JvmStatic external fun complete(
        bridge: Long,
        id: Long,
        error: Int,
        payload: ByteArray,
    ): Int

    @JvmStatic external fun cancel(
        bridge: Long,
        id: Long,
    ): Int

    /** Positive cancelled call ID, zero when empty, or a negative failure code. */
    @JvmStatic external fun nextCancelled(bridge: Long): Long

    @JvmStatic external fun emit(
        bridge: Long,
        plugin: Int,
        kind: Int,
        payload: ByteArray,
    ): Int

    @JvmStatic external fun release(owner: Long)
}

/** Decode UTF-8 strictly, matching the Rust/Swift contract. */
fun ByteBuffer.utf8(): String =
    try {
        Charsets.UTF_8
            .newDecoder()
            .onMalformedInput(CodingErrorAction.REPORT)
            .onUnmappableCharacter(CodingErrorAction.REPORT)
            .decode(asReadOnlyBuffer())
            .toString()
    } catch (error: java.nio.charset.CharacterCodingException) {
        throw PluginException(PluginException.INVALID_ARGUMENT, error)
    }

/** Host-thread-confined registry; exceptions are not silently swallowed. */
class NativeDispatcher : AutoCloseable {
    private val plugins = mutableMapOf<Int, NativePlugin>()

    /** Registers an explicitly selected plugin and grants its declared methods. */
    fun register(
        bridge: Long,
        plugin: NativePlugin,
    ) {
        val id = NativeBridge.register(bridge, plugin.name.toByteArray(Charsets.UTF_8), plugin.methods)
        require(id in 0..Int.MAX_VALUE.toLong()) { "Cannot register ${plugin.name}: $id" }
        install(id.toInt(), plugin)
        for (method in plugin.methods) {
            check(NativeBridge.grant(bridge, id.toInt(), method, true) == 0)
        }
    }

    fun install(
        id: Int,
        plugin: NativePlugin,
    ) {
        require(id !in plugins) { "Duplicate plugin ID" }
        plugins[id] = plugin
    }

    /** The host must also cancel dispatcher coroutines before unloading. */
    override fun close() {
        var failure: Exception? = null
        for (plugin in plugins.values) {
            try {
                plugin.close()
            } catch (error: Exception) {
                if (failure == null) failure = error else failure.addSuppressed(error)
            }
        }
        plugins.clear()
        failure?.let { throw it }
    }

    suspend fun dispatch(request: NativeRequest): ByteArray =
        request.use {
            val plugin = plugins[it.plugin] ?: throw PluginException(PluginException.UNAVAILABLE)
            if (it.method !in plugin.methods) throw PluginException(PluginException.UNAVAILABLE)
            plugin.invoke(it.method, it.bytes())
        }
}

/**
 * Host-thread accounting retained until a native job actually finishes.
 * Guest cancellation does not release its payload or native worker reservation.
 */
class NativeJobBudget {
    private val reservations = mutableMapOf<Long, Int>()
    private var bytes = 0

    /** Reserve room for the largest possible bridge request before taking it. */
    val canTakeRequest: Boolean
        get() = reservations.size < 128 && bytes <= 32 * 1024 * 1024 - MAXIMUM_PAYLOAD_BYTES

    fun acquire(
        id: Long,
        payloadBytes: Int,
    ) {
        check(canTakeRequest) { "Native job budget exhausted" }
        require(id !in reservations && payloadBytes in 0..MAXIMUM_PAYLOAD_BYTES)
        reservations[id] = payloadBytes
        bytes += payloadBytes
    }

    fun release(id: Long) {
        val size = reservations.remove(id) ?: return
        bytes -= size
    }

    companion object {
        const val MAXIMUM_PAYLOAD_BYTES = 8 * 1024 * 1024
    }
}
