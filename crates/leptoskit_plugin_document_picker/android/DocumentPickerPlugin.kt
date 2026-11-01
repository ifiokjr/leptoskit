package dev.leptoskit.plugins.documentpicker

import android.app.Activity
import android.content.ContentResolver
import android.content.Context
import android.content.Intent
import android.database.Cursor
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
    override val methods = intArrayOf(OPEN)
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

        if (method != OPEN) throw PluginException(PluginException.UNAVAILABLE)

        val limit = readLimit(payload)

        if (busy) throw PluginException(PluginException.BUSY)

        val activity = activity ?: throw PluginException(PluginException.UNAVAILABLE)
        val host = host ?: throw PluginException(PluginException.UNAVAILABLE)
        busy = true
        pendingJob = currentCoroutineContext()[Job]

        try {
            return choose(activity, host, limit)
        } finally {
            busy = false
            pendingJob = null
        }
    }

    /** Reads the caller's byte limit, refusing one this reader cannot honour. */
    private fun readLimit(payload: ByteBuffer): Int {
        if (payload.remaining() != LIMIT_BYTES) throw PluginException(PluginException.INVALID_ARGUMENT)

        val limit = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN).int

        if (limit !in 0..DocumentEncoding.MAX_BYTES) throw PluginException(PluginException.TOO_LARGE)

        return limit
    }

    /** Runs the picker, translating the platform's refusals into plugin errors. */
    private suspend fun choose(
        activity: Activity,
        host: NativeActivityResultHost,
        limit: Int,
    ): ByteArray =
        try {
            pick(activity, host, limit)
        } catch (error: SecurityException) {
            throw PluginException(PluginException.PERMISSION_DENIED, error)
        } catch (error: IOException) {
            throw PluginException(PluginException.NATIVE, error)
        }

    /** Presents the system picker, then reads the one document it returned. */
    private suspend fun pick(
        activity: Activity,
        host: NativeActivityResultHost,
        limit: Int,
    ): ByteArray {
        val result = host.launchPluginActivity(openDocument())

        if (result.resultCode == Activity.RESULT_CANCELED) throw PluginException(PluginException.CANCELLED)

        if (result.resultCode != Activity.RESULT_OK) throw PluginException(PluginException.NATIVE)

        val uri = result.data?.data ?: throw PluginException(PluginException.NATIVE)

        if (uri.scheme != CONTENT_SCHEME) throw PluginException(PluginException.INVALID_ARGUMENT)

        val bytes = withContext(Dispatchers.IO) { read(activity, uri, limit) }

        if (closed) throw PluginException(PluginException.CANCELLED)

        return bytes
    }

    /** Asks for one readable document; the grant lasts only as long as this call. */
    private fun openDocument(): Intent =
        Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "*/*"
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }

    /** Names the document, then copies its content within the caller's limit. */
    private suspend fun read(
        activity: Activity,
        uri: Uri,
        limit: Int,
    ): ByteArray {
        currentCoroutineContext().ensureActive()
        val resolver = activity.contentResolver
        val name = readName(resolver, uri, limit).toByteArray(Charsets.UTF_8)
        val mime = (resolver.getType(uri) ?: "").toByteArray(Charsets.UTF_8)
        DocumentEncoding.validateMetadata(name, mime)
        val content = readContent(resolver, uri, limit)

        return DocumentEncoding.encode(name, mime, content)
    }

    /** Reads the provider's display name, refusing a document larger than the limit. */
    private fun readName(
        resolver: ContentResolver,
        uri: Uri,
        limit: Int,
    ): String {
        val columns = arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE)

        val cursor = resolver.query(uri, columns, null, null, null) ?: return DEFAULT_NAME

        return cursor.use { rowName(it, limit) ?: DEFAULT_NAME }
    }

    /** Reads the provider's single row, refusing a size the caller cannot hold. */
    private fun rowName(
        cursor: Cursor,
        limit: Int,
    ): String? {
        if (!cursor.moveToFirst()) return null

        val sizeIndex = cursor.getColumnIndex(OpenableColumns.SIZE)

        if (sizeIndex >= 0 && !cursor.isNull(sizeIndex) && cursor.getLong(sizeIndex) > limit) {
            throw PluginException(PluginException.TOO_LARGE)
        }

        val nameIndex = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)

        return if (nameIndex >= 0 && !cursor.isNull(nameIndex)) cursor.getString(nameIndex) else null
    }

    /** Reads the content, stopping at the first chunk that would pass the limit. */
    private suspend fun readContent(
        resolver: ContentResolver,
        uri: Uri,
        limit: Int,
    ): ByteArray {
        val stream = resolver.openInputStream(uri) ?: throw PluginException(PluginException.NATIVE)

        return stream.use {
            DocumentEncoding.readBounded(it, limit) { currentCoroutineContext().ensureActive() }
        }
    }

    private companion object {
        /** The only method: present the picker and read one document. */
        const val OPEN = 0

        /** The request carries one u32 byte limit. */
        const val LIMIT_BYTES = 4

        /** Only a content provider URI can be opened through the resolver. */
        const val CONTENT_SCHEME = "content"

        /** The name the reply carries when the provider offers none. */
        const val DEFAULT_NAME = "document"
    }
}
