package dev.leptoskit.plugins.localauthentication

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction

/** A capabilities query is one little-endian policy u32. */
internal const val POLICY_BYTES = 4

/** Wire version the Rust crate sends and expects. */
internal const val WIRE_VERSION = 1

/** Biometrics only; Android restricts this to Class 3 (strong) sensors. */
internal const val BIOMETRIC_POLICY = 0

/** Biometrics or the operating system's device PIN, pattern, or password. */
internal const val CREDENTIAL_POLICY = 1

/** Longest accepted reason, in UTF-8 bytes. */
internal const val MAX_REASON_BYTES = 256

/** Longest accepted cancel label, in UTF-8 bytes. */
internal const val MAX_CANCEL_BYTES = 64

/** Every wire integer is one little-endian u32. */
private const val FIELD_BYTES = 4

/** Version and policy precede the two length prefixes. */
private const val HEADER_BYTES = 2 * FIELD_BYTES

internal data class AuthenticationRequest(
    val policy: Int,
    val reason: String,
    val cancelLabel: String,
) {
    companion object {
        fun decode(payload: ByteBuffer): AuthenticationRequest {
            val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)

            if (input.remaining() < HEADER_BYTES || input.int != WIRE_VERSION) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }

            val decoded = readPolicy(input.int)
            val reason = text(input, MAX_REASON_BYTES)
            val cancel = text(input, MAX_CANCEL_BYTES)

            if (input.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

            return AuthenticationRequest(decoded, reason, cancel)
        }
    }
}

/** Reads the policy, rejecting a value the Rust crate does not define. */
private fun readPolicy(value: Int): Int {
    if (value != BIOMETRIC_POLICY && value != CREDENTIAL_POLICY) {
        throw PluginException(PluginException.INVALID_ARGUMENT)
    }

    return value
}

/** Reads one length-prefixed string, rejecting blank or NUL-bearing text. */
private fun text(
    input: ByteBuffer,
    maxBytes: Int,
): String {
    val count = length(input, maxBytes)
    val value = decodeUtf8(input.slice().apply { limit(count) })
    input.position(input.position() + count)

    if (value.isBlank() || '\u0000' in value) throw PluginException(PluginException.INVALID_ARGUMENT)

    return value
}

/** Reads the length prefix, which must fit both the buffer and the field's limit. */
private fun length(
    input: ByteBuffer,
    maxBytes: Int,
): Int {
    if (input.remaining() < FIELD_BYTES) throw PluginException(PluginException.INVALID_ARGUMENT)

    val count = input.int

    if (count < 0 || count > maxBytes) throw PluginException(PluginException.TOO_LARGE)

    if (count > input.remaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

    return count
}

/** Decodes strict UTF-8, so malformed bytes are an argument error, not replacement text. */
private fun decodeUtf8(bytes: ByteBuffer): String =
    try {
        Charsets.UTF_8
            .newDecoder()
            .onMalformedInput(CodingErrorAction.REPORT)
            .onUnmappableCharacter(CodingErrorAction.REPORT)
            .decode(bytes)
            .toString()
    } catch (error: CharacterCodingException) {
        throw PluginException(PluginException.INVALID_ARGUMENT, error)
    }

internal fun authenticationFields(vararg values: Int): ByteArray {
    val output = ByteBuffer.allocate(values.size * FIELD_BYTES).order(ByteOrder.LITTLE_ENDIAN)

    for (value in values) output.putInt(value)

    return output.array()
}
