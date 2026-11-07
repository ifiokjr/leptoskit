package dev.leptoskit.plugins.liveactivities

import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer

/** Android has no ActivityKit Live Activities; absence is never a synthetic success. */
class LiveActivitiesPlugin : NativePlugin {
    override val name = "live-activities"
    override val methods = intArrayOf(0, 1, 2, 3, 4, 5, 6)
    private var closed = false

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)
        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)
        // Payload shape is validated on the Rust side for guest callers; the
        // Android host still refuses the capability rather than inventing state.
        throw PluginException(PluginException.UNAVAILABLE)
    }

    override fun close() {
        closed = true
    }
}
