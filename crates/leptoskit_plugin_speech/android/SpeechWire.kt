package dev.leptoskit.plugins.speech

import dev.leptoskit.bridge.PluginException
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction

/** Wire version the Rust crate sends and expects. */
internal const val WIRE_VERSION = 1

/** Every wire integer is one little-endian i32. */
private const val FIELD_BYTES = 4

/** Version and the three rate, pitch, and volume fields. */
private const val HEADER_BYTES = 4 * FIELD_BYTES

/** Byte limits the Rust crate enforces on each string. */
private const val MAX_TEXT_BYTES = 4096
private const val MAX_LANGUAGE_BYTES = 64
private const val MAX_VOICE_BYTES = 512

/** Characters a platform engine accepts in one utterance. */
private const val MAX_TEXT_LENGTH = 4000

/** Supported rate and pitch are a fraction of the engine's normal rate, in thousandths. */
private const val THOUSANDTHS_MIN = 500
private const val THOUSANDTHS_MAX = 2000
private const val VOLUME_MAX = 1000

internal data class SpeechRequest(
    val text: String,
    val rate: Float,
    val pitch: Float,
    val volume: Float,
    val language: String?,
    val voice: String?,
) {
    companion object {
        fun decode(payload: ByteBuffer): SpeechRequest {
            val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)

            if (input.remaining() < HEADER_BYTES || input.int != WIRE_VERSION) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }

            val rate = input.int
            val pitch = input.int
            val volume = input.int
            requireOptions(rate, pitch, volume)

            val text = readText(input, MAX_TEXT_BYTES)
            val language = readText(input, MAX_LANGUAGE_BYTES).ifEmpty { null }
            val voice = readText(input, MAX_VOICE_BYTES).ifEmpty { null }
            requireScope(text, language, voice, input)

            return SpeechRequest(
                text = text,
                rate = rate / 1000f,
                pitch = pitch / 1000f,
                volume = volume / 1000f,
                language = language,
                voice = voice,
            )
        }

        /** Rejects options the Rust crate would refuse to play. */
        private fun requireOptions(
            rate: Int,
            pitch: Int,
            volume: Int,
        ) {
            if (rate !in THOUSANDTHS_MIN..THOUSANDTHS_MAX) throw PluginException(PluginException.INVALID_ARGUMENT)

            if (pitch !in THOUSANDTHS_MIN..THOUSANDTHS_MAX) throw PluginException(PluginException.INVALID_ARGUMENT)

            if (volume !in 0..VOLUME_MAX) throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        /** Rejects text the engine cannot speak, and a request that names two voices. */
        private fun requireScope(
            text: String,
            language: String?,
            voice: String?,
            input: ByteBuffer,
        ) {
            requireText(text, input)

            if (language != null && voice != null) throw PluginException(PluginException.INVALID_ARGUMENT)

            if (language != null && !LANGUAGE.matches(language)) throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        /** Rejects empty or overlong text, and trailing bytes. */
        private fun requireText(
            text: String,
            input: ByteBuffer,
        ) {
            if (text.length > MAX_TEXT_LENGTH) throw PluginException(PluginException.TOO_LARGE)

            if (text.isEmpty()) throw PluginException(PluginException.INVALID_ARGUMENT)

            if (input.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        private val LANGUAGE = Regex("[A-Za-z]{2,3}(?:-[A-Za-z]{4})?(?:-(?:[A-Za-z]{2}|[0-9]{3}))?")

        /** Reads one length-prefixed string. */
        private fun readText(
            input: ByteBuffer,
            maxBytes: Int,
        ): String {
            val size = length(input, maxBytes)
            val value = decodeUtf8(input.slice().apply { limit(size) })
            input.position(input.position() + size)

            return value
        }

        /** Reads the length prefix, which must fit both the field limit and the payload. */
        private fun length(
            input: ByteBuffer,
            maxBytes: Int,
        ): Int {
            if (input.remaining() < FIELD_BYTES) throw PluginException(PluginException.INVALID_ARGUMENT)

            val size = input.int

            if (size < 0 || size > maxBytes) throw PluginException(PluginException.TOO_LARGE)

            if (size > input.remaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

            return size
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
    }
}

internal class SpeechEncoder {
    private val output = ByteArrayOutputStream(ENCODER_CAPACITY)

    fun integer(value: Int) {
        val bytes = ByteBuffer.allocate(FIELD_BYTES).order(ByteOrder.LITTLE_ENDIAN).putInt(value).array()

        output.write(bytes)
    }

    fun text(value: String) {
        val bytes = value.toByteArray(Charsets.UTF_8)

        if (bytes.isEmpty()) throw PluginException(PluginException.NATIVE)

        if (bytes.size > MAX_VOICE_BYTES) throw PluginException(PluginException.TOO_LARGE)

        integer(bytes.size)
        output.write(bytes)
    }

    fun bytes(): ByteArray = output.toByteArray()

    private companion object {
        /** A voice list starts small and grows on demand. */
        const val ENCODER_CAPACITY = 256

        /** Every wire integer is one little-endian i32. */
        const val FIELD_BYTES = 4
    }
}
