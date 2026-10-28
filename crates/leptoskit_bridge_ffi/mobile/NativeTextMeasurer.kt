package dev.leptoskit.mobile

import android.graphics.Typeface
import android.text.Layout
import android.text.StaticLayout
import android.text.TextDirectionHeuristics
import android.text.TextPaint
import android.util.DisplayMetrics
import android.util.TypedValue
import kotlin.math.ceil
import kotlin.math.max

/**
 * Measures the shared tree's text with the typeface and size [NativeWidgetProps]
 * draws it with, so layout gives each label and button the width Android needs.
 * The native library calls [measure] by name on whichever thread lays out.
 */
internal class NativeTextMeasurer(
    private val metrics: DisplayMetrics,
) {
    /**
     * Returns the logical width and height of UTF-8 [text] at [fontSize] (in the
     * scaled points text views use), CSS [weight], and [rtl] direction within
     * [width] logical points: `-1` is unbounded and `-2` is the widest word.
     */
    @Suppress("unused") // Called by JNI.
    fun measure(
        text: ByteArray,
        fontSize: Float,
        weight: Int,
        width: Float,
        rtl: Boolean,
    ): FloatArray {
        val value = text.toString(Charsets.UTF_8)
        val paint =
            TextPaint(TextPaint.ANTI_ALIAS_FLAG).apply {
                textSize = TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_SP, fontSize, metrics)
                typeface = Typeface.defaultFromStyle(if (weight >= BOLD_WEIGHT) Typeface.BOLD else Typeface.NORMAL)
            }
        val limit =
            when (width) {
                UNBOUNDED -> Layout.getDesiredWidth(value, paint)
                WIDEST_WORD -> value.split(Regex("\\s+")).maxOfOrNull { Layout.getDesiredWidth(it, paint) } ?: 0f
                else -> width * metrics.density
            }
        val layout =
            StaticLayout.Builder
                .obtain(value, 0, value.length, paint, ceil(limit.toDouble()).toInt().coerceAtLeast(1))
                .setIncludePad(false)
                .setTextDirection(if (rtl) TextDirectionHeuristics.RTL else TextDirectionHeuristics.LTR)
                .setBreakStrategy(Layout.BREAK_STRATEGY_SIMPLE)
                .setHyphenationFrequency(Layout.HYPHENATION_FREQUENCY_NONE)
                .build()
        var widest = 0f
        repeat(layout.lineCount) { widest = max(widest, layout.getLineWidth(it)) }
        val pixels = ceil(widest.coerceAtMost(limit.coerceAtLeast(0f)).toDouble()).toFloat()
        return floatArrayOf(pixels / metrics.density, layout.height / metrics.density)
    }

    private companion object {
        const val UNBOUNDED = -1f
        const val WIDEST_WORD = -2f

        // NativeWidgetProps draws semibold and bold with the bold typeface.
        const val BOLD_WEIGHT = 600
    }
}
