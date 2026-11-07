package dev.leptoskit.plugins.appshortcuts

import android.content.Context
import android.content.pm.ShortcutInfo
import android.content.pm.ShortcutManager
import android.os.Build
import android.os.Looper
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction
import java.nio.charset.StandardCharsets

/** Publishes dynamic launcher shortcuts through ShortcutManager. */
class AppShortcutsPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "app-shortcuts"
    override val methods = intArrayOf(SET, CLEAR, CONSUME_LAUNCH)

    private var closed = false
    private var launchId: String? = null

    /** Records that the OS launched the app through this shortcut id. */
    fun noteLaunch(id: String) {
        validateId(id)
        launchId = id
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        checkMain()
        if (closed) throw PluginException(PluginException.CANCELLED)
        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)

        return when (method) {
            SET -> {
                set(decodeList(payload))
                byteArrayOf()
            }
            CLEAR -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
                clear()
                byteArrayOf()
            }
            else -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
                encodeLaunch(launchId.also { launchId = null })
            }
        }
    }

    override fun close() {
        closed = true
    }

    private fun set(items: List<Shortcut>) {
        if (Build.VERSION.SDK_INT < 25) throw PluginException(PluginException.UNAVAILABLE)
        val manager =
            context.getSystemService(ShortcutManager::class.java)
                ?: throw PluginException(PluginException.UNAVAILABLE)
        val shortcuts =
            items.map { item ->
                ShortcutInfo
                    .Builder(context, item.id)
                    .setShortLabel(item.title)
                    .setLongLabel(item.subtitle.ifEmpty { item.title })
                    .setIntent(
                        context.packageManager.getLaunchIntentForPackage(context.packageName)
                            ?: throw PluginException(PluginException.UNAVAILABLE),
                    ).build()
            }
        manager.dynamicShortcuts = shortcuts
    }

    private fun clear() {
        if (Build.VERSION.SDK_INT < 25) throw PluginException(PluginException.UNAVAILABLE)
        val manager =
            context.getSystemService(ShortcutManager::class.java)
                ?: throw PluginException(PluginException.UNAVAILABLE)
        manager.removeAllDynamicShortcuts()
    }

    private data class Shortcut(
        val id: String,
        val title: String,
        val subtitle: String,
    )

    private fun decodeList(payload: ByteBuffer): List<Shortcut> {
        val buffer = payload.order(ByteOrder.LITTLE_ENDIAN)
        if (buffer.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        val count = buffer.int
        if (count !in 0..8) throw PluginException(PluginException.TOO_LARGE)
        val items = ArrayList<Shortcut>(count)
        val seen = HashSet<String>()
        repeat(count) {
            val id = readString(buffer, 64)
            val title = readString(buffer, 128)
            val subtitle = readString(buffer, 128)
            validateId(id)
            validateText(title, allowEmpty = false)
            validateText(subtitle, allowEmpty = true)
            if (!seen.add(id)) throw PluginException(PluginException.INVALID_ARGUMENT)
            items.add(Shortcut(id, title, subtitle))
        }
        if (buffer.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
        return items
    }

    private fun encodeLaunch(id: String?): ByteArray {
        val value = id ?: ""
        if (value.isNotEmpty()) validateId(value)
        val bytes = value.toByteArray(StandardCharsets.UTF_8)
        val out = ByteBuffer.allocate(4 + bytes.size).order(ByteOrder.LITTLE_ENDIAN)
        out.putInt(bytes.size)
        out.put(bytes)
        return out.array()
    }

    private fun readString(
        buffer: ByteBuffer,
        max: Int,
    ): String {
        if (buffer.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        val length = buffer.int
        if (length !in 0..max || buffer.remaining() < length) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
        val bytes = ByteArray(length)
        buffer.get(bytes)
        val decoder =
            StandardCharsets.UTF_8
                .newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
        return try {
            decoder.decode(ByteBuffer.wrap(bytes)).toString()
        } catch (_: CharacterCodingException) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
    }

    private fun validateId(id: String) {
        if (id.isEmpty() || id.length > 64) throw PluginException(PluginException.INVALID_ARGUMENT)
        if (!id.all { it.isLetterOrDigit() || it == '_' || it == '-' || it == '.' }) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
    }

    private fun validateText(
        value: String,
        allowEmpty: Boolean,
    ) {
        if ((!allowEmpty && value.isEmpty()) || value.length > 128) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
        if (value.any { Character.isISOControl(it) }) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
    }

    private fun checkMain() {
        check(Looper.myLooper() == Looper.getMainLooper())
    }

    private companion object {
        const val SET = 0
        const val CLEAR = 1
        const val CONSUME_LAUNCH = 2
    }
}
