package dev.leptoskit.plugins.speech

import dev.leptoskit.bridge.PluginException
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CodingErrorAction

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
            if (input.remaining() < 16 || input.int != 1) throw PluginException(PluginException.INVALID_ARGUMENT)
            val rate = input.int
            val pitch = input.int
            val volume = input.int
            if (rate !in 500..2000 || pitch !in 500..2000 || volume !in 0..1000) throw PluginException(PluginException.INVALID_ARGUMENT)
            val text = readText(input, 4096)
            val language = readText(input, 64).ifEmpty { null }
            val voice = readText(input, 512).ifEmpty { null }
            if (text.length > 4000) throw PluginException(PluginException.TOO_LARGE)
            if (text.isEmpty() || input.hasRemaining() ||
                (language != null && voice != null)
            ) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }
            if (language != null && !LANGUAGE.matches(language)) throw PluginException(PluginException.INVALID_ARGUMENT)
            return SpeechRequest(text, rate / 1000f, pitch / 1000f, volume / 1000f, language, voice)
        }

        private val LANGUAGE = Regex("[A-Za-z]{2,3}(?:-[A-Za-z]{4})?(?:-(?:[A-Za-z]{2}|[0-9]{3}))?")

        private fun readText(
            input: ByteBuffer,
            limit: Int,
        ): String {
            if (input.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)
            val size = input.int
            if (size < 0) throw PluginException(PluginException.TOO_LARGE)
            if (size > limit) throw PluginException(PluginException.TOO_LARGE)
            if (size > input.remaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
            val slice = input.slice().apply { limit(size) }
            val value =
                try {
                    Charsets.UTF_8
                        .newDecoder()
                        .onMalformedInput(CodingErrorAction.REPORT)
                        .onUnmappableCharacter(CodingErrorAction.REPORT)
                        .decode(slice)
                        .toString()
                } catch (error: java.nio.charset.CharacterCodingException) {
                    throw PluginException(PluginException.INVALID_ARGUMENT, error)
                }
            input.position(input.position() + size)
            return value
        }
    }
}

internal class SpeechEncoder {
    private val output = ByteArrayOutputStream(256)

    fun integer(value: Int) {
        output.write(
            ByteBuffer
                .allocate(4)
                .order(ByteOrder.LITTLE_ENDIAN)
                .putInt(value)
                .array(),
        )
    }

    fun text(value: String) {
        val bytes = value.toByteArray(Charsets.UTF_8)
        if (bytes.isEmpty()) throw PluginException(PluginException.NATIVE)
        if (bytes.size > 512) throw PluginException(PluginException.TOO_LARGE)
        integer(bytes.size)
        output.write(bytes)
    }

    fun bytes(): ByteArray = output.toByteArray()
}
