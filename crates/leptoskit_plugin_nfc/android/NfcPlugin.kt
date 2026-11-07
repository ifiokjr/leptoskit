package dev.leptoskit.plugins.nfc

import android.app.Activity
import android.content.Context
import android.nfc.NdefMessage
import android.nfc.NdefRecord
import android.nfc.NfcAdapter
import android.nfc.NfcManager
import android.os.Looper
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.Charset
import java.nio.charset.StandardCharsets
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.suspendCancellableCoroutine

/** Foreground NDEF text/URI reads through the activity's reader mode. */
class NfcPlugin(
    context: Context,
) : NativePlugin {
    override val name = "nfc"
    override val methods = intArrayOf(AVAILABILITY, READ)

    private val activity = context as? Activity
    private val adapter =
        (context.getSystemService(Context.NFC_SERVICE) as? NfcManager)?.defaultAdapter
    private var closed = false
    private var pending: CancellableContinuation<ByteArray>? = null

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        checkMain()
        if (closed) throw PluginException(PluginException.CANCELLED)
        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        return when (method) {
            AVAILABILITY -> encodeAvailability(availability())
            else -> read()
        }
    }

    override fun close() {
        checkMain()
        if (closed) return
        closed = true
        disableReader()
        pending?.let {
            pending = null
            if (it.isActive) it.resumeWithException(PluginException(PluginException.CANCELLED))
        }
    }

    private fun availability(): Int =
        when {
            pending != null -> BUSY
            adapter == null -> HARDWARE_ABSENT
            !adapter.isEnabled -> HARDWARE_ABSENT
            activity == null -> HARDWARE_ABSENT
            else -> READY
        }

    private suspend fun read(): ByteArray {
        if (pending != null) throw PluginException(PluginException.BUSY)
        val owner = activity ?: throw PluginException(PluginException.UNAVAILABLE)
        val nfc = adapter ?: throw PluginException(PluginException.UNAVAILABLE)
        if (!nfc.isEnabled) throw PluginException(PluginException.UNAVAILABLE)

        return suspendCancellableCoroutine { continuation ->
            pending = continuation
            continuation.invokeOnCancellation {
                owner.runOnUiThread {
                    if (pending === continuation) {
                        pending = null
                        disableReader()
                    }
                }
            }
            try {
                nfc.enableReaderMode(
                    owner,
                    { tag ->
                        owner.runOnUiThread { onTag(tag.getNdef()) }
                    },
                    NfcAdapter.FLAG_READER_NFC_A or
                        NfcAdapter.FLAG_READER_NFC_B or
                        NfcAdapter.FLAG_READER_NFC_F or
                        NfcAdapter.FLAG_READER_NFC_V,
                    null,
                )
            } catch (error: RuntimeException) {
                pending = null
                continuation.resumeWithException(PluginException(PluginException.NATIVE, error))
            }
        }
    }

    private fun onTag(message: NdefMessage?) {
        val continuation = pending ?: return
        pending = null
        disableReader()
        if (!continuation.isActive) return
        try {
            continuation.resume(encodeRecord(decode(message)))
        } catch (error: PluginException) {
            continuation.resumeWithException(error)
        }
    }

    private fun disableReader() {
        val owner = activity ?: return
        try {
            adapter?.disableReaderMode(owner)
        } catch (_: RuntimeException) {
        }
    }

    private fun decode(message: NdefMessage?): Pair<Int, String> {
        val record = message?.records?.firstOrNull() ?: throw PluginException(PluginException.UNAVAILABLE)
        return when (record.tnf) {
            NdefRecord.TNF_WELL_KNOWN -> {
                when {
                    record.type.contentEquals(NdefRecord.RTD_URI) ->
                        1 to (record.toUri()?.toString() ?: throw PluginException(PluginException.INVALID_ARGUMENT))
                    record.type.contentEquals(NdefRecord.RTD_TEXT) ->
                        0 to decodeText(record.payload)
                    else -> throw PluginException(PluginException.UNAVAILABLE)
                }
            }
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }.also { (_, value) ->
            if (value.isEmpty() || value.toByteArray(StandardCharsets.UTF_8).size > 4096) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }
        }
    }

    private fun decodeText(payload: ByteArray): String {
        if (payload.isEmpty()) throw PluginException(PluginException.INVALID_ARGUMENT)
        val status = payload[0].toInt()
        val languageLength = status and 0x3F
        val encoding = if (status and 0x80 == 0) StandardCharsets.UTF_8 else Charset.forName("UTF-16")
        if (payload.size < 1 + languageLength) throw PluginException(PluginException.INVALID_ARGUMENT)
        return String(payload, 1 + languageLength, payload.size - 1 - languageLength, encoding)
    }

    private fun encodeAvailability(status: Int): ByteArray =
        ByteBuffer
            .allocate(4)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(status)
            .array()

    private fun encodeRecord(record: Pair<Int, String>): ByteArray {
        val bytes = record.second.toByteArray(StandardCharsets.UTF_8)
        return ByteBuffer
            .allocate(8 + bytes.size)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(record.first)
            .putInt(bytes.size)
            .put(bytes)
            .array()
    }

    private fun android.nfc.Tag.getNdef(): NdefMessage? {
        val ndef = android.nfc.tech.Ndef.get(this) ?: return null
        return try {
            ndef.connect()
            ndef.ndefMessage.also { ndef.close() }
        } catch (_: Exception) {
            try {
                ndef.close()
            } catch (_: Exception) {
            }
            null
        }
    }

    private fun checkMain() {
        check(Looper.myLooper() == Looper.getMainLooper())
    }

    private companion object {
        const val AVAILABILITY = 0
        const val READ = 1
        const val HARDWARE_ABSENT = 1
        const val READY = 2
        const val BUSY = 3
    }
}
