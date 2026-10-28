package dev.leptoskit.mobile

import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.ImageView
import android.widget.TextView
import org.json.JSONObject

internal class NativeWidgetProps(
    private val density: () -> Float,
) {
    /** Apply shared visual state before control-specific text and image state. */
    fun apply(
        widget: View,
        props: JSONObject,
    ) {
        val values = Values(props)
        widget.contentDescription = values.string("accessibility_label")
        widget.isEnabled = values.boolean("enabled", true)
        widget.visibility = if (values.boolean("hidden", false)) View.INVISIBLE else View.VISIBLE
        widget.alpha = values.number("opacity", 1.0).toFloat()
        widget.background =
            GradientDrawable().apply {
                setColor(values.color("background", android.graphics.Color.TRANSPARENT))
                cornerRadius = values.number("corner_radius", 0.0).toFloat() * density()
                setStroke(
                    (values.number("border_width", 0.0) * density()).toInt(),
                    values.color("border_color", 0),
                )
            }
        widget.clipToOutline =
            values.boolean("clip", false) || values.number("corner_radius", 0.0) > 0

        // A button without paint of its own keeps a platform pressed ripple;
        // painted buttons show their `active:` styles or the shared dim.
        if (widget is PressButton) {
            val painted = !props.isNull("background") || !props.isNull("border_width") || !props.isNull("corner_radius")
            widget.foreground = if (painted) null else widget.ripple
        }

        if (widget is TextView) applyText(widget, values)

        if (widget is ImageView) applyImage(widget, values)
    }

    private fun applyText(
        widget: TextView,
        values: Values,
    ) {
        val value = textValue(widget, values)

        if (widget.text.toString() != value) widget.text = value
        // NativeTextMeasurer sizes text without font padding, as layout expects.
        widget.includeFontPadding = false
        widget.hint = values.string("placeholder")
        widget.textSize = values.number("font_size", DEFAULT_FONT_SIZE).toFloat()
        widget.setTextColor(values.color("color", android.graphics.Color.BLACK))

        val weight =
            if (values.string("font_weight") in listOf("bold", "semibold")) Typeface.BOLD else Typeface.NORMAL
        widget.setTypeface(null, weight)
        // A button centres its label unless the element aligns it, as web
        // buttons and UIKit buttons do.
        val unaligned = if (widget is Button) Gravity.CENTER else Gravity.START or Gravity.CENTER_VERTICAL
        widget.gravity =
            when (values.string("text_align")) {
                "center" -> Gravity.CENTER
                "end" -> Gravity.END or Gravity.CENTER_VERTICAL
                "start" -> Gravity.START or Gravity.CENTER_VERTICAL
                else -> unaligned
            }
    }

    private fun applyImage(
        widget: ImageView,
        values: Values,
    ) {
        val source = values.string("source")

        if (source == null) {
            widget.setImageDrawable(null)
        } else {
            val uri = android.net.Uri.parse(source)
            require(uri.scheme == "file") { "Image requires a readable file URL: $source" }
            widget.setImageURI(uri)
            check(widget.drawable != null) { "Cannot load image: $source" }
        }
    }

    private fun textValue(
        widget: TextView,
        values: Values,
    ): String {
        if (widget is EditText) return values.string("value") ?: values.string("text") ?: ""
        return values.string("text") ?: ""
    }

    private companion object {
        const val DEFAULT_FONT_SIZE = 14.0
    }

    private class Values(
        private val props: JSONObject,
    ) {
        /** Missing and JSON-null properties share the renderer's declared defaults. */
        fun string(key: String): String? = if (props.isNull(key)) null else props.getString(key)

        fun number(
            key: String,
            fallback: Double,
        ): Double = if (props.isNull(key)) fallback else props.getDouble(key)

        fun boolean(
            key: String,
            fallback: Boolean,
        ): Boolean = if (props.isNull(key)) fallback else props.getBoolean(key)

        fun color(
            key: String,
            fallback: Int,
        ): Int {
            if (props.isNull(key)) return fallback
            val rgba = props.getLong(key)

            return NativeWidgetColor.argb(rgba)
        }
    }
}

/** Native render colors use RGBA bytes; Android controls consume ARGB integers. */
internal object NativeWidgetColor {
    private const val ALPHA_SHIFT = 24
    private const val RGB_SHIFT = 8
    private const val COMPONENT_MASK = 255L

    fun argb(rgba: Long): Int = ((rgba ushr RGB_SHIFT) or ((rgba and COMPONENT_MASK) shl ALPHA_SHIFT)).toInt()
}
