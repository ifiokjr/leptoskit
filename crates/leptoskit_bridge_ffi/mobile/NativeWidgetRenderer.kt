package dev.leptoskit.mobile

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.DashPathEffect
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.Path
import android.graphics.Rect
import android.graphics.drawable.Drawable
import android.graphics.drawable.GradientDrawable
import android.text.Editable
import android.text.TextWatcher
import android.util.TypedValue
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.ScrollView
import android.widget.TextView
import org.json.JSONObject
import java.nio.ByteBuffer
import kotlin.math.ceil
import kotlin.math.floor

internal class NativeRiveImage(
    context: Context,
) : ImageView(context)

internal class NativeWidgetRenderer(
    private val root: FrameLayout,
    private val event: (Long, Int, String) -> Unit,
    private val canvasChanged: (Int?) -> Unit = {},
) {
    private val context get() = root.context
    private val density get() = root.resources.displayMetrics.density
    private val widgets = mutableMapOf<Long, View>()

    /**
     * Deleted plain containers and labels, kept for the next screen to reuse
     * as React Native does: constructing a `TextView` costs far more than
     * restyling one, and [NativeWidgetProps] sets every property it owns.
     */
    private val recycledViews = ArrayList<FrameLayout>()
    private val recycledTexts = ArrayList<TextView>()
    private val riveBitmaps = mutableMapOf<Long, Bitmap>()
    var onRiveViewportChange: (() -> Unit)? = null
    /** The scroll views a batch asked to move, applied once they are laid out. */
    private val programmaticScrolls = mutableSetOf<ScrollView>()
    private val props = NativeWidgetProps({ density }) { programmaticScrolls.add(it) }
    var applying = false
        private set

    /**
     * The root widget's opaque background. Like a browser painting the whole
     * canvas with the root element's background, the host paints it behind the
     * system bars; null leaves the host's own background.
     */
    var canvas: Int? = null
        private set
    private var rootId: Long? = null

    fun clear() {
        onRiveViewportChange = null
        widgets.values.forEach { (it as? ImageView)?.setImageDrawable(null) }
        riveBitmaps.clear()
        widgets.clear()
        recycledViews.clear()
        recycledTexts.clear()
    }

    fun riveVisible(id: Long): Boolean {
        val view = widgets[id] ?: return false
        return view.isShown && view.getGlobalVisibleRect(Rect())
    }

    /** JNI lends a read-only buffer only for this synchronous upload. */
    fun presentRive(
        id: Long,
        width: Int,
        height: Int,
        pixels: ByteBuffer,
    ): Boolean {
        require(width in 1..4096 && height in 1..4096 && width.toLong() * height <= 4_194_304)
        require(pixels.remaining().toLong() == width.toLong() * height * 4)
        val image = widgets[id] as? NativeRiveImage ?: return false
        var bitmap = riveBitmaps[id]
        if (bitmap == null || bitmap.width != width || bitmap.height != height) {
            bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
            bitmap.density = Bitmap.DENSITY_NONE
            bitmap.setPremultiplied(true)
            riveBitmaps[id] = bitmap
            image.setImageBitmap(bitmap)
        }
        bitmap.copyPixelsFromBuffer(pixels)
        image.invalidate()
        return true
    }

    /** Applies a JSON transaction, as tests and tools write them. */
    fun render(batch: JSONObject) = render(RenderBatch.fromJson(batch))

    fun render(batch: RenderBatch) {
        applying = true
        try {
            for (mutation in batch.mutations) applyMutation(mutation)
            installRoot(batch.root)
            updateCanvas(batch.root)
        } finally {
            applying = false
        }

        updateScrollSizes()
        applyProgrammaticScrolls()
    }

    /**
     * Grows each scroll view's document to the union of its children.
     *
     * The document is a plain container, so it would otherwise stay the
     * viewport's size and leave nothing to scroll.
     */
    private fun updateScrollSizes() {
        for (widget in widgets.values) {
            if (widget !is ScrollView) continue
            val document = widget.getChildAt(0) as? ViewGroup ?: continue
            var extent = 0

            for (index in 0 until document.childCount) {
                // Frames are layout params until the next layout pass, so a
                // child's own bounds still hold the ones it had before.
                val child = document.getChildAt(index)
                val childParams = child.layoutParams as? FrameLayout.LayoutParams
                val bottom =
                    if (childParams != null) childParams.topMargin + childParams.height else child.bottom
                extent = maxOf(extent, bottom)
            }

            val params = document.layoutParams as? FrameLayout.LayoutParams
            // Growing only: before the first layout, every child reports no
            // extent, and shrinking the document to nothing would leave the
            // scroll view with no range at all.
            if (params != null && extent > 0 && extent > params.height) {
                params.height = extent
                document.layoutParams = params
            }
        }
    }

    /**
     * Moves the scroll views a batch asked for, and reports where they landed.
     *
     * The batch runs inside the guest's own render call, so the move happens
     * on a later turn of the main loop: a scroll view needs its first layout
     * before it has any range to scroll, and the guest must never be
     * re-entered while its own render is running.
     */
    private fun applyProgrammaticScrolls() {
        if (programmaticScrolls.isEmpty()) return

        val moved = programmaticScrolls.toList()
        programmaticScrolls.clear()

        // The batch runs inside the guest's own render call, so the reply is
        // delivered on a later turn: re-entering the guest there would alias
        // the state its render is using.
        root.post {
            for (widget in moved) {
                val id = widget.tag as? Long ?: continue
                val density = widget.resources.displayMetrics.density
                event(id, SCROLL, """{"x":${widget.scrollX / density},"y":${widget.scrollY / density}}""")
            }
        }
    }

    private fun updateCanvas(id: Long?) {
        // A batch names its root only when the root changes.
        if (id != null) rootId = id
        val color =
            (rootId?.let { widgets[it] }?.background as? GradientDrawable)
                ?.color
                ?.defaultColor
                ?.takeIf { Color.alpha(it) == OPAQUE }

        if (color == canvas) return
        canvas = color
        canvasChanged(color)
    }

    private fun applyMutation(change: RenderMutation) {
        when (change) {
            is RenderMutation.Create -> {
                create(change)
            }

            is RenderMutation.UpdateProps -> {
                props.apply(widget(change.id), change.props)
            }

            is RenderMutation.UpdateLayout -> {
                layout(change)
            }

            is RenderMutation.InsertChild -> {
                insert(change)
            }

            is RenderMutation.RemoveChild -> {
                detach(widgets[change.child])
            }

            is RenderMutation.Delete -> {
                riveBitmaps.remove(change.id)
                val view = widgets.remove(change.id) ?: return
                if (view is NativeRiveImage) view.setImageDrawable(null)
                detach(view)
                recycle(view)
            }
        }
    }

    /** Keeps a plain container or label, by exact class, for a later create. */
    private fun recycle(view: View) {
        if (view.javaClass == TextView::class.java && recycledTexts.size < RECYCLE_LIMIT) {
            recycledTexts.add(view as TextView)
        } else if (view.javaClass == FrameLayout::class.java &&
            (view as FrameLayout).childCount == 0 &&
            recycledViews.size < RECYCLE_LIMIT
        ) {
            recycledViews.add(view)
        }
    }

    private fun widget(id: Long): View = checkNotNull(widgets[id])

    private fun create(item: RenderMutation.Create) {
        val id = item.id
        val widget =
            when (val kind = item.kind) {
                "view", "View" -> {
                    recycledViews.removeLastOrNull() ?: FrameLayout(context)
                }

                "text", "text_run", "Text", "TextRun" -> {
                    recycledTexts.removeLastOrNull() ?: TextView(context)
                }

                "button", "Button" -> {
                    PressButton(context) { pressed -> event(id, if (pressed) PRESS_IN else PRESS_OUT, "") }.apply {
                        setPadding(0, 0, 0, 0)
                        // Material buttons upper-case their label; the shared
                        // tree's text is shown as written on every platform.
                        isAllCaps = false
                        setOnClickListener { event(id, 0, "") }
                    }
                }

                "pressable", "Pressable" -> {
                    PressFrame(context) { pressed -> event(id, if (pressed) PRESS_IN else PRESS_OUT, "") }.apply {
                        isClickable = true
                        setOnClickListener { event(id, 0, "") }
                    }
                }

                "text_input", "TextInput" -> {
                    textInput(id)
                }

                "scroll_view", "ScrollView" -> {
                    ScrollView(context).apply {
                        addView(FrameLayout(context))
                        setOnScrollChangeListener { _: View, _: Int, left: Int, _: Int, top: Int ->
                            onRiveViewportChange?.invoke()
                            // The guest works in layout units; Android scrolls in pixels.
                            val density = resources.displayMetrics.density
                            event(id, SCROLL, """{"x":${left / density},"y":${top / density}}""")
                        }
                    }
                }

                "image", "Image" -> {
                    ImageView(context)
                }

                "vector", "Vector" -> {
                    NativeVectorView(context)
                }

                "rive", "Rive" -> {
                    NativeRiveImage(context)
                }

                else -> {
                    error("Unsupported widget: $kind")
                }
            }
        widget.tag = id
        widgets[id] = widget
        props.apply(widget, item.props)
    }

    private fun textInput(id: Long): EditText {
        val input = EditText(context)
        input.isSingleLine = true
        input.addTextChangedListener(InputWatcher(id, event))
        input.setOnEditorActionListener { _, _, _ ->
            event(id, SUBMIT, input.text.toString())
            true
        }

        input.setOnFocusChangeListener { _, focused -> event(id, if (focused) FOCUS else BLUR, "") }

        return input
    }

    /**
     * Snap each frame outward to whole pixels. Truncating the size dropped up
     * to a pixel, so text measured to fit its line wrapped its last word.
     */
    private fun layout(item: RenderMutation.UpdateLayout) {
        val x = item.x * density
        val y = item.y * density
        val left = floor(x).toInt()
        val top = floor(y).toInt()
        val width = ceil(x + item.width * density).toInt() - left
        val height = ceil(y + item.height * density).toInt() - top
        val view = widget(item.id)
        val params = view.layoutParams as? FrameLayout.LayoutParams

        // Assigning params requests a layout pass, so an unchanged frame keeps them.
        if (params != null &&
            params.width == width &&
            params.height == height &&
            params.leftMargin == left &&
            params.topMargin == top
        ) {
            return
        }

        view.layoutParams =
            (params ?: FrameLayout.LayoutParams(width, height)).apply {
                this.width = width
                this.height = height
                leftMargin = left
                topMargin = top
            }
    }

    private fun insert(item: RenderMutation.InsertChild) {
        val parent = widget(item.parent) as ViewGroup
        val container = if (parent is ScrollView) parent.getChildAt(0) as ViewGroup else parent
        val child = widget(item.child)
        detach(child)
        // Strokes may overhang a drawing's frame; only the drawing's own clip prop clips them.
        if (child is NativeVectorView) container.clipChildren = false
        container.addView(child, item.index.coerceAtMost(container.childCount))
    }

    private fun detach(child: View?) {
        (child?.parent as? ViewGroup)?.removeView(child)
    }

    private fun installRoot(id: Long?) {
        if (id == null) return
        val child = widget(id)

        if (child.parent === root) return
        detach(child)
        root.addView(child)
    }

    private companion object {
        /** Enough for a screen of a few thousand views; beyond it, views go. */
        const val RECYCLE_LIMIT = 4096
        const val OPAQUE = 255
        const val SUBMIT = 2
        const val FOCUS = 3
        const val BLUR = 4
        const val SCROLL = 5
        /** How many animation frames a scroll waits for its view to be laid out. */
        const val SCROLL_ATTEMPTS = 120
        const val PRESS_IN = 8
        const val PRESS_OUT = 9
    }
}

