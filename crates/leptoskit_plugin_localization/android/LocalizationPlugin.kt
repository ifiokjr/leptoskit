package dev.leptoskit.plugins.localization

import android.content.Context
import android.icu.util.Calendar
import android.text.format.DateFormat
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.TimeZone

class LocalizationPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "localization"
    override val methods = intArrayOf(0)

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (method != 0) throw PluginException(PluginException.UNAVAILABLE)
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
        val locales = context.resources.configuration.locales
        if (locales.size() > 64) throw PluginException(PluginException.TOO_LARGE)
        val output = PreferenceEncoder()
        output.integer(1)
        output.integer(locales.size())
        for (index in 0 until locales.size()) {
            val locale = locales[index]
            output.text(locale.toLanguageTag())
            output.text(locale.language)
            output.text(locale.country)
        }
        val calendar = if (locales.isEmpty) Calendar.getInstance() else Calendar.getInstance(locales[0])
        output.text(if (calendar.type == "gregorian") "gregory" else calendar.type)
        output.text(TimeZone.getDefault().id)
        output.integer(if (DateFormat.is24HourFormat(context)) 2 else 1)
        output.integer((calendar.firstDayOfWeek + 5) % 7 + 1)
        return output.bytes()
    }
}

private class PreferenceEncoder {
    private val output = ByteArrayOutputStream(128)

    fun integer(value: Int) {
        output.write(
            ByteBuffer
                .allocate(4)
                .order(ByteOrder.LITTLE_ENDIAN)
                .putInt(value)
                .array(),
        )
    }

    fun text(value: String?) {
        val bytes = (value ?: "").toByteArray(Charsets.UTF_8)
        if (bytes.size > 256) throw PluginException(PluginException.TOO_LARGE)
        integer(bytes.size)
        output.write(bytes)
    }

    fun bytes(): ByteArray = output.toByteArray()
}
