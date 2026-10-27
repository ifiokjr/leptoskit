package dev.leptoskit.widgets

import android.content.Context
import android.content.res.ColorStateList
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.DashPathEffect
import android.graphics.Paint
import android.graphics.Path
import android.graphics.Rect
import android.graphics.RectF
import android.graphics.Typeface
import android.graphics.drawable.Animatable
import android.graphics.drawable.Drawable
import android.net.Uri
import android.os.Build
import android.os.Looper
import android.text.Editable
import android.text.InputType
import android.text.Layout
import android.text.StaticLayout
import android.text.TextDirectionHeuristics
import android.text.TextPaint
import android.text.TextWatcher
import android.util.TypedValue
import android.view.ContextThemeWrapper
import android.view.Gravity
import android.view.KeyEvent
import android.view.View
import android.view.ViewGroup
import android.view.accessibility.AccessibilityNodeInfo
import android.view.inputmethod.EditorInfo
import android.widget.Button
import android.widget.CheckBox
import android.widget.CompoundButton
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.HorizontalScrollView
import android.widget.ImageView
import android.widget.ProgressBar
import android.widget.RadioButton
import android.widget.ScrollView
import android.widget.SeekBar
import android.widget.Switch
import android.widget.TextView
import org.json.JSONArray
import org.json.JSONObject
import java.io.Closeable
import java.nio.ByteBuffer
import kotlin.math.abs
import kotlin.math.ceil
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt
import kotlin.math.sqrt

