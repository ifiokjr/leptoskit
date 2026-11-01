package dev.leptoskit.plugins.localization

import android.content.Context
import android.icu.util.Calendar
import android.text.format.DateFormat
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.Locale
import java.util.TimeZone

class LocalizationPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "localization"
    override val methods = intArrayOf(STATUS)

    /** Answers the one method: the device's locale preferences. */
    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (method != STATUS) throw PluginException(PluginException.UNAVAILABLE)

        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        return encode()
    }

    /** Packs every configured locale, then the calendar and formatting choices. */
    private fun encode(): ByteArray {
        val locales = context.resources.configuration.locales

        if (locales.size() > MAX_LOCALES) throw PluginException(PluginException.TOO_LARGE)
        val output = PreferenceEncoder()
        output.integer(VERSION_ONE)
        output.integer(locales.size())

        for (index in 0 until locales.size()) {
            writeLocale(output, locales[index])
        }

        val calendar = if (locales.isEmpty) Calendar.getInstance() else Calendar.getInstance(locales[0])

        output.text(if (calendar.type == "gregorian") "gregory" else calendar.type)
        output.text(TimeZone.getDefault().id)
        output.integer(if (DateFormat.is24HourFormat(context)) CLOCK_24H else CLOCK_12H)
        output.integer(weekStart(calendar.firstDayOfWeek))

        return output.bytes()
    }

    /** Writes one locale's tag, language, and country. */
    private fun writeLocale(
        output: PreferenceEncoder,
        locale: Locale,
    ) {
        output.text(locale.toLanguageTag())
        output.text(locale.language)
        output.text(locale.country)
    }

    /** The locale's first weekday, counted from Sunday as the wire format asks. */
    private fun weekStart(icuFirstDay: Int): Int = (icuFirstDay + ICU_WEEK_START) % WEEK_DAYS + 1

    private companion object {
        /** The only method: the device's locale preferences. */
        const val STATUS = 0

        /** The wire format version every reply carries. */
        const val VERSION_ONE = 1

        /** How many locales a reply may carry. */
        const val MAX_LOCALES = 64

        /** `DateFormat` reports a 24-hour clock as 2, and a 12-hour one as 1. */
        const val CLOCK_24H = 2
        const val CLOCK_12H = 1

        /** ICU counts Sunday as 1, where the wire format counts it as 0. */
        const val ICU_WEEK_START = 5

        /** Days in a week, which the first-weekday field wraps at. */
        const val WEEK_DAYS = 7
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
