package dev.leptoskit.mobile

import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.ImageView
import android.widget.TextView
import org.json.JSONArray
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

        if (widget is NativeVectorView) applyVector(widget, values)
    }

    /** A labelled drawing is an image to accessibility services; an unlabelled one is decorative. */
    private fun applyVector(
        widget: NativeVectorView,
        values: Values,
    ) {
        widget.importantForAccessibility =
            if (values.string("accessibility_label").isNullOrEmpty()) {
                View.IMPORTANT_FOR_ACCESSIBILITY_NO
            } else {
                View.IMPORTANT_FOR_ACCESSIBILITY_YES
            }
        widget.drawing = values.drawing("drawing")
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

        fun drawing(key: String): NativeVectorDrawing? =
            if (props.isNull(key)) null else NativeVectorDrawing.decode(props.getJSONObject(key))

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

/**
 * A `Vector`'s `drawing` prop: outlines in view-box coordinates, mapped onto the frame at draw time.
 *
 * Path data arrives canonical (absolute `M`, `L`, `Q`, `C` and `Z`, one space between tokens), so it
 * splits on spaces instead of parsing SVG. Malformed data fails the render batch.
 */
internal class NativeVectorDrawing(
    val viewBox: RectF,
    val stretch: Boolean,
    val paths: List<NativeVectorPath>,
) {
    /** Maps view-box coordinates onto a [width] by [height] frame whose origin is its top-left corner. */
    fun matrix(
        width: Float,
        height: Float,
    ): Matrix {
        val frameWidth = width.coerceAtLeast(0f)
        val frameHeight = height.coerceAtLeast(0f)
        val scaleX = frameWidth / viewBox.width()
        val scaleY = frameHeight / viewBox.height()
        val matrix = Matrix()

        if (stretch) {
            matrix.setScale(scaleX, scaleY)
            matrix.postTranslate(-viewBox.left * scaleX, -viewBox.top * scaleY)
        } else {
            val scale = minOf(scaleX, scaleY)
            matrix.setScale(scale, scale)
            matrix.postTranslate(
                (frameWidth - viewBox.width() * scale) / 2 - viewBox.left * scale,
                (frameHeight - viewBox.height() * scale) / 2 - viewBox.top * scale,
            )
        }

        return matrix
    }

    companion object {
        fun decode(drawing: JSONObject): NativeVectorDrawing {
            val box = drawing.getJSONObject("view_box")
            val minX = box.getDouble("min_x").toFloat()
            val minY = box.getDouble("min_y").toFloat()
            val width = box.getDouble("width").toFloat()
            val height = box.getDouble("height").toFloat()
            require(listOf(minX, minY, width, height).all { it.isFinite() } && width > 0f && height > 0f) {
                "A view box needs a positive, finite size: $box"
            }

            val scaling = drawing.optString("scaling", "fit")
            require(scaling == "fit" || scaling == "stretch") { "Unknown vector scaling: $scaling" }
            val paths = drawing.optJSONArray("paths") ?: JSONArray()

            return NativeVectorDrawing(
                RectF(minX, minY, minX + width, minY + height),
                scaling == "stretch",
                List(paths.length()) { NativeVectorPath.decode(paths.getJSONObject(it)) },
            )
        }
    }
}

/** One outline painted with an optional fill, then an optional stroke. */
internal class NativeVectorPath(
    val outline: Path,
    val fill: Int?,
    val stroke: NativeVectorStroke?,
) {
    companion object {
        fun decode(path: JSONObject): NativeVectorPath =
            NativeVectorPath(
                outline(path.getString("data")),
                if (path.isNull("fill")) null else NativeWidgetColor.argb(path.getLong("fill")),
                if (path.isNull("stroke")) null else NativeVectorStroke.decode(path.getJSONObject("stroke")),
            )

        /** Builds canonical path data in view-box coordinates. */
        fun outline(data: String): Path {
            val path = Path()
            val tokens = data.split(' ').filter { it.isNotEmpty() }
            var index = 0

            fun number(): Float {
                val value = tokens.getOrNull(index++)?.toFloatOrNull()
                require(value != null && value.isFinite()) { "Invalid vector path data: $data" }
                return value
            }

            while (index < tokens.size) {
                val command = tokens[index++]
                require(command == "M" || index > 1) { "Vector path data must start with M: $data" }

                when (command) {
                    "M" -> path.moveTo(number(), number())
                    "L" -> path.lineTo(number(), number())
                    "Q" -> path.quadTo(number(), number(), number(), number())
                    "C" -> path.cubicTo(number(), number(), number(), number(), number(), number())
                    "Z" -> path.close()
                    else -> throw IllegalArgumentException("Unsupported vector path command $command: $data")
                }
            }

            return path
        }
    }
}

/** A stroke in layout units, scaled by the display density but never by the drawing. */
internal class NativeVectorStroke(
    val color: Int,
    val width: Float,
    val cap: Paint.Cap,
    val join: Paint.Join,
) {
    companion object {
        /** Strokes miter until the miter length reaches four times the line width. */
        const val MITER_LIMIT = 4f

        fun decode(stroke: JSONObject): NativeVectorStroke {
            val width = stroke.getDouble("width").toFloat()
            require(width.isFinite() && width > 0f) { "A stroke width must be finite and positive: $width" }

            return NativeVectorStroke(
                NativeWidgetColor.argb(stroke.getLong("color")),
                width,
                when (val cap = stroke.optString("cap", "butt")) {
                    "butt" -> Paint.Cap.BUTT
                    "round" -> Paint.Cap.ROUND
                    "square" -> Paint.Cap.SQUARE
                    else -> throw IllegalArgumentException("Unknown stroke cap: $cap")
                },
                when (val join = stroke.optString("join", "miter")) {
                    "miter" -> Paint.Join.MITER
                    "round" -> Paint.Join.ROUND
                    "bevel" -> Paint.Join.BEVEL
                    else -> throw IllegalArgumentException("Unknown stroke join: $join")
                },
            )
        }
    }
}
