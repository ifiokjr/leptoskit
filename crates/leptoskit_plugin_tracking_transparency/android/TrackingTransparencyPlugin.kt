package dev.leptoskit.plugins.trackingtransparency

import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer

/** Android has no Apple ATT authorization; absence is never an inferred grant. */
class TrackingTransparencyPlugin : NativePlugin {
    override val name = "tracking-transparency"
    override val methods = intArrayOf(0, 1)
    private var closed = false

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method !in 0..1) throw PluginException(PluginException.UNAVAILABLE)

        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
        throw PluginException(PluginException.UNAVAILABLE)
    }

    override fun close() {
        closed = true
    }
}
