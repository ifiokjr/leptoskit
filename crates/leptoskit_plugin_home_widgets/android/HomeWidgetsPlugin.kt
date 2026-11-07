package dev.leptoskit.plugins.homewidgets

import android.appwidget.AppWidgetManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Looper
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction
import java.nio.charset.StandardCharsets

/**
 * Publishes snapshot bytes for Android App Widgets and asks AppWidgetManager to
 * refresh them. Does not declare or draw a widget provider; the app still owns
 * its AppWidgetProvider subclasses.
 */
class HomeWidgetsPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "home-widgets"
    override val methods = intArrayOf(CAPABILITY, SET, GET, CLEAR, RELOAD, INSTALLED)

    private val prefs = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    private var closed = false

    override fun close() {
        closed = true
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper())
        if (closed) throw PluginException(PluginException.CANCELLED)
        currentCoroutineContext().ensureActive()

        return when (method) {
            CAPABILITY -> {
                empty(payload)
                capability()
            }
            SET -> {
                val snapshot = Snapshot.decode(payload)
                // Store raw wire bytes so GET round-trips the guest payload.
                prefs
                    .edit()
                    .putString(PREFIX + snapshot.kind, snapshot.encodedBase64())
                    .apply()
                byteArrayOf()
            }
            GET -> {
                val kind = Kind.decode(payload)
                val encoded =
                    prefs.getString(PREFIX + kind, null)
                        ?: throw PluginException(PluginException.UNAVAILABLE)
                android.util.Base64.decode(encoded, android.util.Base64.NO_WRAP)
            }
            CLEAR -> {
                val kind = Kind.decode(payload)
                prefs.edit().remove(PREFIX + kind).apply()
                byteArrayOf()
            }
            RELOAD -> {
                val kind = optionalKind(payload)
                reload(kind)
                byteArrayOf()
            }
            INSTALLED -> {
                val kind = Kind.decode(payload)
                installed(kind).toByteArray()
            }
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }
    }

    private fun capability(): ByteArray {
        // supported | can_reload | can_query_installed | shared_store
        val flags = 1 or (1 shl 1) or (1 shl 2) or (1 shl 3)
        return ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(flags).array()
    }

    private fun reload(kind: String?) {
        val manager = AppWidgetManager.getInstance(context) ?: throw PluginException(PluginException.UNAVAILABLE)
        val providers = providers()
        if (providers.isEmpty()) throw PluginException(PluginException.UNAVAILABLE)
        for (provider in providers) {
            if (kind != null && !provider.className.contains(kind, ignoreCase = true)) {
                // Kind filtering is best-effort by provider class name; apps that
                // need exact mapping should name providers after their kinds.
                continue
            }
            val ids = manager.getAppWidgetIds(provider)
            if (ids.isEmpty()) continue
            val intent =
                Intent(AppWidgetManager.ACTION_APPWIDGET_UPDATE).apply {
                    component = provider
                    putExtra(AppWidgetManager.EXTRA_APPWIDGET_IDS, ids)
                }
            context.sendBroadcast(intent)
        }
        if (kind != null) {
            val matched =
                providers.any { provider ->
                    provider.className.contains(kind, ignoreCase = true) &&
                        manager.getAppWidgetIds(provider).isNotEmpty()
                }
            // Reload with an unknown kind is still accepted when no providers match:
            // the snapshot may be waiting for a future pin.
            if (!matched && providers.none { it.className.contains(kind, ignoreCase = true) }) {
                // Fall through: still accepted. Snapshot storage is the source of truth.
            }
        }
    }

    private fun installed(kind: String): Int {
        val manager = AppWidgetManager.getInstance(context) ?: return INSTALLED_UNKNOWN
        val providers = providers()
        if (providers.isEmpty()) return INSTALLED_NONE
        val matching =
            providers.filter { it.className.contains(kind, ignoreCase = true) }.ifEmpty { providers }
        return if (matching.any { manager.getAppWidgetIds(it).isNotEmpty() }) {
            INSTALLED_SOME
        } else {
            INSTALLED_NONE
        }
    }

    private fun providers(): List<ComponentName> {
        val intent = Intent(AppWidgetManager.ACTION_APPWIDGET_UPDATE).setPackage(context.packageName)
        val flags =
            if (android.os.Build.VERSION.SDK_INT >= 33) {
                PackageManager.ResolveInfoFlags.of(PackageManager.GET_RECEIVERS.toLong())
            } else {
                null
            }
        val resolved =
            if (flags != null) {
                context.packageManager.queryBroadcastReceivers(intent, flags)
            } else {
                @Suppress("DEPRECATION")
                context.packageManager.queryBroadcastReceivers(intent, PackageManager.GET_RECEIVERS)
            }
        return resolved.mapNotNull { info ->
            val receiver = info.activityInfo ?: return@mapNotNull null
            ComponentName(receiver.packageName, receiver.name)
        }
    }

    private companion object {
        const val CAPABILITY = 0
        const val SET = 1
        const val GET = 2
        const val CLEAR = 3
        const val RELOAD = 4
        const val INSTALLED = 5
        const val PREFS = "leptoskit.home_widgets"
        const val PREFIX = "leptoskit.home-widgets."
        const val INSTALLED_UNKNOWN = 0
        const val INSTALLED_NONE = 1
        const val INSTALLED_SOME = 2
        const val MAX_SNAPSHOT = 16 + 64 + 256 + 1024 + 2048
    }
}

