package dev.leptoskit.mobile

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Path
import android.text.Editable
import android.text.TextWatcher
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.ScrollView
import android.widget.TextView
import org.json.JSONObject

internal class NativeWidgetRenderer(
    private val root: FrameLayout,
    private val event: (Long, Int, String) -> Unit,
) {
    private val context get() = root.context
    private val density get() = root.resources.displayMetrics.density
    private val widgets = mutableMapOf<Long, View>()
    private val props = NativeWidgetProps { density }
    var applying = false
        private set

    fun clear() {
        widgets.clear()
    }

    fun render(batch: JSONObject) {
        applying = true
        try {
            val mutations = batch.getJSONArray("mutations")

            for (index in 0 until mutations.length()) applyMutation(mutations.getJSONObject(index))
            installRoot(batch)
        } finally {
            applying = false
        }
    }

    private fun applyMutation(change: JSONObject) {
        change.optJSONObject("Create")?.let { create(it) }
        change.optJSONObject("UpdateProps")?.let { props.apply(widget(it.getLong("id")), it.getJSONObject("props")) }
        change.optJSONObject("UpdateLayout")?.let { layout(it) }
        change.optJSONObject("InsertChild")?.let { insert(it) }
        change.optJSONObject("RemoveChild")?.let { detach(widgets[it.getLong("child")]) }
        change.optJSONObject("Delete")?.let { detach(widgets.remove(it.getLong("id"))) }
    }

    private fun widget(id: Long): View = checkNotNull(widgets[id])

    private fun create(item: JSONObject) {
        val id = item.getLong("id")
        val widget =
            when (val kind = item.getString("kind")) {
                "view", "View" -> {
                    FrameLayout(context)
                }

                "text", "text_run", "Text", "TextRun" -> {
                    TextView(context)
                }

                "button", "Button" -> {
                    Button(context).apply {
                        setPadding(0, 0, 0, 0)
                        setOnClickListener { event(id, 0, "") }
                    }
                }

                "pressable", "Pressable" -> {
                    FrameLayout(context).apply {
                        isClickable = true
                        setOnClickListener { event(id, 0, "") }
                    }
                }

                "text_input", "TextInput" -> {
                    textInput(id)
                }

                "scroll_view", "ScrollView" -> {
                    ScrollView(context).apply { addView(FrameLayout(context)) }
                }

                "image", "Image" -> {
                    ImageView(context)
                }

                "vector", "Vector" -> {
                    NativeVectorView(context)
                }

                else -> {
                    error("Unsupported widget: $kind")
                }
            }
        widget.tag = id
        widgets[id] = widget
        props.apply(widget, item.getJSONObject("props"))
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

    private fun layout(item: JSONObject) {
        val params =
            FrameLayout.LayoutParams(
                (item.getDouble("width") * density).toInt(),
                (item.getDouble("height") * density).toInt(),
            )
        params.leftMargin = (item.getDouble("x") * density).toInt()
        params.topMargin = (item.getDouble("y") * density).toInt()
        widget(item.getLong("id")).layoutParams = params
    }

    private fun insert(item: JSONObject) {
        val parent = widget(item.getLong("parent")) as ViewGroup
        val container = if (parent is ScrollView) parent.getChildAt(0) as ViewGroup else parent
        val child = widget(item.getLong("child"))
        detach(child)
        // Strokes may overhang a drawing's frame; only the drawing's own clip prop clips them.
        if (child is NativeVectorView) container.clipChildren = false
        container.addView(child, item.getInt("index").coerceAtMost(container.childCount))
    }

    private fun detach(child: View?) {
        (child?.parent as? ViewGroup)?.removeView(child)
    }

    private fun installRoot(batch: JSONObject) {
        if (batch.isNull("root")) return
        val child = widget(batch.getLong("root"))

        if (child.parent === root) return
        detach(child)
        root.addView(child)
    }

    private companion object {
        const val SUBMIT = 2
        const val FOCUS = 3
        const val BLUR = 4
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

/** Draws a `Vector`, remapping its outlines onto the frame whenever its drawing or size changes. */
internal class NativeVectorView(
    context: Context,
) : View(context) {
    private val fillPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.FILL }
    private val strokePaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            style = Paint.Style.STROKE
            strokeMiter = NativeVectorStroke.MITER_LIMIT
        }

    var drawing: NativeVectorDrawing? = null
        set(value) {
            field = value
            remap()
        }

    /** Each outline mapped onto the current frame in pixels, in painting order. */
    var framePaths: List<Path> = emptyList()
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
        val paths = drawing?.paths ?: return
        val density = resources.displayMetrics.density

        paths.zip(framePaths).forEach { (path, outline) ->
            path.fill?.let {
                fillPaint.color = it
                canvas.drawPath(outline, fillPaint)
            }
            path.stroke?.let {
                strokePaint.color = it.color
                strokePaint.strokeWidth = it.width * density
                strokePaint.strokeCap = it.cap
                strokePaint.strokeJoin = it.join
                canvas.drawPath(outline, strokePaint)
            }
        }
    }

    override fun getAccessibilityClassName(): CharSequence = ImageView::class.java.name

    /** Maps points before stroking, so stroke widths stay in layout units at any scale. */
    private fun remap() {
        val drawing = drawing
        framePaths =
            if (drawing == null) {
                emptyList()
            } else {
                val matrix = drawing.matrix(width.toFloat(), height.toFloat())
                drawing.paths.map { path ->
                    Path().also {
                        path.outline.transform(matrix, it)
                        it.fillType = Path.FillType.WINDING
                    }
                }
            }
        invalidate()
    }
}
