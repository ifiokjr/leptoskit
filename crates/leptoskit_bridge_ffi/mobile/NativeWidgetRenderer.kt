package dev.leptoskit.mobile

import android.content.Context
import android.graphics.drawable.Drawable
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
                    PressButton(context) { pressed -> event(id, if (pressed) PRESS_IN else PRESS_OUT, "") }.apply {
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
