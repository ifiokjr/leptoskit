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
import android.widget.ScrollView
import android.widget.TextView
import org.json.JSONArray
import org.json.JSONObject
import java.util.WeakHashMap

internal class NativeWidgetProps(
    private val density: () -> Float,
    /** Called for a scroll view a batch asked to move, so the renderer can apply it once laid out. */
    private val scrolled: (ScrollView) -> Unit = {},
) {
    /** The backdrop each view last received, so an unchanged one is not drawn again. */
    private val backdrops = WeakHashMap<View, Backdrop>()

    /** The text style each label last received. */
    private val textStyles = WeakHashMap<TextView, TextStyle>()

    /**
     * Apply shared visual state before control-specific text and image state.
     *
     * Each property is set only when it changes: a new or reused view
     * already holds most values, and every Android setter invalidates or
     * re-lays out the view, which added up across a screen of views.
     */
    fun apply(
        widget: View,
        props: WidgetProps,
    ) {
        val description = props.accessibilityLabel
        if (widget.contentDescription?.toString() != description) widget.contentDescription = description
        val enabled = props.enabled ?: true
        if (widget.isEnabled != enabled) widget.isEnabled = enabled
        val visibility = if (props.hidden == true) View.INVISIBLE else View.VISIBLE
        if (widget.visibility != visibility) widget.visibility = visibility
        val alpha = (props.opacity ?: 1.0).toFloat()
        if (widget.alpha != alpha) widget.alpha = alpha
        val clip = props.clip == true || (props.cornerRadius ?: 0.0) > 0
        backdrop(widget, props, clip)
        if (widget.clipToOutline != clip) widget.clipToOutline = clip

        // A button without paint of its own keeps a platform pressed ripple;
        // painted buttons show their `active:` styles or the shared dim.
        if (widget is PressButton) {
            val painted = props.background != null || props.borderWidth != null || props.cornerRadius != null
            widget.foreground = if (painted) null else widget.ripple
        }

        if (widget is TextView) applyText(widget, props)

        if (widget is NativeRiveImage) {
            widget.scaleType = ImageView.ScaleType.FIT_CENTER
            widget.importantForAccessibility =
                if (props.rest?.optJSONObject("aria")?.optBoolean("aria_hidden", false) == true) {
                    View.IMPORTANT_FOR_ACCESSIBILITY_NO
                } else {
                    View.IMPORTANT_FOR_ACCESSIBILITY_YES
                }
        } else if (widget is ImageView) {
            applyImage(widget, props)
        }

        if (widget is NativeVectorView) applyVector(widget, props)

        if (widget is ScrollView) applyScroll(widget, props)
    }

    /**
     * Records the offset a scroll view's props ask for.
     *
     * The move itself waits until the view has been laid out: a scroll view
     * clamps to the content it has, and it has none before its first layout.
     * The last command each view received is remembered, so an unrelated
     * property change never re-applies an offset the user has scrolled away
     * from.
     */
    private fun applyScroll(
        widget: ScrollView,
        props: WidgetProps,
    ) {
        val json = props.rest?.optJSONObject("scroll") ?: return
        val requested =
            ScrollCommand(json.getDouble("x"), json.getDouble("y"), json.optBoolean("animated", false))

        if (appliedScrolls[widget] == requested) return

        appliedScrolls[widget] = requested
        val left = (requested.x * density()).toInt()
        val top = (requested.y * density()).toInt()

        if (requested.animated) widget.smoothScrollTo(left, top) else widget.scrollTo(left, top)
        scrolled(widget)
    }

    private data class ScrollCommand(
        val x: Double,
        val y: Double,
        val animated: Boolean,
    )

    private val appliedScrolls = WeakHashMap<ScrollView, ScrollCommand>()

    /**
     * Gives the view a background only when it draws something or clips to
     * its outline, reusing the drawable it already has: allocating one for
     * every view, transparent or not, was most of creating a screen.
     */
    private fun backdrop(
        widget: View,
        props: WidgetProps,
        clip: Boolean,
    ) {
        val backdrop =
            Backdrop(
                color = props.background?.let(NativeWidgetColor::argb) ?: android.graphics.Color.TRANSPARENT,
                radius = (props.cornerRadius ?: 0.0).toFloat() * density(),
                strokeWidth = ((props.borderWidth ?: 0.0) * density()).toInt(),
                strokeColor = props.borderColor?.let(NativeWidgetColor::argb) ?: 0,
            )

        if (backdrops[widget] == backdrop) return
        backdrops[widget] = backdrop

        widget.background =
            if (!backdrop.draws && !clip) {
                null
            } else {
                (widget.background as? GradientDrawable ?: GradientDrawable()).apply {
                    setColor(backdrop.color)
                    cornerRadius = backdrop.radius
                    setStroke(backdrop.strokeWidth, backdrop.strokeColor)
                }
            }
    }

    /** A view's background color, corner radius and border. */
    private data class Backdrop(
        val color: Int,
        val radius: Float,
        val strokeWidth: Int,
        val strokeColor: Int,
    ) {
        val draws get() = color != android.graphics.Color.TRANSPARENT || radius > 0 || strokeWidth > 0
    }

    /** A label's size, color, weight and alignment. */
    private data class TextStyle(
        val size: Float,
        val color: Int,
        val weight: Int,
        val gravity: Int,
    )

    /** A labelled drawing is an image to accessibility services; an unlabelled one is decorative. */
    private fun applyVector(
        widget: NativeVectorView,
        props: WidgetProps,
    ) {
        widget.importantForAccessibility =
            if (props.accessibilityLabel.isNullOrEmpty()) {
                View.IMPORTANT_FOR_ACCESSIBILITY_NO
            } else {
                View.IMPORTANT_FOR_ACCESSIBILITY_YES
            }
        val rest = props.rest
        widget.drawing =
            if (rest == null || rest.isNull("drawing")) null else NativeVectorDrawing.decode(rest.getJSONObject("drawing"))
    }

    private fun applyText(
        widget: TextView,
        props: WidgetProps,
    ) {
        val value = textValue(widget, props)

        if (widget.text.toString() != value) widget.text = value
        // NativeTextMeasurer sizes text without font padding, as layout expects.
        if (widget.includeFontPadding) widget.includeFontPadding = false
        val hint = props.placeholder
        if (widget.hint?.toString() != hint) widget.hint = hint

        val weight =
            if (props.fontWeight == "bold" || props.fontWeight == "semibold") Typeface.BOLD else Typeface.NORMAL
        // A button centres its label unless the element aligns it, as web
        // buttons and UIKit buttons do.
        val unaligned = if (widget is Button) Gravity.CENTER else Gravity.START or Gravity.CENTER_VERTICAL
        val style =
            TextStyle(
                size = (props.fontSize ?: DEFAULT_FONT_SIZE).toFloat(),
                color = props.color?.let(NativeWidgetColor::argb) ?: android.graphics.Color.BLACK,
                weight = weight,
                gravity =
                    when (props.textAlign) {
                        "center" -> Gravity.CENTER
                        "end" -> Gravity.END or Gravity.CENTER_VERTICAL
                        "start" -> Gravity.START or Gravity.CENTER_VERTICAL
                        else -> unaligned
                    },
            )

        if (textStyles[widget] == style) return
        textStyles[widget] = style
        widget.textSize = style.size
        widget.setTextColor(style.color)
        widget.setTypeface(null, style.weight)
        widget.gravity = style.gravity
    }

    private fun applyImage(
        widget: ImageView,
        props: WidgetProps,
    ) {
        val source = props.source

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
        props: WidgetProps,
    ): String {
        if (widget is EditText) return props.value ?: props.text ?: ""
        return props.text ?: ""
    }

    private companion object {
        const val DEFAULT_FONT_SIZE = 14.0
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
 * splits on spaces instead of parsing SVG. Every field but the view box is optional and decodes to its
 * default; malformed data, an unknown keyword, or an invalid number fails the render batch. Drawings
 * compare by value, so an unchanged drawing sent again is not remapped.
 */
internal data class NativeVectorDrawing(
    val viewBox: RectF,
    val scaling: Scaling,
    /** The share of the spare width placed left of a fitted or filled drawing. */
    val alignX: Float,
    /** The share of the spare height placed above a fitted or filled drawing. */
    val alignY: Float,
    val paths: List<NativeVectorPath>,
) {
    /** How the view box maps onto the frame. */
    enum class Scaling {
        /** Uniform and as large as fits, so the whole view box shows: SVG's `meet`. */
        FIT,

        /** Uniform and as small as covers the frame, overflowing it: SVG's `slice`. */
        FILL,

        /** Each axis on its own, filling the frame exactly; alignment does not apply. */
        STRETCH,
    }

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

        if (scaling == Scaling.STRETCH) {
            matrix.setScale(scaleX, scaleY)
            matrix.postTranslate(-viewBox.left * scaleX, -viewBox.top * scaleY)
        } else {
            val scale = if (scaling == Scaling.FIT) minOf(scaleX, scaleY) else maxOf(scaleX, scaleY)
            matrix.setScale(scale, scale)
            matrix.postTranslate(
                (frameWidth - viewBox.width() * scale) * alignX - viewBox.left * scale,
                (frameHeight - viewBox.height() * scale) * alignY - viewBox.top * scale,
            )
        }

        return matrix
    }

    companion object {
        private const val CENTER = 0.5f

        fun decode(drawing: JSONObject): NativeVectorDrawing {
            val box = drawing.getJSONObject("view_box")
            val minX = box.getDouble("min_x").toFloat()
            val minY = box.getDouble("min_y").toFloat()
            val width = box.getDouble("width").toFloat()
            val height = box.getDouble("height").toFloat()
            require(listOf(minX, minY, width, height).all { it.isFinite() } && width > 0f && height > 0f) {
                "A view box needs a positive, finite size: $box"
            }

            val scaling =
                when (val scaling = drawing.optString("scaling", "fit")) {
                    "fit" -> Scaling.FIT
                    "fill" -> Scaling.FILL
                    "stretch" -> Scaling.STRETCH
                    else -> throw IllegalArgumentException("Unknown vector scaling: $scaling")
                }
            val align = if (drawing.isNull("align")) null else drawing.getJSONObject("align")
            val paths = drawing.optJSONArray("paths") ?: JSONArray()

            return NativeVectorDrawing(
                RectF(minX, minY, minX + width, minY + height),
                scaling,
                align?.let { align(it.getString("x")) } ?: CENTER,
                align?.let { align(it.getString("y")) } ?: CENTER,
                List(paths.length()) { NativeVectorPath.decode(paths.getJSONObject(it)) },
            )
        }

        /** The share of the spare space placed before the drawing for an alignment keyword. */
        private fun align(keyword: String): Float =
            when (keyword) {
                "start" -> 0f
                "center" -> CENTER
                "end" -> 1f
                else -> throw IllegalArgumentException("Unknown vector alignment: $keyword")
            }
    }
}

/** One outline painted with an optional fill, then an optional stroke. */
internal data class NativeVectorPath(
    /** The canonical path data. */
    val data: String,
    val fill: Int?,
    /** Which regions of a self-intersecting or nested outline the fill covers. */
    val fillType: Path.FillType,
    val stroke: NativeVectorStroke?,
) {
    /** The outline in view-box coordinates; malformed data fails construction. */
    val outline: Path = parse(data)

    companion object {
        fun decode(path: JSONObject): NativeVectorPath =
            NativeVectorPath(
                path.getString("data"),
                if (path.isNull("fill")) null else NativeWidgetColor.argb(path.getLong("fill")),
                when (val rule = path.optString("fill_rule", "nonzero")) {
                    "nonzero" -> Path.FillType.WINDING
                    "evenodd" -> Path.FillType.EVEN_ODD
                    else -> throw IllegalArgumentException("Unknown fill rule: $rule")
                },
                if (path.isNull("stroke")) null else NativeVectorStroke.decode(path.getJSONObject("stroke")),
            )

        /** Builds canonical path data in view-box coordinates. */
        fun parse(data: String): Path {
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

/** How an outline is stroked: in layout units when fixed, in view-box units when it scales with the drawing. */
internal data class NativeVectorStroke(
    val color: Int,
    val width: Float,
    val cap: Paint.Cap,
    val join: Paint.Join,
    /** The ratio of miter length to line width beyond which a miter join is beveled. */
    val miterLimit: Float,
    /** Alternating dash and gap lengths: empty for a solid line, else an even count. */
    val dashes: List<Float>,
    /** How far into the dash pattern the stroke starts. */
    val dashOffset: Float,
    /** Whether the width and dashes follow the drawing's scale instead of staying in layout units. */
    val scalesWithDrawing: Boolean,
) {
    companion object {
        /** SVG's default miter limit: strokes miter until the miter length reaches four line widths. */
        const val MITER_LIMIT = 4f

        fun decode(stroke: JSONObject): NativeVectorStroke {
            val width = stroke.getDouble("width").toFloat()
            require(width.isFinite() && width > 0f) { "A stroke width must be finite and positive: $width" }

            val miterLimit = stroke.optDouble("miter_limit", MITER_LIMIT.toDouble()).toFloat()
            require(miterLimit.isFinite() && miterLimit >= 1f) { "A miter limit must be finite and at least 1: $stroke" }

            val lengths = if (stroke.isNull("dashes")) JSONArray() else stroke.getJSONArray("dashes")
            val dashes = List(lengths.length()) { lengths.getDouble(it).toFloat() }
            val dashOffset = stroke.optDouble("dash_offset", 0.0).toFloat()
            require(dashOffset.isFinite() && dashes.all { it.isFinite() && it >= 0f }) {
                "Dash lengths must be finite and non-negative, and the dash offset finite: $stroke"
            }

            // As the core's `Stroke::dashes`: an all-zero list is solid, and an odd one repeats to make it even.
            val pattern =
                when {
                    dashes.all { it == 0f } -> emptyList()
                    dashes.size % 2 == 0 -> dashes
                    else -> dashes + dashes
                }

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
                miterLimit,
                pattern,
                if (pattern.isEmpty()) 0f else dashOffset,
                when (val scaling = stroke.optString("scaling", "fixed")) {
                    "fixed" -> false
                    "drawing" -> true
                    else -> throw IllegalArgumentException("Unknown stroke scaling: $scaling")
                },
            )
        }
    }
}
