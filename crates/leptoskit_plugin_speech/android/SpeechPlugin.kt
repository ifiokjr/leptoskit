package dev.leptoskit.plugins.speech

import android.content.Context
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.speech.tts.TextToSpeech
import android.speech.tts.UtteranceProgressListener
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeoutOrNull
import java.nio.ByteBuffer
import java.util.Locale
import java.util.UUID
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

class SpeechPlugin(
    context: Context,
) : NativePlugin {
    override val name = "speech"
    override val methods = intArrayOf(0, 1, 2, 3)
    private val context = context.applicationContext
    private val handler = Handler(Looper.getMainLooper())
    private var initialized: CompletableDeferred<Int>? = null
    private var engine: TextToSpeech? = null
    private var active: ActiveSpeech? = null
    private var closed = false

    private data class ActiveSpeech(
        val id: String,
        val completion: CancellableContinuation<ByteArray>,
    )

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)
        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)
        if (method == 1) return speak(SpeechRequest.decode(payload))
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
        return when (method) {
            0 -> {
                voices(ready())
            }

            2 -> {
                if (!cancel()) throw PluginException(PluginException.NATIVE)
                ByteArray(0)
            }

            else -> {
                byteArrayOf(if (active == null) 0 else 1, 0, 0, 0)
            }
        }
    }

    private suspend fun ready(): TextToSpeech {
        val pending =
            initialized ?: CompletableDeferred<Int>().also { deferred ->
                initialized = deferred
                val created = TextToSpeech(context) { status -> handler.post { deferred.complete(status) } }
                engine = created
                created.setOnUtteranceProgressListener(progress())
            }
        val status = withTimeoutOrNull(10_000) { pending.await() } ?: throw PluginException(PluginException.UNAVAILABLE)
        if (closed) throw PluginException(PluginException.CANCELLED)
        if (status != TextToSpeech.SUCCESS) {
            // A failed engine never recovers. Release its service binding so the
            // next call creates a fresh engine, e.g. after one is installed.
            if (initialized === pending) {
                engine?.shutdown()
                engine = null
                initialized = null
            }
            throw PluginException(PluginException.UNAVAILABLE)
        }
        return engine ?: throw PluginException(PluginException.CANCELLED)
    }

    private fun progress() =
        object : UtteranceProgressListener() {
            override fun onStart(utteranceId: String?) {}

            override fun onDone(utteranceId: String?) {
                handler.post { finish(utteranceId, null) }
            }

            @Deprecated("Required legacy Android callback")
            override fun onError(utteranceId: String?) {
                handler.post { finish(utteranceId, PluginException.NATIVE) }
            }

            override fun onError(
                utteranceId: String?,
                errorCode: Int,
            ) {
                handler.post { finish(utteranceId, PluginException.NATIVE) }
            }

            override fun onStop(
                utteranceId: String?,
                interrupted: Boolean,
            ) {
                handler.post { finish(utteranceId, PluginException.CANCELLED) }
            }
        }

    private fun voices(speech: TextToSpeech): ByteArray {
        val voices = speech.voices?.sortedBy { it.name } ?: emptyList()
        if (voices.size > 1024) throw PluginException(PluginException.TOO_LARGE)
        val output = SpeechEncoder()
        output.integer(1)
        output.integer(voices.size)
        for (voice in voices) {
            output.text(voice.name)
            output.text(voice.name)
            output.text(voice.locale.toLanguageTag())
            output.integer(if (voice.quality >= android.speech.tts.Voice.QUALITY_HIGH) 2 else 1)
            output.integer(if (voice.isNetworkConnectionRequired) 1 else 2)
        }
        return output.bytes()
    }

    private suspend fun speak(request: SpeechRequest): ByteArray {
        val speech = ready()
        if (active != null) throw PluginException(PluginException.BUSY)
        if (request.text.length > TextToSpeech.getMaxSpeechInputLength()) throw PluginException(PluginException.TOO_LARGE)
        if (request.voice != null) {
            val voice = speech.voices?.firstOrNull { it.name == request.voice } ?: throw PluginException(PluginException.UNAVAILABLE)
            if (speech.setVoice(voice) == TextToSpeech.ERROR) throw PluginException(PluginException.NATIVE)
        } else {
            val language = request.language?.let { Locale.forLanguageTag(it) } ?: Locale.getDefault()
            if (speech.setLanguage(language) < 0) throw PluginException(PluginException.UNAVAILABLE)
        }
        if (speech.setSpeechRate(request.rate) == TextToSpeech.ERROR ||
            speech.setPitch(request.pitch) == TextToSpeech.ERROR
        ) {
            throw PluginException(PluginException.NATIVE)
        }
        return suspendCancellableCoroutine { continuation ->
            if (!continuation.isActive) return@suspendCancellableCoroutine
            val id = UUID.randomUUID().toString()
            active = ActiveSpeech(id, continuation)
            continuation.invokeOnCancellation {
                if (Looper.myLooper() == handler.looper) {
                    if (active?.id == id) cancel()
                } else {
                    handler.post { if (active?.id == id) cancel() }
                }
            }
            val options = Bundle().apply { putFloat(TextToSpeech.Engine.KEY_PARAM_VOLUME, request.volume) }
            try {
                if (speech.speak(request.text, TextToSpeech.QUEUE_FLUSH, options, id) ==
                    TextToSpeech.ERROR
                ) {
                    finish(id, PluginException.NATIVE)
                }
            } catch (error: RuntimeException) {
                finish(id, PluginException.NATIVE, error)
            }
        }
    }

    private fun finish(
        id: String?,
        error: Int?,
        cause: Throwable? = null,
    ) {
        val current = active ?: return
        if (current.id != id) return
        active = null
        if (!current.completion.isActive) return
        if (error == null) {
            current.completion.resume(ByteArray(0))
        } else {
            current.completion.resumeWithException(PluginException(error, cause))
        }
    }

    private fun cancel(): Boolean {
        val current = active ?: return true
        active = null
        val stopped = engine?.stop() != TextToSpeech.ERROR
        if (current.completion.isActive) {
            current.completion.resumeWithException(
                PluginException(if (stopped) PluginException.CANCELLED else PluginException.NATIVE),
            )
        }
        return stopped
    }

    override fun close() {
        closed = true
        cancel()
        initialized?.cancel()
        initialized = null
        engine?.shutdown()
        engine = null
    }
}