private fun empty(payload: ByteBuffer) {
    if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
}

private fun optionalKind(payload: ByteBuffer): String? {
    if (!payload.hasRemaining()) return null
    return Kind.decode(payload)
}

private fun Int.toByteArray(): ByteArray =
    ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(this).array()

private object Kind {
    fun decode(payload: ByteBuffer): String {
        val reader = Reader(payload, 68)
        val kind = reader.identifier()
        reader.finish()
        return kind
    }
}

private class Snapshot(
    val kind: String,
    val raw: ByteArray,
) {
    fun encodedBase64(): String = android.util.Base64.encodeToString(raw, android.util.Base64.NO_WRAP)

    companion object {
        fun decode(payload: ByteBuffer): Snapshot {
            val start = payload.position()
            val reader = Reader(payload, MAX_SNAPSHOT)
            val kind = reader.identifier()
            val family = reader.u32()
            if (family !in 0..4) throw PluginException(PluginException.INVALID_ARGUMENT)
            reader.text(256, multiline = false, required = true)
            reader.text(1024, multiline = true, required = false)
            val url = reader.text(2048, multiline = false, required = false)
            if (url.isNotEmpty() && (!url.contains(':') || url.any { it.code < 32 || it.code == 127 })) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }
            reader.finish()
            val end = payload.position()
            val raw = ByteArray(end - start)
            payload.duplicate().also {
                it.position(start)
                it.limit(end)
                it.get(raw)
            }
            return Snapshot(kind, raw)
        }

        private const val MAX_SNAPSHOT = 16 + 64 + 256 + 1024 + 2048
    }
}

private class Reader(
    private val buffer: ByteBuffer,
    maximum: Int,
) {
    private val start = buffer.position()
    private val endExclusive = start + buffer.remaining()

    init {
        if (buffer.remaining() > maximum) throw PluginException(PluginException.INVALID_ARGUMENT)
        buffer.order(ByteOrder.LITTLE_ENDIAN)
    }

    fun u32(): Int {
        if (buffer.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        return buffer.int
    }

    fun identifier(): String {
        val value = text(64, multiline = false, required = true)
        if (!value.all { it.isLetterOrDigit() || it == '-' || it == '_' || it == '.' }) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
        return value
    }

    fun text(
        maximum: Int,
        multiline: Boolean,
        required: Boolean,
    ): String {
        if (buffer.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        val length = buffer.int
        if (length < 0 || length > maximum || length > buffer.remaining()) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
        val bytes = ByteArray(length)
        buffer.get(bytes)
        val value =
            try {
                StandardCharsets.UTF_8
                    .newDecoder()
                    .onMalformedInput(CodingErrorAction.REPORT)
                    .onUnmappableCharacter(CodingErrorAction.REPORT)
                    .decode(ByteBuffer.wrap(bytes))
                    .toString()
            } catch (_: CharacterCodingException) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }
        if (required && value.trim().isEmpty()) throw PluginException(PluginException.INVALID_ARGUMENT)
        if (value.any { ch ->
                val code = ch.code
                (code <= 31 || code in 127..159) && !(multiline && (code == 10 || code == 9))
            }
        ) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
        return value
    }

    fun finish() {
        if (buffer.position() != endExclusive) throw PluginException(PluginException.INVALID_ARGUMENT)
        // Silence unused start warning in some Kotlin versions.
        if (start > endExclusive) throw PluginException(PluginException.INVALID_ARGUMENT)
    }
}
