package dev.example

import android.content.Context
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer

/** Echo capability with a distinct identity from the Rust session service. */
class PlatformPlugin(context: Context) : NativePlugin {
    init {
        require(context.packageName.isNotEmpty())
    }

    override val name = "fixture-platform"
    override val methods = intArrayOf(1)

    override suspend fun invoke(method: Int, payload: ByteBuffer): ByteArray {
        if (method != 1) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        val bytes = ByteArray(payload.remaining())
        payload.asReadOnlyBuffer().get(bytes)

        return bytes
    }
}