/** Reports the pressed state Android draws a button's ripple from. */
internal class PressButton(
    context: Context,
    private val pressedChanged: (Boolean) -> Unit,
) : Button(context) {
    /** The theme's borderless ripple, shown while the button draws no paint of its own. */
    val ripple: Drawable? =
        TypedValue().let { value ->
            if (context.theme.resolveAttribute(android.R.attr.selectableItemBackground, value, true)) {
                context.getDrawable(value.resourceId)
            } else {
                null
            }
        }

    override fun setPressed(pressed: Boolean) {
        val changed = pressed != isPressed
        super.setPressed(pressed)
        if (changed) pressedChanged(pressed)
    }
}

/** A clickable container reporting the pressed state Android keeps while it is held. */
internal class PressFrame(
    context: Context,
    private val pressedChanged: (Boolean) -> Unit,
) : FrameLayout(context) {
    override fun setPressed(pressed: Boolean) {
        val changed = pressed != isPressed
        super.setPressed(pressed)
        if (changed) pressedChanged(pressed)
    }
}

private class InputWatcher(
    private val id: Long,
    private val event: (Long, Int, String) -> Unit,
) : TextWatcher {
    override fun beforeTextChanged(
        text: CharSequence?,
        start: Int,
        count: Int,
        after: Int,
    ) {}

    override fun afterTextChanged(text: Editable?) {}

    override fun onTextChanged(
        text: CharSequence?,
        start: Int,
        before: Int,
        count: Int,
    ) {
        event(id, 1, text?.toString() ?: "")
    }
}