/** Owns Android Views for exactly one Rust NativeBackend, on the UI thread. */
class WidgetHost(
    private val context: Context,
) : Closeable {
    val density: Float = context.resources.displayMetrics.density
    private val nodes = linkedMapOf<Long, Node>()
    private var handle = 0L
    private var applying = 0
    private var dark = false
    private var foreground = true
    private var closed = false
    private var suspendedFocus: Long? = null
    private var mountedRoot: RootMount? = null

    private class RootMount(
        val id: Long,
        val container: FrameLayout,
        val view: ExactGroup,
    )

    private class Node(
        val id: Long,
        val kind: String,
        val outer: ExactGroup,
        val control: View?,
        val content: ExactGroup,
        var props: JSONObject,
        val scroll: ScrollParts? = null,
    ) {
        val nativeBackground: Drawable? = control?.background
        var imageSource: String? = null
        var riveBitmap: Bitmap? = null
        var parent: Long? = null
        val children = mutableListOf<Long>()
    }

    private class ScrollParts(
        val horizontal: HorizontalScrollView,
        val vertical: ScrollView,
    )

    fun checkUiThread() {
        check(Looper.myLooper() === Looper.getMainLooper()) { "WidgetHost must run on the UI thread" }
        check(!closed) { "WidgetHost is closed" }
    }

    fun bind(value: Long) {
        checkUiThread()
        check(handle == 0L) { "WidgetHost already has an owner" }
        handle = value
    }

    /** Exact Yoga outer allocation. Null means the node has been deleted. */
    fun view(id: Long): View? = nodes[id]?.outer

    /** The actual Android widget, available to native hosts and verification. */
    fun control(id: Long): View? = nodes[id]?.control

    /** Effective native visibility includes ancestor clipping and scrolling. */
    fun riveVisible(id: Long): Boolean {
        checkUiThread()
        val view = nodes[id]?.control ?: return false
        return foreground && view.isShown && view.getGlobalVisibleRect(Rect())
    }

    /** Copies a borrowed read-only native frame once into reusable bitmap storage. */
    fun presentRive(
        id: Long,
        width: Int,
        height: Int,
        pixels: ByteBuffer,
    ): Boolean {
        checkUiThread()
        require(width in 1..4096 && height in 1..4096 && width.toLong() * height <= 4_194_304)
        require(pixels.remaining().toLong() == width.toLong() * height * 4)
        val node = nodes[id] ?: return false
        if (node.kind != "rive") return false
        val image = node.control as? ImageView ?: return false
        var bitmap = node.riveBitmap
        if (bitmap == null || bitmap.width != width || bitmap.height != height) {
            bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
            bitmap.density = Bitmap.DENSITY_NONE
            bitmap.setPremultiplied(true)
            node.riveBitmap = bitmap
            image.setImageBitmap(bitmap)
        }
        bitmap.copyPixelsFromBuffer(pixels)
        image.invalidate()
        return true
    }

    /** Mounts only the owned root, using its physical Yoga frame in pixels. */
    fun mount(
        rootId: Long?,
        container: FrameLayout,
    ) {
        checkUiThread()
        suppress {
            val root = rootId?.let { checkNotNull(nodes[it]) { "Unknown native root $it" } }
            val previous = mountedRoot
            if (previous?.view !== root?.outer || previous?.container !== container) {
                previous?.let { (it.view.parent as? ViewGroup)?.removeView(it.view) }
                mountedRoot = root?.let { RootMount(it.id, container, it.outer) }
                if (root != null) container.addView(root.outer)
            }
            refreshMountedRoot()
        }
    }

    private fun refreshMountedRoot() {
        val mounted = mountedRoot ?: return
        val node = nodes[mounted.id]
        if (node == null) {
            mounted.container.removeView(mounted.view)
            mountedRoot = null
            return
        }
        val frame = node.outer.frame
        val width = (frame.right.roundToInt() - frame.left.roundToInt()).coerceAtLeast(0)
        val height = (frame.bottom.roundToInt() - frame.top.roundToInt()).coerceAtLeast(0)
        val left = frame.left.roundToInt()
        val top = frame.top.roundToInt()
        val current = node.outer.layoutParams as? FrameLayout.LayoutParams
        if (current == null || current.width != width || current.height != height || current.leftMargin != left ||
            current.topMargin != top
        ) {
            node.outer.layoutParams =
                FrameLayout.LayoutParams(width, height, Gravity.TOP or Gravity.LEFT).apply {
                    leftMargin = left
                    topMargin = top
                }
        }
    }

    fun apply(encoded: String) {
        checkUiThread()
        suppress {
            val mutations = JSONArray(encoded)
            repeat(mutations.length()) { index ->
                val mutation = mutations.getJSONObject(index)
                val name = mutation.keys().next()
                val fields = mutation.getJSONObject(name)
                val id = fields.optLong("id", -1)
                when (name) {
                    "Create" -> {
                        create(id, fields.getString("kind"), fields.getJSONObject("props"))
                    }

                    "UpdateProps" -> {
                        val node = checkNotNull(nodes[id])
                        node.props = fields.getJSONObject("props")
                        update(node)
                    }

                    "UpdateLayout" -> {
                        val node = checkNotNull(nodes[id])
                        node.outer.frame =
                            RectF(
                                px(fields.getDouble("x")),
                                px(fields.getDouble("y")),
                                px(fields.getDouble("x") + fields.getDouble("width")),
                                px(fields.getDouble("y") + fields.getDouble("height")),
                            )
                        node.outer.requestLayout()
                        (node.outer.parent as? View)?.requestLayout()
                        resizeScroll(node)
                    }

                    "InsertChild" -> {
                        val parent = checkNotNull(nodes[fields.getLong("parent")])
                        val child = checkNotNull(nodes[fields.getLong("child")])
                        check(child.parent == null)
                        val position = fields.getInt("index")
                        parent.children.add(position, child.id)
                        child.parent = parent.id
                        parent.content.addView(
                            child.outer,
                            position + if (parent.content === parent.outer && parent.control != null) 1 else 0,
                        )
                        if (parent.kind == "pressable") decorative(child.outer)
                        resizeScroll(parent)
                    }

                    "RemoveChild" -> {
                        val parent = checkNotNull(nodes[fields.getLong("parent")])
                        val child = checkNotNull(nodes[fields.getLong("child")])
                        parent.content.removeView(child.outer)
                        parent.children.remove(child.id)
                        child.parent = null
                        resizeScroll(parent)
                    }

                    "Delete" -> {
                        nodes.remove(id)?.let { node ->
                            node.outer.clearFocus()
                            (node.outer.parent as? ViewGroup)?.removeView(node.outer)
                            stopAnimation(node)
                            if (node.kind == "rive") {
                                (node.control as? ImageView)?.setImageDrawable(null)
                                node.riveBitmap = null
                            }
                            node.outer.removeAllViews()
                        }
                    }

                    else -> {
                        error("Unknown native mutation $name")
                    }
                }
            }
            refresh()
            refreshMountedRoot()
        }
    }

    fun setDarkMode(value: Boolean) {
        checkUiThread()
        dark = value
        suppress {
            nodes.values.forEach(::update)
            refresh()
        }
    }

    /** Activity lifecycle owner must forward onStart/onStop. */
    fun setForeground(value: Boolean) {
        checkUiThread()
        if (!value && foreground) {
            suspendedFocus = nodes.values.firstOrNull { it.control?.isFocused == true || it.outer.isFocused }?.id
        }
        foreground = value
        suppress {
            refresh()
            if (value) {
                suspendedFocus?.let { id -> nodes[id]?.let { (it.control ?: it.outer).requestFocus() } }
                suspendedFocus = null
            }
        }
    }

    override fun close() {
        if (closed) return
        checkUiThread()
        suppress {
            nodes.values.forEach { node ->
                stopAnimation(node)
                node.outer.clearFocus()
                (node.outer.parent as? ViewGroup)?.removeView(node.outer)
                node.outer.removeAllViews()
            }
            nodes.clear()
            mountedRoot = null
            suspendedFocus = null
            handle = 0L
            closed = true
        }
    }

    private inline fun suppress(operation: () -> Unit) {
        applying++
        try {
            operation()
        } finally {
            applying--
        }
    }

    private fun px(value: Double): Float = (value * density).toFloat()

    private fun create(
        id: Long,
        kind: String,
        props: JSONObject,
    ) {
        check(!nodes.containsKey(id))
        val context = ContextThemeWrapper(this.context, if (dark) android.R.style.Theme_Material else android.R.style.Theme_Material_Light)
        val outer = if (kind == "pressable") PressableGroup(context) else ExactGroup(context)
        var scroll: ScrollParts? = null
        var content = outer
        val control: View? =
            when (kind) {
                "view" -> {
                    null
                }

                "text", "text_run" -> {
                    TextView(context).apply {
                        includeFontPadding = false
                        breakStrategy = Layout.BREAK_STRATEGY_SIMPLE
                        hyphenationFrequency = Layout.HYPHENATION_FREQUENCY_NONE
                        setFallbackLineSpacing(false)
                        gravity = Gravity.TOP or Gravity.START
                    }
                }

                "button" -> {
                    PressButton(context).apply {
                        isAllCaps = false
                        setPadding(0, 0, 0, 0)
                        minWidth = 0
                        minHeight = 0
                        minimumWidth = 0
                        minimumHeight =
                            0
                    }
                }

                "text_input", "password_input" -> {
                    EditText(context).apply {
                        setSingleLine(true)
                        inputType =
                            InputType.TYPE_CLASS_TEXT or
                            if (kind == "password_input") InputType.TYPE_TEXT_VARIATION_PASSWORD else InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
                        imeOptions = EditorInfo.IME_ACTION_DONE
                        setPadding(0, 0, 0, 0)
                        includeFontPadding = false
                        importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO
                    }
                }

                "switch" -> {
                    Switch(context)
                }

                "checkbox" -> {
                    CheckBox(context)
                }

                "radio" -> {
                    RadioButton(context)
                }

                "slider" -> {
                    object : SeekBar(context) {
                        override fun onInitializeAccessibilityNodeInfo(info: AccessibilityNodeInfo) {
                            super.onInitializeAccessibilityNodeInfo(info)
                            nodes[id]?.props?.optJSONObject("slider")?.let { info.text = it.getDouble("value").toString() }
                        }

                        override fun performAccessibilityAction(
                            action: Int,
                            arguments: android.os.Bundle?,
                        ): Boolean {
                            if (action == AccessibilityNodeInfo.ACTION_SCROLL_FORWARD ||
                                action == AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD
                            ) {
                                if (!isEnabled) return false
                                emit(id, if (action == AccessibilityNodeInfo.ACTION_SCROLL_FORWARD) 8 else 9)
                                return true
                            }
                            return super.performAccessibilityAction(action, arguments)
                        }

                        override fun onKeyDown(
                            code: Int,
                            event: KeyEvent,
                        ): Boolean {
                            val increase = code == KeyEvent.KEYCODE_DPAD_RIGHT || code == KeyEvent.KEYCODE_DPAD_UP
                            val decrease = code == KeyEvent.KEYCODE_DPAD_LEFT || code == KeyEvent.KEYCODE_DPAD_DOWN
                            if (increase || decrease) {
                                if (event.repeatCount == 0) emit(id, if (increase) 8 else 9)
                                return true
                            }
                            return super.onKeyDown(code, event)
                        }
                    }.apply {
                        max = SLIDER_STEPS
                        setPadding(0, 0, 0, 0)
                    }
                }

                "linear_progress_indicator" -> {
                    ProgressBar(context, null, android.R.attr.progressBarStyleHorizontal).apply { max = SLIDER_STEPS }
                }

                "activity_indicator" -> {
                    ProgressBar(context)
                }

                "image", "rive" -> {
                    ImageView(context).apply { scaleType = ImageView.ScaleType.FIT_CENTER }
                }

                "scroll_view" -> {
                    val horizontal = GatedHorizontalScrollView(context).apply { isFillViewport = true }
                    val vertical = GatedScrollView(context).apply { isFillViewport = true }
                    content = ExactGroup(context)
                    vertical.addView(content)
                    horizontal.addView(vertical)
                    scroll = ScrollParts(horizontal, vertical)
                    horizontal.setOnScrollChangeListener { _, x, _, _, _ -> scrollEvent(id, x, vertical.scrollY) }
                    vertical.setOnScrollChangeListener { _, _, y, _, _ -> scrollEvent(id, horizontal.scrollX, y) }
                    horizontal
                }

                "pressable" -> {
                    null
                }

                "vector" -> {
                    VectorView(context, density)
                }

                else -> {
                    error("Custom Android widgets require a registered native implementation: $kind")
                }
            }
        val node = Node(id, kind, outer, control, content, props, scroll)
        nodes[id] = node
        if (control != null) {
            outer.fill = control
            outer.addView(control)
            control.setOnFocusChangeListener { _, focused -> emit(id, if (focused) 3 else 4) }
        }
        when (control) {
            is EditText -> {
                control.addTextChangedListener(
                    object : TextWatcher {
                        override fun beforeTextChanged(
                            text: CharSequence?,
                            start: Int,
                            count: Int,
                            after: Int,
                        ) = Unit

                        override fun onTextChanged(
                            text: CharSequence?,
                            start: Int,
                            before: Int,
                            count: Int,
                        ) = Unit

                        override fun afterTextChanged(text: Editable?) {
                            emit(id, 1, text = text.toString())
                        }
                    },
                )
                control.setOnEditorActionListener { _, action, event ->
                    if (action == EditorInfo.IME_ACTION_DONE ||
                        (event?.keyCode == KeyEvent.KEYCODE_ENTER && event.action == KeyEvent.ACTION_UP)
                    ) {
                        emit(id, 2, text = control.text.toString())
                        true
                    } else {
                        false
                    }
                }
            }

            is CompoundButton -> {
                control.setOnCheckedChangeListener { _, checked ->
                    if (kind == "radio") {
                        if (checked) emit(id, 5, 1.0)
                    } else {
                        emit(id, 5, if (checked) 1.0 else 0.0)
                    }
                }
                if (kind == "radio") {
                    control.setOnKeyListener { _, code, event ->
                        if (code in RADIO_KEYS && event.action == KeyEvent.ACTION_DOWN) {
                            if (event.repeatCount ==
                                0
                            ) {
                                moveRadio(id, code == KeyEvent.KEYCODE_DPAD_RIGHT || code == KeyEvent.KEYCODE_DPAD_DOWN)
                            }
                            true
                        } else {
                            false
                        }
                    }
                }
            }

            is SeekBar -> {
                control.setOnSeekBarChangeListener(
                    object : SeekBar.OnSeekBarChangeListener {
                        override fun onProgressChanged(
                            bar: SeekBar,
                            position: Int,
                            fromUser: Boolean,
                        ) {
                            if (fromUser) emit(id, 6, position.toDouble() / SLIDER_STEPS)
                        }

                        override fun onStartTrackingTouch(bar: SeekBar) = Unit

                        override fun onStopTrackingTouch(bar: SeekBar) = Unit
                    },
                )
            }

            is PressButton -> {
                control.setOnClickListener { emit(id, 0) }
                control.onPressedChanged = { pressed -> emit(id, if (pressed) 10 else 11) }
            }
        }
        if (outer is PressableGroup) {
            outer.setOnClickListener { emit(id, 0) }
            outer.onPressedChanged = { pressed -> emit(id, if (pressed) 10 else 11) }
            outer.setOnFocusChangeListener { _, focused -> emit(id, if (focused) 3 else 4) }
        }
        update(node)
    }

    /** Maps the shared ARIA properties onto Android's accessibility APIs. */
    private fun applyAccessibility(
        target: View,
        props: JSONObject,
    ) {
        if (props.optBoolean("aria_hidden")) {
            target.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
        } else if (target.importantForAccessibility == View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS) {
            target.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_AUTO
        }

        when (props.string("aria_live")) {
            "polite" -> target.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE)
            "assertive" -> target.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_ASSERTIVE)
            else -> target.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_NONE)
        }

        val role = props.string("role")
        val heading = role == "heading"
        val described = props.string("aria_description")
        val valueText = props.string("aria_value_text")
        val checked = props.string("aria_checked")
        val selected = props.optBoolean("aria_selected")
        val expanded = if (props.isNull("aria_expanded")) null else props.optBoolean("aria_expanded")
        val position = props.number("aria_pos_in_set", -1.0).toInt()
        val setSize = props.number("aria_set_size", -1.0).toInt()
        if (role == null && described == null && valueText == null && checked == null && !selected &&
            expanded == null && position <= 0 && setSize <= 0
        ) {
            return
        }

        target.accessibilityDelegate =
            object : View.AccessibilityDelegate() {
                override fun onInitializeAccessibilityNodeInfo(
                    host: View,
                    info: AccessibilityNodeInfo,
                ) {
                    super.onInitializeAccessibilityNodeInfo(host, info)
                    // TalkBack announces the class name as the control's
                    // kind; the shared resolver already folded labels and
                    // relationships into contentDescription.
                    when (role) {
                        "button", "option", "tab", "treeitem" -> {
                            info.className = "android.widget.Button"
                        }

                        "checkbox", "menuitemcheckbox" -> {
                            info.className = "android.widget.CheckBox"
                        }

                        "radio", "menuitemradio" -> {
                            info.className = "android.widget.RadioButton"
                        }

                        "switch" -> {
                            info.className = "android.widget.Switch"
                        }

                        "img" -> {
                            info.className = "android.widget.ImageView"
                        }

                        "slider" -> {
                            info.className = "android.widget.SeekBar"
                        }

                        "progressbar", "meter" -> {
                            info.className = "android.widget.ProgressBar"
                        }

                        "textbox", "searchbox" -> {
                            info.className = "android.widget.EditText"
                        }

                        "link" -> {
                            info.className = "android.widget.TextView"
                        }

                        "menuitem" -> {
                            info.className = "android.widget.MenuItem"
                        }
                    }
                    if (heading && Build.VERSION.SDK_INT >= 28) info.isHeading = true
                    described?.let { info.hintText = it }
                    if (valueText != null && Build.VERSION.SDK_INT >= 30) {
                        info.stateDescription = valueText
                    }
                    when (checked) {
                        "true" -> {
                            info.isCheckable = true
                            info.isChecked = true
                        }

                        "false" -> {
                            info.isCheckable = true
                            info.isChecked = false
                        }

                        // TalkBack has no mixed check state; expose it as
                        // checkable and let the value text carry the detail.
                        "mixed" -> {
                            info.isCheckable = true
                        }
                    }
                    if (selected) info.isSelected = true
                    expanded?.let {
                        info.addAction(
                            if (it) AccessibilityNodeInfo.ACTION_EXPAND else AccessibilityNodeInfo.ACTION_COLLAPSE,
                        )
                    }
                    if (position > 0 && setSize > 0) {
                        info.collectionItemInfo =
                            AccessibilityNodeInfo.CollectionItemInfo.obtain(
                                0,
                                1,
                                position - 1,
                                1,
                                heading,
                            )
                    }
                }
            }
    }

    private fun update(node: Node) {
        val props = node.props
        val outer = node.outer
        val rtl = props.string("direction") == "Rtl"
        outer.layoutDirection = if (rtl) View.LAYOUT_DIRECTION_RTL else View.LAYOUT_DIRECTION_LTR
        outer.visibility = if (props.optBoolean("hidden")) View.GONE else View.VISIBLE
        outer.alpha = props.number("opacity", 1.0).toFloat()
        outer.radius = px(props.number("corner_radius"))
        outer.clipping = props.optBoolean("clip") || outer.radius > 0f
        outer.background =
            SurfaceDrawable(
                props.color("background", android.graphics.Color.TRANSPARENT),
                props.color("border_color", android.graphics.Color.TRANSPARENT),
                px(props.number("border_width")),
                outer.radius,
            )
        val control = node.control
        control?.layoutDirection = outer.layoutDirection
        val label = props.string("accessibility_label")
        val target = control ?: outer
        target.contentDescription = label
        applyAccessibility(target, props)
        // A labelled drawing is an image; an unlabelled one is decoration.
        if (control is VectorView && !props.optBoolean("aria_hidden")) {
            control.importantForAccessibility =
                if (label != null) View.IMPORTANT_FOR_ACCESSIBILITY_YES else View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }
        val foregroundColor = props.color("color", if (dark) 0xffeeeeee.toInt() else 0xff202124.toInt())
        if (control is Button || control is EditText) {
            val sharedPaint =
                !props.isNull("background") || !props.isNull("border_width") || !props.isNull("border_color") || outer.radius > 0f
            control.background = if (sharedPaint) null else node.nativeBackground
            if (!sharedPaint) {
                control.backgroundTintList =
                    ColorStateList.valueOf(
                        if (control is EditText) {
                            foregroundColor
                        } else if (dark) {
                            0xff424242.toInt()
                        } else {
                            0xffe0e0e0.toInt()
                        },
                    )
            }
            control.setPadding(0, 0, 0, 0)
        }
        if (control is TextView) {
            control.textDirection = if (rtl) View.TEXT_DIRECTION_RTL else View.TEXT_DIRECTION_LTR
            val weight =
                when (props.string("font_weight")) {
                    "medium" -> 500
                    "semibold" -> 600
                    "bold" -> 700
                    else -> 400
                }
            control.typeface = Typeface.create(Typeface.DEFAULT, weight, false)
            control.setTextSize(TypedValue.COMPLEX_UNIT_PX, px(props.number("font_size", 14.0)))
            control.setTextColor(foregroundColor)
            control.gravity =
                (if (control is EditText || control is Button) Gravity.CENTER_VERTICAL else Gravity.TOP) or
                when (props.string("text_align")) {
                    "center" -> Gravity.CENTER_HORIZONTAL
                    "end" -> Gravity.END
                    "start" -> Gravity.START
                    else -> if (node.kind == "button") Gravity.CENTER_HORIZONTAL else Gravity.START
                }
            if (control is EditText) {
                val value = props.string("value") ?: props.string("text") ?: ""
                if (control.text.toString() != value) {
                    val start = control.selectionStart.coerceAtLeast(0).coerceAtMost(value.length)
                    val end = control.selectionEnd.coerceAtLeast(0).coerceAtMost(value.length)
                    control.setText(value)
                    control.setSelection(start, end)
                }
                control.hint = props.string("placeholder")
                control.setHintTextColor(if (dark) 0xffa0a0a0.toInt() else 0xff707070.toInt())
            } else if (control !is CompoundButton) {
                control.text = props.string("text") ?: ""
            }
        }
        when (control) {
            is CompoundButton -> {
                control.isChecked = props.optBoolean("checked")
                val tint =
                    ColorStateList(
                        arrayOf(intArrayOf(android.R.attr.state_checked), intArrayOf()),
                        intArrayOf(props.color("color", 0xff2563eb.toInt()), if (dark) 0xffbbbbbb.toInt() else 0xff616161.toInt()),
                    )
                if (control is Switch) {
                    control.thumbTintList = tint
                    control.trackTintList = tint
                } else {
                    control.buttonTintList = tint
                }
            }

            is SeekBar -> {
                val slider = props.optJSONObject("slider")
                if (slider !=
                    null
                ) {
                    control.progress =
                        (
                            ((slider.getDouble("value") - slider.getDouble("min")) / (slider.getDouble("max") - slider.getDouble("min"))) *
                                SLIDER_STEPS
                        ).roundToInt().coerceIn(0, SLIDER_STEPS)
                    if (android.os.Build.VERSION.SDK_INT >= 30) control.stateDescription = slider.getDouble("value").toString()
                }
                control.progressTintList = ColorStateList.valueOf(props.color("color", 0xff2563eb.toInt()))
                control.thumbTintList = control.progressTintList
                control.progressBackgroundTintList = ColorStateList.valueOf(if (dark) 0xffbbbbbb.toInt() else 0xff616161.toInt())
            }

            is ProgressBar -> {
                val progress = props.optJSONObject("progress")
                val fraction = progress?.optDouble("determinate", Double.NaN)
                control.isIndeterminate = node.kind == "activity_indicator" || fraction == null || fraction.isNaN()
                if (fraction != null && !fraction.isNaN()) control.progress = (fraction * SLIDER_STEPS).roundToInt()
                control.progressTintList = ColorStateList.valueOf(props.color("color", 0xff2563eb.toInt()))
                control.indeterminateTintList = control.progressTintList
            }

            is ImageView -> {
                val source = props.string("source")
                if (node.imageSource != source) {
                    node.imageSource = source
                    control.setImageDrawable(null)
                    if (source != null) {
                        val uri = Uri.parse(source)
                        require(uri.scheme == null || uri.scheme == "file" || uri.scheme == "android.resource") {
                            "Images require a local file or android.resource URI"
                        }
                        control.setImageURI(if (uri.scheme == null) Uri.fromFile(java.io.File(source)) else uri)
                    }
                }
            }

            is VectorView -> {
                control.drawing = props.optJSONObject("drawing")
            }
        }
        outer.invalidate()
    }

    private fun enabled(node: Node): Boolean {
        var current: Node? = node
        while (current != null) {
            if (current.props.optBoolean("hidden") || !current.props.optBoolean("enabled", true)) return false
            current = current.parent?.let(nodes::get)
        }
        return true
    }

    private fun visible(node: Node): Boolean {
        var current: Node? = node
        while (current != null) {
            if (current.props.optBoolean("hidden")) return false
            current = current.parent?.let(nodes::get)
        }
        return true
    }

    private fun refresh() {
        nodes.values.forEach { node ->
            val interactive = enabled(node) && foreground
            val previouslyEnabled = node.control?.isEnabled == true
            node.outer.isEnabled = interactive
            node.control?.isEnabled = interactive
            node.scroll?.vertical?.isEnabled = interactive
            if (!interactive && previouslyEnabled) node.scroll?.let(::stopFling)
            if (!interactive && node.outer.hasFocus()) node.outer.clearFocus()
            if (node.control is ProgressBar) {
                val requested = node.kind != "activity_indicator" || node.props.optBoolean("running", true)
                val running = requested && visible(node) && foreground
                node.control.visibility = if (running) View.VISIBLE else View.INVISIBLE
                node.control.indeterminateDrawable?.setVisible(running, false)
                val animation = node.control.indeterminateDrawable as? Animatable
                if (node.control.isIndeterminate && running) animation?.start() else animation?.stop()
            }
        }
    }

    /**
     * Stops a running fling. A zero-velocity fling replaces it at the current
     * offset; finishing that fling immediately keeps a view that is not drawing,
     * such as one in a backgrounded Activity, from later snapping back to this
     * offset over newer scroll positions on its next frame.
     */
    private fun stopFling(scroll: ScrollParts) {
        scroll.horizontal.fling(0)
        scroll.horizontal.computeScroll()
        scroll.vertical.fling(0)
        scroll.vertical.computeScroll()
    }

    private fun stopAnimation(node: Node) {
        (node.control as? ProgressBar)?.let { (it.indeterminateDrawable as? Animatable)?.stop() }
    }

    private fun resizeScroll(node: Node) {
        val owner = if (node.scroll != null) node else node.parent?.let(nodes::get)
        val scroll = owner?.scroll ?: return
        var width =
            owner.outer.frame
                .width()
                .roundToInt()
                .coerceAtLeast(0)
        var height =
            owner.outer.frame
                .height()
                .roundToInt()
                .coerceAtLeast(0)
        owner.children.forEach { id ->
            nodes[id]?.outer?.frame?.let {
                width = max(width, ceil(it.right.toDouble()).toInt())
                height =
                    max(height, ceil(it.bottom.toDouble()).toInt())
            }
        }
        owner.content.layoutParams = FrameLayout.LayoutParams(width, height)
        scroll.vertical.layoutParams =
            FrameLayout.LayoutParams(
                width,
                owner.outer.frame
                    .height()
                    .roundToInt()
                    .coerceAtLeast(0),
            )
        scroll.horizontal.requestLayout()
    }

    private fun emit(
        id: Long,
        kind: Int,
        value: Double = 0.0,
        text: String = "",
    ) {
        if (applying > 0 || closed) return
        val node = nodes[id] ?: return
        if (!foreground || !enabled(node)) {
            suppress {
                update(node)
                refresh()
            }
            return
        }
        nativeEvent(handle, id, kind, value, text)
        // The callback may replace or delete the firing node. Always restore
        // the newest controlled description, never the captured old props.
        nodes[id]?.let { current ->
            suppress {
                update(current)
                refresh()
            }
        }
    }

    private fun scrollEvent(
        id: Long,
        x: Int,
        y: Int,
    ) {
        emit(id, 7, text = "[${x / density},${y / density}]")
    }

    private fun moveRadio(
        id: Long,
        forwards: Boolean,
    ) {
        val node = nodes[id] ?: return
        if (!enabled(node)) return
        val group = node.props.string("radio_group") ?: return
        var root = node
        while (root.parent != null) root = nodes[root.parent] ?: break
        val members = mutableListOf<Node>()

        fun visit(current: Node) {
            if (current.kind == "radio" && current.props.string("radio_group") == group && enabled(current)) members.add(current)
            current.children.forEach { child -> nodes[child]?.let(::visit) }
        }
        visit(root)
        val position = members.indexOf(node)
        if (position < 0 || members.isEmpty()) return
        val next = members[(position + if (forwards) 1 else members.size - 1) % members.size]
        next.control?.requestFocus()
        emit(next.id, 5, 1.0)
    }

    private fun decorative(view: View) {
        view.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
        view.isFocusable = false
        view.isClickable = false
        if (view is ViewGroup) repeat(view.childCount) { decorative(view.getChildAt(it)) }
    }

    private class GatedHorizontalScrollView(
        context: Context,
    ) : HorizontalScrollView(context) {
        override fun dispatchTouchEvent(event: android.view.MotionEvent): Boolean = isEnabled && super.dispatchTouchEvent(event)

        override fun dispatchKeyEvent(event: KeyEvent): Boolean = isEnabled && super.dispatchKeyEvent(event)

        override fun dispatchGenericMotionEvent(event: android.view.MotionEvent): Boolean =
            isEnabled && super.dispatchGenericMotionEvent(event)

        override fun performAccessibilityAction(
            action: Int,
            arguments: android.os.Bundle?,
        ): Boolean = isEnabled && super.performAccessibilityAction(action, arguments)
    }

    private class GatedScrollView(
        context: Context,
    ) : ScrollView(context) {
        override fun dispatchTouchEvent(event: android.view.MotionEvent): Boolean = isEnabled && super.dispatchTouchEvent(event)

        override fun dispatchKeyEvent(event: KeyEvent): Boolean = isEnabled && super.dispatchKeyEvent(event)

        override fun dispatchGenericMotionEvent(event: android.view.MotionEvent): Boolean =
            isEnabled && super.dispatchGenericMotionEvent(event)

        override fun performAccessibilityAction(
            action: Int,
            arguments: android.os.Bundle?,
        ): Boolean = isEnabled && super.performAccessibilityAction(action, arguments)
    }

    private class SurfaceDrawable(
        private val background: Int,
        private val border: Int,
        private val stroke: Float,
        private val radius: Float,
    ) : Drawable() {
        private val paint = Paint(Paint.ANTI_ALIAS_FLAG)

        override fun draw(canvas: Canvas) {
            val rect = RectF(bounds)
            paint.style = Paint.Style.FILL
            paint.color = background
            canvas.drawRoundRect(rect, radius, radius, paint)
            if (stroke > 0f) {
                rect.inset(stroke / 2, stroke / 2)
                paint.style = Paint.Style.STROKE
                paint.strokeWidth = stroke
                paint.color = border
                canvas.drawRoundRect(rect, (radius - stroke / 2).coerceAtLeast(0f), (radius - stroke / 2).coerceAtLeast(0f), paint)
            }
        }

        override fun setAlpha(alpha: Int) {
            paint.alpha = alpha
        }

        override fun setColorFilter(filter: android.graphics.ColorFilter?) {
            paint.colorFilter = filter
        }

        @Deprecated("Deprecated in Android")
        override fun getOpacity(): Int = android.graphics.PixelFormat.TRANSLUCENT
    }

    /**
     * Draws a shared vector drawing. The view box maps onto this view's bounds,
     * and the points are mapped before each path is built, so a fixed stroke
     * stays in layout units however the drawing scales, while a stroke scaling
     * with the drawing grows by the square root of its area scale. Paths and
     * dash effects are rebuilt only when the drawing or the size changes, never
     * while drawing.
     */
    private class VectorView(
        context: Context,
        private val density: Float,
    ) : View(context) {
        private class Shape(
            val path: Path,
            val fill: Int?,
            val stroke: Int?,
            val strokeWidth: Float,
            val cap: Paint.Cap,
            val join: Paint.Join,
            val miter: Float,
            /** The dash pattern in pixels, or null for a solid stroke. */
            val dashes: DashPathEffect?,
        )

        private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
        private var encoded: String? = null
        private var shapes = emptyArray<Shape>()

        var drawing: JSONObject? = null
            set(value) {
                val next = value?.toString()
                if (next == encoded) return
                encoded = next
                field = value
                rebuild()
                invalidate()
            }

        override fun getAccessibilityClassName(): CharSequence = ImageView::class.java.name

        override fun onSizeChanged(
            width: Int,
            height: Int,
            previousWidth: Int,
            previousHeight: Int,
        ) {
            super.onSizeChanged(width, height, previousWidth, previousHeight)
            rebuild()
        }

        override fun onDraw(canvas: Canvas) {
            for (shape in shapes) {
                shape.fill?.let {
                    paint.style = Paint.Style.FILL
                    paint.color = it
                    paint.pathEffect = null
                    canvas.drawPath(shape.path, paint)
                }
                shape.stroke?.let {
                    paint.style = Paint.Style.STROKE
                    paint.color = it
                    paint.strokeWidth = shape.strokeWidth
                    paint.strokeCap = shape.cap
                    paint.strokeJoin = shape.join
                    paint.strokeMiter = shape.miter
                    paint.pathEffect = shape.dashes
                    canvas.drawPath(shape.path, paint)
                }
            }
        }

        private fun rebuild() {
            val drawing = drawing
            if (drawing == null) {
                shapes = emptyArray()
                return
            }
            val box = drawing.getJSONObject("view_box")
            val minX = box.getDouble("min_x").toFloat()
            val minY = box.getDouble("min_y").toFloat()
            val boxWidth = box.getDouble("width").toFloat()
            val boxHeight = box.getDouble("height").toFloat()
            val frameWidth = width.toFloat()
            val frameHeight = height.toFloat()
            val scaleX: Float
            val scaleY: Float
            val offsetX: Float
            val offsetY: Float
            val scaling = drawing.optString("scaling", "fit")
            if (scaling == "stretch") {
                scaleX = frameWidth / boxWidth
                scaleY = frameHeight / boxHeight
                offsetX = -minX * scaleX
                offsetY = -minY * scaleY
            } else {
                val scale =
                    if (scaling == "fill") {
                        max(frameWidth / boxWidth, frameHeight / boxHeight)
                    } else {
                        min(frameWidth / boxWidth, frameHeight / boxHeight)
                    }
                // Omitted when centred on both axes.
                val align = drawing.optJSONObject("align")
                scaleX = scale
                scaleY = scale
                offsetX = (frameWidth - boxWidth * scale) * alignFactor(align?.optString("x")) - minX * scale
                offsetY = (frameHeight - boxHeight * scale) * alignFactor(align?.optString("y")) - minY * scale
            }
            // Pixels per stroke unit: layout units for a fixed stroke, view box
            // units for one scaling with the drawing.
            val drawingStrokeScale = sqrt(abs(scaleX * scaleY))
            val paths = drawing.optJSONArray("paths") ?: JSONArray()
            shapes =
                Array(paths.length()) { index ->
                    val entry = paths.getJSONObject(index)
                    val path =
                        Path().apply {
                            fillType =
                                if (entry.optString("fill_rule") == "evenodd") {
                                    Path.FillType.EVEN_ODD
                                } else {
                                    Path.FillType.WINDING
                                }
                        }
                    // Canonical path data: absolute M, L, Q, C, and Z, every
                    // token separated by exactly one space.
                    val data = entry.getString("data")
                    if (data.isNotEmpty()) {
                        val tokens = data.split(' ')
                        var position = 0

                        fun x(): Float = tokens[position++].toFloat() * scaleX + offsetX

                        fun y(): Float = tokens[position++].toFloat() * scaleY + offsetY
                        while (position < tokens.size) {
                            when (val command = tokens[position++]) {
                                "M" -> path.moveTo(x(), y())
                                "L" -> path.lineTo(x(), y())
                                "Q" -> path.quadTo(x(), y(), x(), y())
                                "C" -> path.cubicTo(x(), y(), x(), y(), x(), y())
                                "Z" -> path.close()
                                else -> error("Unknown vector path command $command")
                            }
                        }
                    }
                    val stroke = entry.optJSONObject("stroke")
                    val strokeScale = if (stroke?.optString("scaling") == "drawing") drawingStrokeScale else density
                    val strokeWidth = (stroke?.getDouble("width") ?: 0.0).toFloat() * strokeScale
                    // Present only for a dashed stroke, always with an even
                    // number of lengths.
                    val dashes = stroke?.optJSONArray("dashes")
                    val dashOffset = (stroke?.optDouble("dash_offset", 0.0) ?: 0.0).toFloat()
                    Shape(
                        path,
                        if (entry.isNull("fill")) null else rgbaToArgb(entry.getLong("fill")),
                        // Android draws a zero width as a hairline; a stroke
                        // scaled to nothing draws nothing.
                        if (strokeWidth > 0f) stroke?.let { rgbaToArgb(it.getLong("color")) } else null,
                        strokeWidth,
                        when (stroke?.optString("cap")) {
                            "round" -> Paint.Cap.ROUND
                            "square" -> Paint.Cap.SQUARE
                            else -> Paint.Cap.BUTT
                        },
                        when (stroke?.optString("join")) {
                            "round" -> Paint.Join.ROUND
                            "bevel" -> Paint.Join.BEVEL
                            else -> Paint.Join.MITER
                        },
                        (stroke?.optDouble("miter_limit", MITER_LIMIT) ?: MITER_LIMIT).toFloat(),
                        if (dashes == null || dashes.length() == 0 || strokeWidth <= 0f) {
                            null
                        } else {
                            DashPathEffect(
                                FloatArray(dashes.length()) { dashes.getDouble(it).toFloat() * strokeScale },
                                dashOffset * strokeScale,
                            )
                        },
                    )
                }
        }

        private companion object {
            /** The shared `Stroke::MITER_LIMIT`, omitted from the wire. */
            const val MITER_LIMIT = 4.0

            /** The share of the spare space placed before a fitted or filled drawing. */
            fun alignFactor(align: String?): Float =
                when (align) {
                    "start" -> 0f
                    "end" -> 1f
                    else -> 0.5f
                }
        }
    }

    /** Ignores toolkit intrinsic minima; Yoga owns every outer allocation. */
    private open class ExactGroup(
        context: Context,
    ) : ViewGroup(context) {
        var frame = RectF()
        var fill: View? = null
        var radius = 0f
        var clipping = false

        init {
            clipChildren = false
            clipToPadding = false
        }

        // Like FrameLayout, a container that never scrolls shows a child's
        // pressed state on touch down; scroll views still delay it.
        override fun shouldDelayChildPressedState(): Boolean = false

        override fun onMeasure(
            widthMeasureSpec: Int,
            heightMeasureSpec: Int,
        ) {
            val width = measuredExtent(widthMeasureSpec, layoutParams?.width ?: 0)
            val height = measuredExtent(heightMeasureSpec, layoutParams?.height ?: 0)
            setMeasuredDimension(width, height)
            repeat(childCount) { index ->
                val child = getChildAt(index)
                val rect = (child as? ExactGroup)?.frame
                child.measure(
                    MeasureSpec.makeMeasureSpec(
                        (rect?.let { it.right.roundToInt() - it.left.roundToInt() } ?: width).coerceAtLeast(0),
                        MeasureSpec.EXACTLY,
                    ),
                    MeasureSpec.makeMeasureSpec(
                        (
                            rect?.let { it.bottom.roundToInt() - it.top.roundToInt() }
                                ?: height
                        ).coerceAtLeast(0),
                        MeasureSpec.EXACTLY,
                    ),
                )
            }
        }

        override fun onLayout(
            changed: Boolean,
            left: Int,
            top: Int,
            right: Int,
            bottom: Int,
        ) {
            repeat(childCount) { index ->
                val child = getChildAt(index)
                val rect = (child as? ExactGroup)?.frame
                if (rect !=
                    null
                ) {
                    child.layout(rect.left.roundToInt(), rect.top.roundToInt(), rect.right.roundToInt(), rect.bottom.roundToInt())
                } else {
                    child.layout(0, 0, width, height)
                }
            }
        }

        private fun measuredExtent(
            spec: Int,
            desired: Int,
        ): Int =
            when (MeasureSpec.getMode(spec)) {
                MeasureSpec.EXACTLY -> MeasureSpec.getSize(spec)
                MeasureSpec.AT_MOST -> desired.coerceAtLeast(0).coerceAtMost(MeasureSpec.getSize(spec))
                else -> desired.coerceAtLeast(0)
            }

        override fun draw(canvas: Canvas) {
            if (!clipping) {
                super.draw(canvas)
                return
            }
            val saved = canvas.save()
            val path = Path().apply { addRoundRect(0f, 0f, width.toFloat(), height.toFloat(), radius, radius, Path.Direction.CW) }
            canvas.clipPath(path)
            super.draw(canvas)
            canvas.restoreToCount(saved)
        }
    }

    /**
     * A platform button that reports the pressed state its ripple follows:
     * held by a finger that stays near it, or by a key.
     */
    private class PressButton(
        context: Context,
    ) : Button(context) {
        var onPressedChanged: ((Boolean) -> Unit)? = null

        override fun setPressed(pressed: Boolean) {
            val changed = pressed != isPressed
            super.setPressed(pressed)
            if (changed) onPressedChanged?.invoke(pressed)
        }
    }

    private class PressableGroup(
        context: Context,
    ) : ExactGroup(context) {
        var onPressedChanged: ((Boolean) -> Unit)? = null

        init {
            isClickable = true
            isFocusable = true
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_YES
        }

        override fun setPressed(pressed: Boolean) {
            val changed = pressed != isPressed
            super.setPressed(pressed)
            if (changed) onPressedChanged?.invoke(pressed)
        }

        override fun onInterceptTouchEvent(event: android.view.MotionEvent): Boolean = true

        override fun getAccessibilityClassName(): CharSequence = Button::class.java.name

        override fun onInitializeAccessibilityNodeInfo(info: AccessibilityNodeInfo) {
            super.onInitializeAccessibilityNodeInfo(info)
            info.className = Button::class.java.name
            info.isClickable = isEnabled
            if (isEnabled) info.addAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_CLICK)
        }

        override fun onKeyDown(
            code: Int,
            event: KeyEvent,
        ): Boolean {
            if (code == KeyEvent.KEYCODE_SPACE || code == KeyEvent.KEYCODE_ENTER || code == KeyEvent.KEYCODE_DPAD_CENTER) {
                if (isEnabled && event.repeatCount == 0) isPressed = true
                return true
            }
            return super.onKeyDown(code, event)
        }

        override fun onKeyUp(
            code: Int,
            event: KeyEvent,
        ): Boolean {
            if (code == KeyEvent.KEYCODE_SPACE || code == KeyEvent.KEYCODE_ENTER || code == KeyEvent.KEYCODE_DPAD_CENTER) {
                val activate = isPressed && isEnabled && !event.isCanceled
                isPressed = false
                if (activate) performClick()
                return true
            }
            return super.onKeyUp(code, event)
        }
    }

    companion object {
        const val SLIDER_STEPS = 1_000_000
        private val RADIO_KEYS =
            setOf(KeyEvent.KEYCODE_DPAD_LEFT, KeyEvent.KEYCODE_DPAD_RIGHT, KeyEvent.KEYCODE_DPAD_UP, KeyEvent.KEYCODE_DPAD_DOWN)

        @JvmStatic private external fun nativeEvent(
            handle: Long,
            id: Long,
            kind: Int,
            value: Double,
            text: String,
        )

        /** Pure per-call platform measurement; safe from an attached Rust worker. */
        @JvmStatic fun measureText(
            text: String,
            fontSize: Float,
            weight: Int,
            width: Float,
            density: Float,
            rtl: Boolean = false,
        ): FloatArray {
            val paint =
                TextPaint(Paint.ANTI_ALIAS_FLAG).apply {
                    textSize = fontSize * density
                    typeface = Typeface.create(Typeface.DEFAULT, weight, false)
                }
            val desired = Layout.getDesiredWidth(text, paint)
            val limit =
                when {
                    width == -1f -> desired
                    width == -2f -> text.split(Regex("\\s+")).maxOfOrNull { Layout.getDesiredWidth(it, paint) } ?: 0f
                    else -> width * density
                }
            val layout =
                StaticLayout.Builder
                    .obtain(
                        text,
                        0,
                        text.length,
                        paint,
                        ceil(limit.toDouble()).toInt().coerceAtLeast(1),
                    ).setIncludePad(false)
                    .setTextDirection(if (rtl) TextDirectionHeuristics.RTL else TextDirectionHeuristics.LTR)
                    .setBreakStrategy(Layout.BREAK_STRATEGY_SIMPLE)
                    .setHyphenationFrequency(Layout.HYPHENATION_FREQUENCY_NONE)
                    .setUseLineSpacingFromFallbacks(false)
                    .build()
            var actual = 0f
            repeat(layout.lineCount) { actual = max(actual, layout.getLineWidth(it)) }
            val pixelWidth = ceil(actual.coerceAtMost(limit.coerceAtLeast(0f)).toDouble()).toFloat()
            return floatArrayOf(pixelWidth / density, layout.height / density)
        }
    }
}

private fun JSONObject.string(name: String): String? = if (isNull(name)) null else getString(name)

private fun JSONObject.number(
    name: String,
    fallback: Double = 0.0,
): Double = if (isNull(name)) fallback else getDouble(name)

private fun JSONObject.color(
    name: String,
    fallback: Int,
): Int {
    if (isNull(name)) return fallback
    return rgbaToArgb(getLong(name))
}

/** Converts a shared packed 0xRRGGBBAA color to Android's 0xAARRGGBB. */
private fun rgbaToArgb(rgba: Long): Int = ((rgba and 0xff) shl 24 or (rgba ushr 8)).toInt()
