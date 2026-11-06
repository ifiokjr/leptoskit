package dev.leptoskit.mobile

import android.animation.Animator
import android.animation.AnimatorListenerAdapter
import android.animation.TimeAnimator
import android.animation.TimeInterpolator
import android.animation.ValueAnimator
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
import android.text.InputType
import android.text.TextWatcher
import android.util.TypedValue
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.animation.LinearInterpolator
import android.view.animation.PathInterpolator
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputMethodManager
import android.widget.Button
import android.widget.CheckBox
import android.widget.CompoundButton
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.ProgressBar
import android.widget.RadioButton
import android.widget.ScrollView
import android.widget.SeekBar
import android.widget.Switch
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
    private val controlProps = mutableMapOf<Long, WidgetProps>()

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
    private val motion = NativeMotion { density }
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
        motion.clear()
        widgets.clear()
        controlProps.clear()

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
            motion.commit()
            retargeted.forEach(NativeScrollView::scrollToTarget)
            updateCanvas(batch.root)
            configureEditors()
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
                rememberControl(change.id, view, change.props)
                motion.touch(change.id, view, change.props)
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
                val child = widgets[change.child] ?: return
                val parent = child.parent as? ViewGroup
                // A view exiting, or inside one, stays whole until its exit ends.
                if (!motion.exiting(child) && (parent == null || !motion.exit(change.child, child, parent))) detach(child)
            }

            is RenderMutation.Delete -> {
                riveBitmaps.remove(change.id)
                controlProps.remove(change.id)

                val view = widgets.remove(change.id) ?: return
                if (view is NativeRiveImage) view.setImageDrawable(null)
                val exiting = motion.exiting(view)
                motion.forget(change.id, view)
                if (exiting) return
                motion.finishExits(view)
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

                "switch", "Switch" -> {
                    toggle(id, Switch(context))
                }

                "checkbox", "Checkbox" -> {
                    toggle(id, CheckBox(context))
                }

                "radio", "Radio" -> {
                    toggle(id, RadioButton(context))
                }

                "slider", "Slider" -> {
                    NativeSlider(context).apply {
                        setOnSeekBarChangeListener(
                            object : SeekBar.OnSeekBarChangeListener {
                                override fun onProgressChanged(
                                    bar: SeekBar,
                                    progress: Int,
                                    fromUser: Boolean,
                                ) {
                                    if (fromUser && !applying && isEnabled) {
                                        val value = proposedValue(progress)
                                        restoreControl(id, this@apply)
                                        event(id, 7, "{\"value\":$value}")
                                    }
                                }

                                override fun onStartTrackingTouch(bar: SeekBar) {}

                                override fun onStopTrackingTouch(bar: SeekBar) {}
                            },
                        )
                    }
                }

                "linear_progress_indicator", "LinearProgressIndicator" -> {
                    ProgressBar(context, null, android.R.attr.progressBarStyleHorizontal)
                }

                "activity_indicator", "ActivityIndicator" -> {
                    ProgressBar(context)
                }

                "text_input", "TextInput" -> {
                    textInput(id, secure = false)
                }

                "password_input", "PasswordInput" -> {
                    textInput(id, secure = true)
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
        rememberControl(id, widget, item.props)
        motion.touch(id, widget, item.props, isNew = true)
        props.apply(widget, item.props)
        retarget(widget, item.props.scrollOffset)
    }

    private fun rememberControl(
        id: Long,
        view: View,
        value: WidgetProps,
    ) {
        if (view is CompoundButton || view is NativeSlider) controlProps[id] = value
    }

    // Events are proposals. A guest may reject them without sending another prop update.
    private fun restoreControl(
        id: Long,
        view: View,
    ) {
        val value = controlProps[id] ?: return
        applying = true
        try {
            props.apply(view, value)
        } finally {
            applying = false
        }
    }

    private fun configureEditors() {
        val editors = mutableListOf<EditText>()

        fun visit(view: View) {
            if (!view.isEnabled || view.visibility != View.VISIBLE) return
            if (view is EditText) editors.add(view)
            if (view is ViewGroup) for (index in 0 until view.childCount) visit(view.getChildAt(index))
        }
        visit(root)
        editors.forEachIndexed { index, editor ->
            val next = editors.getOrNull(index + 1)
            editor.imeOptions = if (next == null) EditorInfo.IME_ACTION_DONE else EditorInfo.IME_ACTION_NEXT
            editor.nextFocusForwardId = next?.id ?: View.NO_ID
        }
    }

    private fun toggle(
        id: Long,
        control: CompoundButton,
    ): CompoundButton {
        control.setPadding(0, 0, 0, 0)
        control.minimumWidth = 0
        control.minimumHeight = 0
        val original = control.buttonDrawable
        if (control !is Switch && original != null) {
            // Android's themed glyph can be wider than Yoga's 20dp checkbox/radio frame.
            // Let the native stateful drawable scale into that frame rather than clipping it.
            control.buttonDrawable =
                object : android.graphics.drawable.InsetDrawable(original, 0) {
                    private fun extent(): Int =
                        minOf(
                            original.intrinsicWidth,
                            control.width.takeIf { it > 0 } ?: original.intrinsicWidth,
                            control.height.takeIf { it > 0 } ?: original.intrinsicHeight,
                        )

                    override fun getIntrinsicWidth(): Int = extent()

                    override fun getIntrinsicHeight(): Int = extent()
                }
        }
        control.setOnCheckedChangeListener { _, checked ->
            if (!applying && control.isEnabled && (control !is RadioButton || checked)) {
                restoreControl(id, control)
                event(id, 6, "{\"checked\":$checked}")
            }
        }
        return control
    }

    /** A secure field masks its text, and the keyboard neither suggests nor learns it. */
    private fun textInput(
        id: Long,
        secure: Boolean,
    ): EditText {
        val input = EditText(context)
        input.id = View.generateViewId()
        input.isSingleLine = true
        // After single-line mode, which would replace the password masking.
        if (secure) {
            input.inputType =
                InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
        }
        input.addTextChangedListener(InputWatcher(id, event))
        input.imeOptions = EditorInfo.IME_ACTION_DONE
        input.setOnEditorActionListener { _, action, key ->
            val enter = action == EditorInfo.IME_NULL && key?.keyCode == android.view.KeyEvent.KEYCODE_ENTER
            // Consume the down as well: EditText otherwise performs its own focus search first.
            if (enter && key?.action == android.view.KeyEvent.ACTION_DOWN) return@setOnEditorActionListener true
            val requested = if (enter) input.imeOptions and EditorInfo.IME_MASK_ACTION else action
            when (requested) {
                EditorInfo.IME_ACTION_NEXT -> {
                    root.findViewById<EditText>(input.nextFocusForwardId)?.requestFocus() == true
                }

                EditorInfo.IME_ACTION_DONE, EditorInfo.IME_ACTION_GO, EditorInfo.IME_ACTION_SEND, EditorInfo.IME_ACTION_SEARCH -> {
                    dismissEditor(root, input)
                    event(id, SUBMIT, input.text.toString())
                    true
                }

                else -> {
                    false
                }
            }
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
        motion.touch(item.id, view, null)
    }

    private fun insert(item: RenderMutation.InsertChild) {
        val parent = widget(item.parent) as ViewGroup
        val container = if (parent is ScrollView) parent.getChildAt(0) as ViewGroup else parent
        val child = widget(item.child)
        motion.cancelExit(child)
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

/** A finite slider range mapped to Android's integer track without truncating its values. */
internal class NativeSlider(
    context: Context,
) : SeekBar(context) {
    var minimum = 0.0
    var maximum = 1.0
    var step: Double? = null

    init {
        max = 10000
    }

    fun proposedValue(progress: Int): Double {
        val value = minimum + progress.toDouble() / max * (maximum - minimum)
        val increment = step ?: return value.coerceIn(minimum, maximum)
        val last = floor((maximum - minimum) / increment + 1e-10)
        val index = floor((value - minimum) / increment + 0.5).coerceIn(0.0, last)
        return (minimum + index * increment).coerceIn(minimum, maximum)
    }
}

/** Clear focus without letting Android immediately give it back to the first editor. */
internal fun dismissEditor(
    root: ViewGroup,
    editor: EditText,
) {
    root.isFocusableInTouchMode = true
    root.requestFocus()
    editor.clearFocus()
    (root.context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager)
        .hideSoftInputFromWindow(editor.windowToken, 0)
}

/** Observe the down event; let its original target still receive the entire gesture. */
internal fun dismissEditorOutside(
    root: ViewGroup,
    event: MotionEvent,
) {
    if (event.actionMasked != MotionEvent.ACTION_DOWN) return
    val editor = root.findFocus() as? EditText ?: return
    val bounds = Rect()
    if (!editor.getGlobalVisibleRect(bounds) || !bounds.contains(event.rawX.toInt(), event.rawY.toInt())) {
        dismissEditor(root, editor)
    }
}

/**
 * Runs widgets' `transform` and `animation` props on Android's animators, so
 * the guest describes a motion once and the platform plays it.
 *
 * The renderer [touch]es each widget a batch creates, updates, or lays out,
 * before applying its props, then [commit]s the batch: a widget whose
 * transition covers a changed value moves to it from where it is on screen,
 * a new widget enters, and keyframes restart when they change.
 */
internal class NativeMotion(
    private val density: () -> Float,
) {
    private val states = mutableMapOf<Long, State>()
    private val touched = LinkedHashMap<Long, View>()
    private val created = HashSet<Long>()

    /** Views drawn on after their removal while they exit, with their parents. */
    private val exits = mutableMapOf<View, Exit>()

    private class State(
        var props: WidgetProps,
    ) {
        /** The opacity and transform last applied, which a transition compares against. */
        var own: MotionVisual? = null
        var transition: Runner? = null
        var keyframes: MotionKeyframes? = null
        var keyframesAnimator: TimeAnimator? = null

        /** The background color before this batch, for color transitions. */
        var colorBefore: Int? = null
        var layout: ValueAnimator? = null
        var layoutTarget: Rect? = null
        var listening = false

        /** The properties running keyframes move, which transitions leave to them. */
        val keyframesMask
            get() = keyframes?.takeIf { keyframesAnimator?.isStarted == true }?.let { mask(it) } ?: 0
    }

    private class Exit(
        val parent: ViewGroup,
        val animator: Animator,
    )

    /** One run of a curve, moving the properties in [mask] from [from] to [to]. */
    private class Runner(
        val state: State,
        val view: View,
        val from: Look,
        val to: Look,
        var mask: Int,
        val colors: Pair<Int, Int>?,
        val curve: MotionCurve,
        delayMs: Float,
        done: () -> Unit = {},
    ) {
        val animator: ValueAnimator =
            ValueAnimator.ofFloat(0f, 1f).apply {
                duration = curve.durationMs.toLong()
                startDelay = delayMs.toLong()
                interpolator = curveInterpolator(curve)
                addUpdateListener { step(progress(it)) }
                addListener(
                    object : AnimatorListenerAdapter() {
                        override fun onAnimationEnd(animation: Animator) = done()
                    },
                )
            }

        fun step(t: Float) {
            from.lerp(to, t).applyTo(view, mask and state.keyframesMask.inv())

            if (colors != null) {
                (view.background as? GradientDrawable)?.setColor(blend(colors.first, colors.second, t.coerceIn(0f, 1f)))
            }
        }
    }

    /** A widget's opacity and transform in pixels, as the view's own properties hold them. */
    private data class Look(
        val alpha: Float,
        val translationX: Float,
        val translationY: Float,
        val scaleX: Float,
        val scaleY: Float,
        val rotation: Float,
    ) {
        fun lerp(
            to: Look,
            t: Float,
        ) = Look(
            mix(alpha, to.alpha, t).coerceIn(0f, 1f),
            mix(translationX, to.translationX, t),
            mix(translationY, to.translationY, t),
            mix(scaleX, to.scaleX, t),
            mix(scaleY, to.scaleY, t),
            mix(rotation, to.rotation, t),
        )

        fun applyTo(
            view: View,
            mask: Int,
        ) {
            if (mask and ALPHA != 0) view.alpha = alpha
            if (mask and parts(MotionTransform.TRANSLATE) != 0) {
                view.translationX = translationX
                view.translationY = translationY
            }
            if (mask and parts(MotionTransform.SCALE) != 0) {
                view.scaleX = scaleX
                view.scaleY = scaleY
            }
            if (mask and parts(MotionTransform.ROTATE) != 0) view.rotation = rotation
        }

        companion object {
            fun of(view: View) = Look(view.alpha, view.translationX, view.translationY, view.scaleX, view.scaleY, view.rotation)
        }
    }

    /** Notes a widget the batch changes, before its new [props] apply; null props for a layout change. */
    fun touch(
        id: Long,
        view: View,
        props: WidgetProps?,
        isNew: Boolean = false,
    ) {
        val state = states[id]

        if (state == null) {
            if (props == null || (props.transform == null && props.animation == null)) return
            states[id] = State(props)
        } else if (props != null) {
            state.props = props
        }

        val current = states.getValue(id)

        if (isNew) created.add(id)
        if (!touched.containsKey(id)) current.colorBefore = (view.background as? GradientDrawable)?.color?.defaultColor
        touched[id] = view
    }

    /** Applies the batch's motion, once every frame and prop in it is set. */
    fun commit() {
        for ((id, view) in touched) {
            val state = states[id] ?: continue
            update(id, view, state, id in created)
        }
        touched.clear()
        created.clear()
    }

    private fun update(
        id: Long,
        view: View,
        state: State,
        isNew: Boolean,
    ) {
        val props = state.props
        val animation = props.animation
        val own = MotionVisual((props.opacity ?: 1.0).toFloat(), props.transform)
        val (width, height) = size(view)
        val target = look(own, width, height)
        val previous = state.own
        state.own = own

        if (props.transform == null && animation == null) {
            stop(id, view)
            states.remove(id)
            target.applyTo(view, ALL)
            return
        }

        if (animation?.layout != null && !state.listening) {
            state.listening = true
            view.addOnLayoutChangeListener { _, left, top, right, bottom, oldLeft, oldTop, oldRight, oldBottom ->
                relayout(id, view, Rect(oldLeft, oldTop, oldRight, oldBottom), Rect(left, top, right, bottom))
            }
        }

        val entering = animation?.entering

        if (isNew && entering != null) {
            state.transition?.animator?.cancel()
            val from = look(entering.visual.over(own), width, height)
            from.applyTo(view, ALL)
            state.transition =
                Runner(state, view, from, target, ALL, null, entering.curve, entering.delayMs).also { it.animator.start() }
        } else {
            transition(state, view, previous, own, target, animation?.transition)
        }

        keyframes(state, view, animation?.keyframes)
    }

    private fun transition(
        state: State,
        view: View,
        previous: MotionVisual?,
        own: MotionVisual,
        target: Look,
        transition: MotionTransition?,
    ) {
        val covers = transition?.properties ?: 0
        var changed = 0

        if (previous != null && previous.opacity != own.opacity) changed = changed or ALPHA
        if (previous != null && previous.transform != own.transform) changed = changed or TRANSFORM
        val animate =
            changed and
                (
                    (if (covers and MotionTransition.OPACITY != 0) ALPHA else 0) or
                        (if (covers and MotionTransition.TRANSFORM != 0) TRANSFORM else 0)
                )
        val colorAfter = (view.background as? GradientDrawable)?.color?.defaultColor
        val colorBefore = state.colorBefore ?: Color.TRANSPARENT
        val colors =
            if (covers and MotionTransition.COLORS != 0 && colorAfter != null && colorBefore != colorAfter) {
                colorBefore to colorAfter
            } else {
                null
            }
        val running = state.transition?.takeIf { it.animator.isStarted }
        val keep = running?.mask?.and(changed.inv()) ?: 0
        val free = ALL and state.keyframesMask.inv()

        if (transition == null || (animate == 0 && colors == null)) {
            // Values that changed without a transition jump; a running
            // transition keeps moving the rest, toward their new targets.
            target.applyTo(view, free and keep.inv())
            if (running == null) return
            running.mask = keep
            if (keep != 0 && running.to != target) {
                restart(state, view, target, keep, null, running.curve, 0f)
            } else if (keep == 0) {
                running.animator.cancel()
            }
            return
        }

        val mask = animate or keep
        target.applyTo(view, free and mask.inv())
        restart(state, view, target, mask, colors, transition.curve, transition.delayMs)
    }

    private fun restart(
        state: State,
        view: View,
        target: Look,
        mask: Int,
        colors: Pair<Int, Int>?,
        curve: MotionCurve,
        delayMs: Float,
    ) {
        state.transition?.animator?.cancel()
        state.transition = Runner(state, view, Look.of(view), target, mask, colors, curve, delayMs).also { it.animator.start() }
    }

    private fun keyframes(
        state: State,
        view: View,
        keyframes: MotionKeyframes?,
    ) {
        if (keyframes == state.keyframes) return
        state.keyframesAnimator?.cancel()
        state.keyframesAnimator = null
        state.keyframes = keyframes
        if (keyframes == null) {
            state.own?.let { look(it, size(view)).applyTo(view, ALL) }
            return
        }

        val mask = mask(keyframes)
        state.keyframesAnimator =
            TimeAnimator().apply {
                setTimeListener { animator, total, _ ->
                    val own = state.own ?: MotionVisual()
                    val sample = keyframes.sample(total.toFloat(), own)

                    if (sample == null) {
                        look(own, size(view)).applyTo(view, mask)
                        animator.end()
                    } else {
                        look(sample, size(view)).applyTo(view, mask)
                    }
                }
                start()
            }
    }

    /** Moves [view] from its [old] frame to its [new] one, when its layout animates. */
    private fun relayout(
        id: Long,
        view: View,
        old: Rect,
        new: Rect,
    ) {
        val state = states[id] ?: return
        val curve = state.props.animation?.layout

        // A layout pass that put the view at its target mid-move leaves the move running.
        if (curve == null || old.isEmpty || old == new || (state.layout?.isRunning == true && state.layoutTarget == new)) return
        val running = state.layout

        running?.cancel()
        state.layoutTarget = Rect(new)
        val from = Rect(old)
        state.layout =
            ValueAnimator.ofFloat(0f, 1f).apply {
                duration = curve.durationMs.toLong()
                interpolator = curveInterpolator(curve)
                addUpdateListener {
                    val t = progress(it)
                    view.left = mix(from.left.toFloat(), new.left.toFloat(), t).toInt()
                    view.top = mix(from.top.toFloat(), new.top.toFloat(), t).toInt()
                    view.right = mix(from.right.toFloat(), new.right.toFloat(), t).toInt()
                    view.bottom = mix(from.bottom.toFloat(), new.bottom.toFloat(), t).toInt()
                }
                start()
            }
        view.left = from.left
        view.top = from.top
        view.right = from.right
        view.bottom = from.bottom
    }

    /**
     * Starts [view]'s exit when its props have one: it leaves [parent]'s children,
     * so indices stay right, but [parent] draws it until the exit ends.
     */
    fun exit(
        id: Long,
        view: View,
        parent: ViewGroup,
    ): Boolean {
        val state = states[id] ?: return false
        val exiting = state.props.animation?.exiting ?: return false
        val own = state.own ?: return false
        val (width, height) = size(view)
        state.transition?.animator?.cancel()
        parent.startViewTransition(view)
        parent.removeView(view)
        state.keyframesAnimator?.cancel()
        state.keyframes = null
        val runner =
            Runner(state, view, Look.of(view), look(exiting.visual.over(own), width, height), ALL, null, exiting.curve, exiting.delayMs) {
                if (exits[view]?.parent === parent) finishExit(view)
            }
        exits[view] = Exit(parent, runner.animator)
        runner.animator.start()

        return true
    }

    /** Whether [view] is exiting or inside a view that is. */
    fun exiting(view: View): Boolean {
        if (exits.isEmpty()) return false
        var current: View? = view

        while (current != null) {
            if (current in exits) return true
            current = current.parent as? View
        }

        return false
    }

    /** Stops [view]'s exit and puts it back as it was, when it is inserted again. */
    fun cancelExit(view: View) {
        val exit = exits.remove(view) ?: return
        exit.animator.removeAllListeners()
        exit.animator.cancel()
        exit.parent.endViewTransition(view)
        val id = view.tag as? Long ?: return
        val state = states[id] ?: return
        state.own?.let { look(it, size(view)).applyTo(view, ALL) }
        keyframes(state, view, state.props.animation?.keyframes)
    }

    /** Ends the exits drawn inside [parent], before it is reused. */
    fun finishExits(parent: View) {
        exits.entries
            .filter { it.value.parent === parent }
            .map { it.key }
            .forEach(::finishExit)
    }

    private fun finishExit(view: View) {
        val exit = exits.remove(view) ?: return
        exit.animator.removeAllListeners()
        exit.animator.cancel()
        exit.parent.endViewTransition(view)
    }

    /** Stops [id]'s motion and resets its view, when it is deleted. */
    fun forget(
        id: Long,
        view: View,
    ) {
        if (view !in exits) stop(id, view)
        states.remove(id)
        touched.remove(id)
    }

    private fun stop(
        id: Long,
        view: View,
    ) {
        val state = states[id] ?: return
        state.transition?.animator?.cancel()
        state.keyframesAnimator?.cancel()
        state.layout?.cancel()
        state.transition = null
        state.keyframesAnimator = null
        state.keyframes = null
        state.layout = null
        view.translationX = 0f
        view.translationY = 0f
        view.scaleX = 1f
        view.scaleY = 1f
        view.rotation = 0f
    }

    fun clear() {
        exits.keys.toList().forEach(::finishExit)
        for (state in states.values) {
            state.transition?.animator?.cancel()
            state.keyframesAnimator?.cancel()
            state.layout?.cancel()
        }
        states.clear()
        touched.clear()
        created.clear()
    }

    /** The view's size in pixels: its laid-out frame, or the one the renderer just assigned. */
    private fun size(view: View): Pair<Float, Float> {
        val params = view.layoutParams

        return if (params != null && params.width >= 0 && params.height >= 0) {
            params.width.toFloat() to params.height.toFloat()
        } else {
            view.width.toFloat() to view.height.toFloat()
        }
    }

    private fun look(
        visual: MotionVisual,
        size: Pair<Float, Float>,
    ) = look(visual, size.first, size.second)

    private fun look(
        visual: MotionVisual,
        width: Float,
        height: Float,
    ): Look {
        val transform = visual.transform ?: MotionTransform.IDENTITY
        val density = density()

        return Look(
            visual.opacity ?: 1f,
            transform.translateX.resolve(width, density),
            transform.translateY.resolve(height, density),
            transform.scaleX,
            transform.scaleY,
            transform.rotate,
        )
    }

    private companion object {
        const val ALPHA = 1

        /** The transform parts, shifted past [ALPHA]. */
        const val TRANSFORM = (MotionTransform.TRANSLATE or MotionTransform.SCALE or MotionTransform.ROTATE) shl 1
        const val ALL = ALPHA or TRANSFORM

        /** Transform [parts] as mask bits beside [ALPHA]. */
        fun parts(parts: Int) = parts shl 1

        fun mask(keyframes: MotionKeyframes) = (if (keyframes.animatesOpacity) ALPHA else 0) or parts(keyframes.animatedParts)

        /** The animator's eased progress, exactly one at its end: a spring's curve stops a hair short. */
        fun progress(animator: ValueAnimator) = if (animator.animatedFraction >= 1f) 1f else animator.animatedValue as Float

        fun curveInterpolator(curve: MotionCurve): TimeInterpolator =
            when (curve) {
                is MotionCurve.Timing -> {
                    val easing = curve.easing
                    if (easing ==
                        MotionEasing.LINEAR
                    ) {
                        LinearInterpolator()
                    } else {
                        PathInterpolator(easing.x1, easing.y1, easing.x2, easing.y2)
                    }
                }

                is MotionCurve.Spring -> {
                    TimeInterpolator { curve.progress(it) }
                }
            }

        fun blend(
            from: Int,
            to: Int,
            t: Float,
        ): Int =
            Color.argb(
                mix(Color.alpha(from).toFloat(), Color.alpha(to).toFloat(), t).toInt(),
                mix(Color.red(from).toFloat(), Color.red(to).toFloat(), t).toInt(),
                mix(Color.green(from).toFloat(), Color.green(to).toFloat(), t).toInt(),
                mix(Color.blue(from).toFloat(), Color.blue(to).toFloat(), t).toInt(),
            )
    }
}
