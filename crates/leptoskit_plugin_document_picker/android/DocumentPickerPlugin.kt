package dev.leptoskit.plugins.documentpicker

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Looper
import android.provider.OpenableColumns
import dev.leptoskit.bridge.NativeActivityResultHost
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import java.io.IOException
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Imports one document, retaining no persistable URI grant and requesting no storage permission. */
class DocumentPickerPlugin(
    context: Context,
) : NativePlugin {
    override val name = "document-picker"
    override val methods = intArrayOf(0)
    private val activity = context as? Activity
    private val host = context as? NativeActivityResultHost
    private var busy = false
    private var closed = false
    private var pendingJob: Job? = null

    override fun close() {
        closed = true
        pendingJob?.cancel()
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Document presentation requires the UI thread" }
        if (closed) throw PluginException(PluginException.CANCELLED)
        if (method != 0) throw PluginException(PluginException.UNAVAILABLE)
        if (payload.remaining() != 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        val limit = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN).int
        if (limit !in 0..DocumentEncoding.MAX_BYTES) throw PluginException(PluginException.TOO_LARGE)
        if (busy) throw PluginException(PluginException.BUSY)
        val activity = activity ?: throw PluginException(PluginException.UNAVAILABLE)
        val host = host ?: throw PluginException(PluginException.UNAVAILABLE)
        busy = true
        pendingJob = currentCoroutineContext()[Job]
        try {
            val intent =
                Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                    addCategory(Intent.CATEGORY_OPENABLE)
                    type = "*/*"
                    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                }
            val result = host.launchPluginActivity(intent)
            if (result.resultCode == Activity.RESULT_CANCELED) throw PluginException(PluginException.CANCELLED)
            if (result.resultCode != Activity.RESULT_OK) throw PluginException(PluginException.NATIVE)
            val uri = result.data?.data ?: throw PluginException(PluginException.NATIVE)
            if (uri.scheme != "content") throw PluginException(PluginException.INVALID_ARGUMENT)
            val bytes = withContext(Dispatchers.IO) { read(activity, uri, limit) }
            if (closed) throw PluginException(PluginException.CANCELLED)
            return bytes
        } catch (error: SecurityException) {
            throw PluginException(PluginException.PERMISSION_DENIED, error)
        } catch (error: IOException) {
            throw PluginException(PluginException.NATIVE, error)
        } finally {
            busy = false
            pendingJob = null
        }
    }

    private suspend fun read(
        activity: Activity,
        uri: Uri,
        limit: Int,
    ): ByteArray {
        currentCoroutineContext().ensureActive()
        val resolver = activity.contentResolver
        var name: String? = null
        resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { cursor ->
            if (cursor.moveToFirst()) {
                val nameIndex = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                if (nameIndex >= 0 && !cursor.isNull(nameIndex)) name = cursor.getString(nameIndex)
                val sizeIndex = cursor.getColumnIndex(OpenableColumns.SIZE)
                if (sizeIndex >= 0 && !cursor.isNull(sizeIndex) && cursor.getLong(sizeIndex) > limit) {
                    throw PluginException(PluginException.TOO_LARGE)
                }
            }
        }
        val nameBytes = (name ?: "document").toByteArray(Charsets.UTF_8)
        val mimeBytes = (resolver.getType(uri) ?: "").toByteArray(Charsets.UTF_8)
        DocumentEncoding.validateMetadata(nameBytes, mimeBytes)
        val content =
            resolver.openInputStream(uri)?.use { stream ->
                DocumentEncoding.readBounded(stream, limit) { currentCoroutineContext().ensureActive() }
            } ?: throw PluginException(PluginException.NATIVE)
        return DocumentEncoding.encode(nameBytes, mimeBytes, content)
    }
}