/**
 * Draws a `Vector`, remapping its outlines onto the frame only when its drawing or size changes.
 *
 * Remapping builds each frame outline, stroke width and dash effect once, so drawing allocates nothing.
 */
internal class NativeVectorView(
    context: Context,
) : View(context) {
    private val fillPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.FILL }
    private val strokePaint = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.STROKE }

    var drawing: NativeVectorDrawing? = null
        set(value) {
            if (value == field) return
            field = value
            remap()
        }

    /** Each path mapped onto the current frame in pixels, in painting order. */
    var frames: List<NativeVectorFrame> = emptyList()
        private set

    override fun onSizeChanged(
        width: Int,
        height: Int,
        oldWidth: Int,
        oldHeight: Int,
    ) {
        super.onSizeChanged(width, height, oldWidth, oldHeight)
        remap()
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)

        for (index in frames.indices) {
            val frame = frames[index]
            val fill = frame.path.fill
            if (fill != null) {
                fillPaint.color = fill
                canvas.drawPath(frame.outline, fillPaint)
            }

            val stroke = frame.path.stroke ?: continue
            strokePaint.color = stroke.color
            strokePaint.strokeWidth = frame.strokeWidth
            strokePaint.strokeCap = stroke.cap
            strokePaint.strokeJoin = stroke.join
            strokePaint.strokeMiter = stroke.miterLimit
            strokePaint.pathEffect = frame.dashes
            canvas.drawPath(frame.outline, strokePaint)
        }
    }

    override fun getAccessibilityClassName(): CharSequence = ImageView::class.java.name

    /**
     * Maps points before stroking, then sizes each stroke in pixels: by the display density when it is
     * fixed, and by the drawing's own scale when it scales.
     */
    private fun remap() {
        val drawing = drawing
        frames =
            if (drawing == null) {
                emptyList()
            } else {
                val matrix = drawing.matrix(width.toFloat(), height.toFloat())
                val density = resources.displayMetrics.density
                // For a scale-and-translate matrix, a mapped unit radius is sqrt(|scaleX * scaleY|).
                val drawingScale = matrix.mapRadius(1f)
                drawing.paths.map { path ->
                    NativeVectorFrame.map(path, matrix, if (path.stroke?.scalesWithDrawing == true) drawingScale else density)
                }
            }
        invalidate()
    }
}

/** One path mapped onto a view's frame, with its stroke sized in pixels. */
internal class NativeVectorFrame(
    val path: NativeVectorPath,
    /** The outline in pixels, with the path's fill rule. */
    val outline: Path,
    /** The stroke width in pixels; zero when the path has no stroke. */
    val strokeWidth: Float,
    /** The dash pattern in pixels; null for a solid or unstroked path. */
    val dashes: DashPathEffect?,
) {
    companion object {
        /** Maps [path] through [matrix], sizing its stroke at [scale] pixels per stroke unit. */
        fun map(
            path: NativeVectorPath,
            matrix: Matrix,
            scale: Float,
        ): NativeVectorFrame {
            val outline = Path()
            path.outline.transform(matrix, outline)
            outline.fillType = path.fillType
            val stroke = path.stroke ?: return NativeVectorFrame(path, outline, 0f, null)
            val intervals = FloatArray(stroke.dashes.size) { stroke.dashes[it] * scale }
            // A collapsed frame scales every dash to zero, which is no pattern at all.
            val dashes = if (intervals.any { it > 0f }) DashPathEffect(intervals, stroke.dashOffset * scale) else null

            return NativeVectorFrame(path, outline, stroke.width * scale, dashes)
        }
    }
}
