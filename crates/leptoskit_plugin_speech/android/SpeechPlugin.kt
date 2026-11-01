package dev.leptoskit.plugins.speech

import android.content.Context
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.speech.tts.TextToSpeech
import android.speech.tts.UtteranceProgressListener
import android.speech.tts.Voice
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
    override val methods = intArrayOf(VOICES, SPEAK, STOP, STATUS)
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
        requireOpen(method)

        if (method == SPEAK) return speak(SpeechRequest.decode(payload))

        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        return answer(method)
    }

    /** Rejects a closed plugin or a method this host does not answer. */
    private fun requireOpen(method: Int) {
        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)
    }

    /** Answers one of the methods that carry no payload. */
    private suspend fun answer(method: Int): ByteArray {
        return when (method) {
            VOICES -> voices(ready())
            STOP -> stop()
            else -> speaking()
        }
    }

    /** Dismisses the current utterance, or reports that the engine refused. */
    private fun stop(): ByteArray {
        if (!cancel()) throw PluginException(PluginException.NATIVE)

        return ByteArray(0)
    }

    /** Whether an utterance is in flight. */
    private fun speaking(): ByteArray {
        return byteArrayOf(if (active == null) 0 else 1, 0, 0, 0)
    }

    /** The engine, once the one-time initialization it may still need has run. */
    private suspend fun ready(): TextToSpeech {
        val pending = initialized ?: initialize()
        val status =
            withTimeoutOrNull(INIT_TIMEOUT_MS) { pending.await() }
                ?: throw PluginException(PluginException.UNAVAILABLE)

        if (closed) throw PluginException(PluginException.CANCELLED)

        if (status == TextToSpeech.SUCCESS) return engine ?: throw PluginException(PluginException.CANCELLED)

        releaseFailed(pending)

        throw PluginException(PluginException.UNAVAILABLE)
    }

    /** Starts the engine's one-time initialization. */
    private fun initialize(): CompletableDeferred<Int> {
        val deferred = CompletableDeferred<Int>()
        initialized = deferred
        val created = TextToSpeech(context) { status -> handler.post { deferred.complete(status) } }
        engine = created
        created.setOnUtteranceProgressListener(progress())

        return deferred
    }

    /** Releases a failed engine, so the next call can install a fresh one. */
    private fun releaseFailed(pending: CompletableDeferred<Int>) {
        // A failed engine never recovers; releasing its service binding lets the
        // next call create a fresh engine, e.g. after one is installed.
        if (initialized !== pending) return

        engine?.shutdown()
        engine = null
        initialized = null
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

        if (voices.size > MAX_VOICES) throw PluginException(PluginException.TOO_LARGE)
        val output = SpeechEncoder()
        output.integer(WIRE_VERSION)
        output.integer(voices.size)
        voices.forEach { write(it, output) }

        return output.bytes()
    }

    /** Writes one voice: identifier, name, language, quality, and source. */
    private fun write(
        voice: Voice,
        output: SpeechEncoder,
    ) {
        // Android reports a single name per voice, so it fills both wire fields.
        output.text(voice.name)
        output.text(voice.name)

        output.text(voice.locale.toLanguageTag())

        output.integer(if (voice.quality >= Voice.QUALITY_HIGH) QUALITY_ENHANCED else QUALITY_DEFAULT)

        output.integer(if (voice.isNetworkConnectionRequired) SOURCE_NETWORK else SOURCE_LOCAL)
    }

    private suspend fun speak(request: SpeechRequest): ByteArray {
        val speech = ready()

        if (active != null) throw PluginException(PluginException.BUSY)

        if (request.text.length > TextToSpeech.getMaxSpeechInputLength()) throw PluginException(PluginException.TOO_LARGE)

        configure(speech, request)

        return awaitUtterance(speech, request)
    }

    /** Applies the voice or language, rate, and pitch the request asked for. */
    private fun configure(speech: TextToSpeech, request: SpeechRequest) {
        if (request.voice != null) selectVoice(speech, request.voice) else selectLanguage(speech, request.language)

        if (speech.setSpeechRate(request.rate) == TextToSpeech.ERROR) throw PluginException(PluginException.NATIVE)

        if (speech.setPitch(request.pitch) == TextToSpeech.ERROR) throw PluginException(PluginException.NATIVE)
    }

    /** Selects one installed voice by name. */
    private fun selectVoice(speech: TextToSpeech, name: String) {
        val voice = speech.voices?.firstOrNull { it.name == name } ?: throw PluginException(PluginException.UNAVAILABLE)

        if (speech.setVoice(voice) == TextToSpeech.ERROR) throw PluginException(PluginException.NATIVE)
    }

    /** Selects a language, falling back to the device default. */
    private fun selectLanguage(speech: TextToSpeech, language: String?) {
        val locale = language?.let { Locale.forLanguageTag(it) } ?: Locale.getDefault()

        if (speech.setLanguage(locale) < 0) throw PluginException(PluginException.UNAVAILABLE)
    }

    private suspend fun awaitUtterance(
        speech: TextToSpeech,
        request: SpeechRequest,
    ): ByteArray =
        suspendCancellableCoroutine { continuation ->

            if (!continuation.isActive) return@suspendCancellableCoroutine

            val id = UUID.randomUUID().toString()
            active = ActiveSpeech(id, continuation)
            continuation.invokeOnCancellation { cancelWhenCurrent(id) }
            speakUtterance(speech, request, id)
        }

    /** Queues the utterance, answering at once when the engine refuses it. */
    private fun speakUtterance(
        speech: TextToSpeech,
        request: SpeechRequest,
        id: String,
    ) {
        val options = Bundle().apply { putFloat(TextToSpeech.Engine.KEY_PARAM_VOLUME, request.volume) }

        try {
            if (speech.speak(request.text, TextToSpeech.QUEUE_FLUSH, options, id) == TextToSpeech.ERROR) {
                finish(id, PluginException.NATIVE)
            }
        } catch (error: RuntimeException) {
            finish(id, PluginException.NATIVE, error)
        }
    }

    /** Cancels the utterance from whichever thread the cancellation arrives on. */
    private fun cancelWhenCurrent(id: String) {
        if (Looper.myLooper() == handler.looper) {
            cancelIfCurrent(id)

            return
        }

        handler.post { cancelIfCurrent(id) }
    }

    private fun cancelIfCurrent(id: String) {
        if (active?.id == id) cancel()
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

            return
        }

        current.completion.resumeWithException(PluginException(error, cause))
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

    private companion object {
        /** Methods the Rust crate calls. */
        const val VOICES = 0
        const val SPEAK = 1
        const val STOP = 2
        const val STATUS = 3

        /** How long the engine may take to answer its initialization. */
        const val INIT_TIMEOUT_MS = 10_000L

        /** How many voices the reply may carry. */
        const val MAX_VOICES = 1024

        /** Quality codes the voice list carries. */
        const val QUALITY_DEFAULT = 1
        const val QUALITY_ENHANCED = 2

        /** Whether a voice needs a network connection. */
        const val SOURCE_LOCAL = 2
        const val SOURCE_NETWORK = 1
    }
}
