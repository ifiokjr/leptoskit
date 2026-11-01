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
import android.graphics.RectF
import android.graphics.drawable.Drawable
import android.graphics.drawable.GradientDrawable
import android.text.Editable
import android.text.TextWatcher
import android.util.TypedValue
import android.view.MotionEvent
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
import kotlin.math.roundToInt

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

    /** Scroll views that moved since they last reported their offset. */
    private val movedScrolls = LinkedHashSet<Long>()

    /** Scroll views whose target changed in the batch being applied. */
    private val retargeted = ArrayList<NativeScrollView>()
    var onRiveViewportChange: (() -> Unit)? = null
    private val props = NativeWidgetProps { density }
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

    /** The DevTools highlight and picking, in apps `leptoskit dev` reports to a hub. */
    private var devtools: NativeDevtoolsOverlay? = null

    /** Adds the DevTools overlay above the app, once the first app reports to a hub. */
    fun attachDevtools() {
        if (devtools != null || !NativeDevtools.active()) return
        val overlay = NativeDevtoolsOverlay(context, widgets::get, ::nodeOf) { density }
        root.addView(overlay, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        devtools = overlay
    }

    /** The node `view` draws, when the renderer made it. */
    private fun nodeOf(view: View): Long? = (view.tag as? Long)?.takeIf { widgets[it] === view }

    fun clear() {
        devtools?.let(root::removeView)
        devtools = null
        onRiveViewportChange = null
        widgets.values.forEach { (it as? ImageView)?.setImageDrawable(null) }
        riveBitmaps.clear()
        movedScrolls.clear()
        retargeted.clear()
        widgets.clear()

        recycledViews.clear()
        recycledTexts.clear()
    }

    /** The Rive views on screen now, for the guest thread to advance. */
    fun visibleRiveNodes(): LongArray =
        widgets
            .filter { (id, view) -> view is NativeRiveImage && riveVisible(id) }
            .keys
            .toLongArray()

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
            retargeted.forEach(NativeScrollView::scrollToTarget)
            updateCanvas(batch.root)
        } finally {
            retargeted.clear()

            applying = false
        }
    }

    /** Notes a scroll view's target when it differs from the last one it received. */
    private fun retarget(
        view: View,
        target: ScrollOffset?,
    ) {
        if (view !is NativeScrollView || view.target == target) return
        view.target = target

        if (target != null) retargeted.add(view)
    }

    /**
     * Reports where scroll views settled on the next looper turn, once per
     * view however often it moved. Android moves them while handling touches,
     * drawing, laying out, and applying a batch, where entering the guest to
     * render again is unsafe.
     */
    private fun scrolled(id: Long) {
        if (movedScrolls.add(id) && movedScrolls.size == 1) root.post(::reportScrolls)
    }

    private fun reportScrolls() {
        val moved = movedScrolls.toList()
        movedScrolls.clear()

        for (id in moved) {
            val scroll = widgets[id] as? ScrollView ?: continue
            event(id, SCROLL, "{\"x\":${scroll.scrollX / density},\"y\":${scroll.scrollY / density}}")
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
                val view = widget(change.id)
                props.apply(view, change.props)
                retarget(view, change.props.scrollOffset)
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
                    NativeScrollView(context).apply {
                        addView(FrameLayout(context))
                        setOnScrollChangeListener { _: View, _: Int, _: Int, _: Int, _: Int ->
                            onRiveViewportChange?.invoke()
                            scrolled(id)
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
        retarget(widget, item.props.scrollOffset)
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
        // Below the DevTools overlay, which stays on top.
        root.addView(child, devtools?.let(root::indexOfChild) ?: -1)
    }

    private companion object {
        /** Enough for a screen of a few thousand views; beyond it, views go. */
        const val RECYCLE_LIMIT = 4096
        const val OPAQUE = 255
        const val SUBMIT = 2
        const val FOCUS = 3
        const val BLUR = 4
        const val SCROLL = 5
        const val PRESS_IN = 8
        const val PRESS_OUT = 9
    }
}

/**
 * A vertical scroll view that scrolls to its target once its content is laid
 * out. `ScrollView` keeps an offset inside its content's current size, so a
 * target that arrives with new content waits for the layout pass that sizes it.
 */
internal class NativeScrollView(
    context: Context,
) : ScrollView(context) {
    /** The last target the renderer received. */
    var target: ScrollOffset? = null
    private var pending: ScrollOffset? = null

    fun scrollToTarget() {
        pending = target

        if (isLaidOut && !isLayoutRequested) scrollToPending()
    }

    override fun onLayout(
        changed: Boolean,
        left: Int,
        top: Int,
        right: Int,
        bottom: Int,
    ) {
        super.onLayout(changed, left, top, right, bottom)
        scrollToPending()
    }

    private fun scrollToPending() {
        val target = pending ?: return
        pending = null
        val density = resources.displayMetrics.density
        val x = (target.x * density).roundToInt()
        val y = (target.y * density).roundToInt()

        if (target.animated) smoothScrollTo(x, y) else scrollTo(x, y)
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

/**
 * Lies over everything the renderer draws: it outlines the node the DevTools
 * hub highlights, as the web agent does, and while the hub picks a node it
 * takes the next tap instead of the app. Otherwise no touch reaches it.
 *
 * The highlight changes on the runtime's reporting thread, so the overlay
 * reads it ten times a second, the rate the runtime reports layout at,
 * rather than having that thread call into the JVM.
 */
internal class NativeDevtoolsOverlay(
    context: Context,
    private val widget: (Long) -> View?,
    private val node: (View) -> Long?,
    private val density: () -> Float,
) : View(context) {
    private val ids = LongArray(2)
    private val frame = FloatArray(4)
    private val location = IntArray(2)
    private val origin = IntArray(2)
    private val outline = RectF()
    private var shown = false
    private val fill = Paint().apply { color = FILL }
    private val stroke =

        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = STROKE
            style = Paint.Style.STROKE
        }
    private val poll =
        object : Runnable {
            override fun run() {
                refresh()
                postDelayed(this, INTERVAL_MS)
            }
        }

    init {
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
        setWillNotDraw(false)
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        post(poll)
    }

    override fun onDetachedFromWindow() {
        removeCallbacks(poll)
        super.onDetachedFromWindow()
    }

    /** Moves the outline over the highlighted node, wherever its views have scrolled to, or hides it. */
    fun refresh() {
        val placed = NativeDevtools.highlight(ids, frame) && place()

        if (placed || shown) invalidate()
        shown = placed
    }

    /** Sets [outline] in this view's pixels; false when the anchor is not on screen. */
    private fun place(): Boolean {
        val anchor = widget(ids[1])?.takeIf { it.isAttachedToWindow } ?: return false
        getLocationInWindow(origin)

        if (ids[0] == ids[1]) {
            anchor.getLocationInWindow(location)
            val left = (location[0] - origin[0]).toFloat()
            val top = (location[1] - origin[1]).toFloat()
            outline.set(left, top, left + anchor.width, top + anchor.height)
        } else {
            content(anchor).getLocationInWindow(location)
            val scale = density()

            val left = location[0] - origin[0] + frame[0] * scale
            val top = location[1] - origin[1] + frame[1] * scale
            outline.set(left, top, left + frame[2] * scale, top + frame[3] * scale)
        }

        return true
    }

    override fun onDraw(canvas: Canvas) {
        if (!shown) return
        val width = 2 * density()
        stroke.strokeWidth = width
        canvas.drawRect(outline, fill)
        // A stroke is centred on its path; this one lies just outside the frame.
        val half = width / 2
        canvas.drawRect(outline.left - half, outline.top - half, outline.right + half, outline.bottom + half, stroke)
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        if (event.actionMasked == MotionEvent.ACTION_DOWN) return NativeDevtools.picking()

        if (event.actionMasked == MotionEvent.ACTION_UP) {
            pick(event.x, event.y)
            performClick()
        }

        return true
    }

    override fun performClick(): Boolean = super.performClick()

    /**
     * Picks the node under ([x], [y]) in this view's pixels: the deepest
     * drawn view there tells the runtime where, and the runtime finds the
     * node, layout-only containers included.
     */
    private fun pick(
        x: Float,
        y: Float,
    ) {
        getLocationInWindow(origin)
        val windowX = origin[0] + x
        val windowY = origin[1] + y

        var view = deepest(parent as? ViewGroup ?: return, windowX, windowY)

        while (view != null && view !== parent) {
            val id = node(view)

            if (id != null) {
                content(view).getLocationInWindow(location)
                val scale = density()
                NativeDevtools.pick(id, (windowX - location[0]) / scale, (windowY - location[1]) / scale)
                refresh()

                return
            }

            view = view.parent as? View
        }
    }

    /** The deepest visible view under a point in window pixels, drawn last first, skipping this overlay. */
    private fun deepest(
        group: ViewGroup,
        x: Float,
        y: Float,
    ): View? {
        for (index in group.childCount - 1 downTo 0) {
            val child = group.getChildAt(index)

            if (child === this || child.visibility != VISIBLE) continue
            child.getLocationInWindow(location)

            if (x < location[0] || y < location[1] || x >= location[0] + child.width || y >= location[1] + child.height) continue

            return (child as? ViewGroup)?.let { deepest(it, x, y) } ?: child
        }

        return null
    }

    /** The view a drawn widget lays its children out in: a scroll view's content. */
    private fun content(view: View): View = (view as? ScrollView)?.getChildAt(0) ?: view

    private companion object {
        const val INTERVAL_MS = 100L
        const val STROKE = 0xFFE500FF.toInt()
        const val FILL = 0x22E500FF
    }
}
