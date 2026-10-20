package dev.leptoskit.mobile

import android.content.Context
import android.content.res.Configuration
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.text.Editable
import android.text.TextWatcher
import android.util.Log
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.ScrollView
import android.widget.TextView
import dev.leptoskit.bridge.NativeBridge
import dev.leptoskit.bridge.NativeDispatcher
import dev.leptoskit.bridge.NativeRequest
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.json.JSONObject

/** All calls and lifecycle transitions belong to Android's main thread. */
object NativeApp {
    @JvmStatic external fun create(wasm: ByteArray): Long

    @JvmStatic external fun destroy(app: Long)

    @JvmStatic external fun plugins(app: Long): Long

    @JvmStatic external fun start(
        app: Long,
        width: Float,
        height: Float,
    ): Int

    @JvmStatic external fun resize(
        app: Long,
        width: Float,
        height: Float,
        dark: Boolean,
    ): Int

    @JvmStatic external fun wake(app: Long): Int

    @JvmStatic external fun event(
        app: Long,
        node: Long,
        kind: Int,
        text: ByteArray,
    ): Int

    @JvmStatic external fun render(app: Long): ByteArray

    @JvmStatic external fun error(app: Long): ByteArray

    @JvmStatic external fun nextRequest(
        app: Long,
        status: IntArray,
    ): NativeRequest?
}

