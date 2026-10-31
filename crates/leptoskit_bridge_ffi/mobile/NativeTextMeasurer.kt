package dev.leptoskit.mobile

import android.graphics.Typeface
import android.text.BoringLayout
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
 *
 * Every changed label is measured once per relayout, so a frame that rewrites a
 * screen of numbers measures hundreds of strings: the paint is reused per
 * size and weight, and one-line text takes `BoringLayout`, which skips line
 * breaking entirely.
 */
internal class NativeTextMeasurer(
    private val metrics: DisplayMetrics,
) {
    /** The paint the last measurement used; labels in one list share a style. */
    private var painted: TextPaint? = null
    private var paintedSize = Float.NaN
    private var paintedBold = false

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
        val bold = weight >= BOLD_WEIGHT
        val paint = paint(fontSize, bold)
        val limit =
            when (width) {
                UNBOUNDED -> Layout.getDesiredWidth(value, paint)
                WIDEST_WORD -> value.split(Regex("\\s+")).maxOfOrNull { Layout.getDesiredWidth(it, paint) } ?: 0f
                else -> width * metrics.density
            }
        val bound = ceil(limit.toDouble()).toInt().coerceAtLeast(1)
        val direction = if (rtl) TextDirectionHeuristics.RTL else TextDirectionHeuristics.LTR
        // One line of text without tabs or bidi overrides lays out as a single
        // run; `BoringLayout` measures exactly that without a line breaker.
        val boring = BoringLayout.isBoring(value, paint, direction, true, null)
        val measured =
            if (boring != null) {
                BoringLayout.make(
                    value,
                    paint,
                    bound,
                    Layout.Alignment.ALIGN_NORMAL,
                    1.0f,
                    0.0f,
                    boring,
                    false,
                )
            } else {
                StaticLayout.Builder
                    .obtain(value, 0, value.length, paint, bound)
                    .setIncludePad(false)
                    .setTextDirection(direction)
                    .setBreakStrategy(Layout.BREAK_STRATEGY_SIMPLE)
                    .setHyphenationFrequency(Layout.HYPHENATION_FREQUENCY_NONE)
                    .build()
            }
        var widest = 0f
        repeat(measured.lineCount) { widest = max(widest, measured.getLineWidth(it)) }
        val pixels = ceil(widest.coerceAtMost(limit.coerceAtLeast(0f)).toDouble()).toFloat()
        return floatArrayOf(pixels / metrics.density, measured.height / metrics.density)
    }

    private fun paint(
        size: Float,
        bold: Boolean,
    ): TextPaint {
        val existing = painted

        if (existing != null && size == paintedSize && bold == paintedBold) return existing

        val paint =
            (existing ?: TextPaint(TextPaint.ANTI_ALIAS_FLAG)).apply {
                textSize = TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_SP, size, metrics)
                typeface = Typeface.defaultFromStyle(if (bold) Typeface.BOLD else Typeface.NORMAL)
                isAntiAlias = true
            }
        painted = paint
        paintedSize = size
        paintedBold = bold
        return paint
    }

    private companion object {
        const val UNBOUNDED = -1f
        const val WIDEST_WORD = -2f

        // NativeWidgetProps draws semibold and bold with the bold typeface.
        const val BOLD_WEIGHT = 600
    }
}
