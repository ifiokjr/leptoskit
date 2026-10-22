package dev.leptoskit.plugins.localauthentication

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CodingErrorAction

internal data class AuthenticationRequest(
    val policy: Int,
    val reason: String,
    val cancelLabel: String,
) {
    companion object {
        fun decode(payload: ByteBuffer): AuthenticationRequest {
            val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
            if (input.remaining() < 8 || input.int != 1) throw PluginException(PluginException.INVALID_ARGUMENT)
            val policy = input.int
            if (policy !in 0..1) throw PluginException(PluginException.INVALID_ARGUMENT)
            val reason = text(input, 256)
            val cancel = text(input, 64)
            if (input.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
            return AuthenticationRequest(policy, reason, cancel)
        }

        private fun text(
            input: ByteBuffer,
            limit: Int,
        ): String {
            if (input.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)
            val count = input.int
            if (count < 0 || count > limit) throw PluginException(PluginException.TOO_LARGE)
            if (count > input.remaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
            val bytes = input.slice().apply { limit(count) }
            val value =
                try {
                    Charsets.UTF_8
                        .newDecoder()
                        .onMalformedInput(CodingErrorAction.REPORT)
                        .onUnmappableCharacter(CodingErrorAction.REPORT)
                        .decode(bytes)
                        .toString()
                } catch (error: java.nio.charset.CharacterCodingException) {
                    throw PluginException(PluginException.INVALID_ARGUMENT, error)
                }
            input.position(input.position() + count)
            if (value.isBlank() || '\u0000' in value) throw PluginException(PluginException.INVALID_ARGUMENT)
            return value
        }
    }
}

internal fun authenticationFields(vararg values: Int): ByteArray {
    val output = ByteBuffer.allocate(values.size * 4).order(ByteOrder.LITTLE_ENDIAN)
    for (value in values) output.putInt(value)
    return output.array()
}