/** UIKit's counterpart: real Android controls driven by the Rust renderer. */
class LeptosKitView(
    context: Context,
    wasm: ByteArray,
    register: (Long, NativeDispatcher) -> Unit,
) : FrameLayout(context),
    AutoCloseable {
    private var app = 0L
    private val dispatcher = NativeDispatcher()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val jobs = mutableMapOf<Long, Job>()
    private val widgets = mutableMapOf<Long, View>()
    private var started = false
    private var applying = false
    private val density get() = resources.displayMetrics.density

    init {
        NativeBridge.load()
        try {
            app = NativeApp.create(wasm)
            check(app != 0L) { "Cannot create application" }
            register(NativeApp.plugins(app), dispatcher)
        } catch (error: Exception) {
            close()
            throw error
        }
    }

    override fun close() {
        scope.cancel()
        jobs.clear()
        try {
            dispatcher.close()
        } finally {
            if (app != 0L) NativeApp.destroy(app)
            app = 0
            widgets.clear()
        }
    }

    override fun onSizeChanged(
        width: Int,
        height: Int,
        oldWidth: Int,
        oldHeight: Int,
    ) {
        super.onSizeChanged(width, height, oldWidth, oldHeight)
        if (app == 0L || width <= 0 || height <= 0) return
        // Adding children during the current layout pass can discard their
        // requestLayout. Render on the next UI turn so Android measures them.
        post {
            if (app == 0L) return@post
            runUi {
                if (!started) {
                    started = true
                    checkStatus(NativeApp.start(app, width / density, height / density))
                } else {
                    val dark =
                        resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK ==
                            Configuration.UI_MODE_NIGHT_YES
                    checkStatus(NativeApp.resize(app, width / density, height / density, dark))
                }
                pump()
            }
        }
    }

    private fun checkStatus(status: Int) {
        check(status >= 0) { NativeApp.error(app).toString(Charsets.UTF_8) }
    }

    private fun runUi(block: () -> Unit) {
        try {
            block()
        } catch (error: Exception) {
            close()
            removeAllViews()
            addView(
                TextView(context).apply {
                    text = error.message ?: error.javaClass.simpleName
                    contentDescription = "leptoskit-error"
                },
            )
        }
    }

    private fun pump() {
        if (app == 0L) return
        while (true) {
            val cancelled = NativeBridge.nextCancelled(NativeApp.plugins(app))
            if (cancelled == 0L) break
            check(cancelled > 0) { "Invalid cancellation notification" }
            jobs.remove(cancelled)?.cancel()
        }
        val status = IntArray(1)
        repeat(64) {
            val request = NativeApp.nextRequest(app, status)
            checkStatus(status[0])
            if (status[0] == 0) {
                render()
                return
            }
            if (status[0] == 2) {
                render()
                post { runUi { pump() } }
                return
            }
            checkNotNull(request)
            // Main (not Main.immediate) ensures the map entry precedes execution.
            jobs[request.id] =
                scope
                    .launch {
                        var code = 0
                        val payload =
                            try {
                                dispatcher.dispatch(request)
                            } catch (error: CancellationException) {
                                throw error
                            } catch (error: PluginException) {
                                code = error.code
                                byteArrayOf()
                            } catch (error: Exception) {
                                Log.e("LeptosKit", "Native plugin call failed", error)
                                code = PluginException.NATIVE
                                byteArrayOf()
                            }
                        if (app == 0L) return@launch
                        jobs.remove(request.id)
                        runUi {
                            val completed = NativeBridge.complete(NativeApp.plugins(app), request.id, code, payload)
                            if (completed != -8) {
                                checkStatus(completed)
                                checkStatus(NativeApp.wake(app))
                                pump()
                            }
                        }
                    }.also { job ->
                        // If cancelled before its first suspension/body, dispatch never
                        // takes ownership. close is idempotent in either completion path.
                        job.invokeOnCompletion { request.close() }
                    }
        }
        render()
        post { runUi { pump() } }
    }

    private fun event(
        node: Long,
        kind: Int,
        value: String = "",
    ) {
        if (app == 0L || applying) return
        runUi {
            checkStatus(NativeApp.event(app, node, kind, value.toByteArray(Charsets.UTF_8)))
            pump()
        }
    }

    private fun render() {
        val batch = JSONObject(NativeApp.render(app).toString(Charsets.UTF_8))
        check(batch.getInt("version") == 1)
        applying = true
        try {
            val mutations = batch.getJSONArray("mutations")
            for (index in 0 until mutations.length()) {
                val change = mutations.getJSONObject(index)
                change.optJSONObject("Create")?.let { item ->
                    val id = item.getLong("id")
                    val widget =
                        when (item.getString("kind")) {
                            "View" -> {
                                FrameLayout(context)
                            }

                            "Text", "TextRun" -> {
                                TextView(context)
                            }

                            "Button" -> {
                                Button(context).apply { setOnClickListener { event(id, 0) } }
                            }

                            "TextInput" -> {
                                EditText(context).apply {
                                    isSingleLine = true
                                    addTextChangedListener(
                                        object : TextWatcher {
                                            override fun beforeTextChanged(
                                                text: CharSequence?,
                                                start: Int,
                                                count: Int,
                                                after: Int,
                                            ) {}

                                            override fun onTextChanged(
                                                text: CharSequence?,
                                                start: Int,
                                                before: Int,
                                                count: Int,
                                            ) {
                                                event(id, 1, text?.toString() ?: "")
                                            }

                                            override fun afterTextChanged(text: Editable?) {}
                                        },
                                    )
                                    setOnEditorActionListener { _, _, _ ->
                                        event(id, 2, text.toString())
                                        true
                                    }
                                    setOnFocusChangeListener { _, focused -> event(id, if (focused) 3 else 4) }
                                }
                            }

                            "ScrollView" -> {
                                ScrollView(context).apply { addView(FrameLayout(context)) }
                            }

                            "Image" -> {
                                ImageView(context)
                            }

                            else -> {
                                error("Unsupported widget: ${item.getString("kind")}")
                            }
                        }
                    widget.tag = id
                    widgets[id] = widget
                    applyProps(widget, item.getJSONObject("props"))
                }
                change.optJSONObject("UpdateProps")?.let {
                    applyProps(checkNotNull(widgets[it.getLong("id")]), it.getJSONObject("props"))
                }
                change.optJSONObject("UpdateLayout")?.let {
                    val widget = checkNotNull(widgets[it.getLong("id")])
                    val params =
                        FrameLayout.LayoutParams(
                            (it.getDouble("width") * density).toInt(),
                            (it.getDouble("height") * density).toInt(),
                        )
                    params.leftMargin = (it.getDouble("x") * density).toInt()
                    params.topMargin = (it.getDouble("y") * density).toInt()
                    widget.layoutParams = params
                }
                change.optJSONObject("InsertChild")?.let {
                    val parent = checkNotNull(widgets[it.getLong("parent")]) as ViewGroup
                    val container = if (parent is ScrollView) parent.getChildAt(0) as ViewGroup else parent
                    val child = checkNotNull(widgets[it.getLong("child")])
                    (child.parent as? ViewGroup)?.removeView(child)
                    container.addView(child, it.getInt("index").coerceAtMost(container.childCount))
                }
                change.optJSONObject("RemoveChild")?.let {
                    val child = widgets[it.getLong("child")]
                    (child?.parent as? ViewGroup)?.removeView(child)
                }
                change.optJSONObject("Delete")?.let {
                    val child = widgets.remove(it.getLong("id"))
                    (child?.parent as? ViewGroup)?.removeView(child)
                }
            }
            if (!batch.isNull("root")) {
                val root = checkNotNull(widgets[batch.getLong("root")])
                if (root.parent !== this) {
                    (root.parent as? ViewGroup)?.removeView(root)
                    addView(root)
                }
            }
        } finally {
            applying = false
        }
    }

    private fun applyProps(
        widget: View,
        props: JSONObject,
    ) {
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
            return ((rgba ushr 8) or ((rgba and 255) shl 24)).toInt()
        }
        widget.contentDescription = string("accessibility_label")
        widget.isEnabled = boolean("enabled", true)
        widget.visibility = if (boolean("hidden", false)) View.INVISIBLE else View.VISIBLE
        widget.alpha = number("opacity", 1.0).toFloat()
        widget.background =
            GradientDrawable().apply {
                setColor(color("background", android.graphics.Color.TRANSPARENT))
                cornerRadius = number("corner_radius", 0.0).toFloat() * density
                setStroke((number("border_width", 0.0) * density).toInt(), color("border_color", 0))
            }
        widget.clipToOutline = boolean("clip", false) || number("corner_radius", 0.0) > 0
        if (widget is TextView) {
            val value = if (widget is EditText) string("value") ?: string("text") ?: "" else string("text") ?: ""
            if (widget.text.toString() != value) widget.text = value
            widget.hint = string("placeholder")
            widget.textSize = number("font_size", 14.0).toFloat()
            widget.setTextColor(color("color", android.graphics.Color.BLACK))
            widget.setTypeface(null, if (string("font_weight") in listOf("bold", "semibold")) Typeface.BOLD else Typeface.NORMAL)
            widget.gravity =
                when (string("text_align")) {
                    "center" -> Gravity.CENTER
                    "end" -> Gravity.END or Gravity.CENTER_VERTICAL
                    else -> Gravity.START or Gravity.CENTER_VERTICAL
                }
        }
        if (widget is ImageView) {
            val source = string("source")
            if (source == null) {
                widget.setImageDrawable(null)
            } else {
                val uri = android.net.Uri.parse(source)
                require(uri.scheme == "file") { "Image requires a readable file URL: $source" }
                widget.setImageURI(uri)
                check(widget.drawable != null) { "Cannot load image: $source" }
            }
        }
    }
}
