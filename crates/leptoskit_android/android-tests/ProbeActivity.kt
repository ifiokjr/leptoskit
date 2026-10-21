package dev.leptoskit.widgets

import android.app.Activity
import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Bundle
import android.util.Log
import android.view.KeyEvent
import android.view.View
import android.view.ViewGroup
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.Button
import android.widget.CheckBox
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.HorizontalScrollView
import android.widget.ImageView
import android.widget.ProgressBar
import android.widget.RadioButton
import android.widget.SeekBar
import android.widget.Switch
import android.widget.TextView
import org.json.JSONObject
import java.io.File
import kotlin.math.roundToInt

/** Executes assertions against actual Android Views in a real Activity. */
class ProbeActivity : Activity() {
    private val handler = android.os.Handler(android.os.Looper.getMainLooper())
    private lateinit var host: WidgetHost
    private var mounted = false
    private lateinit var root: View
    private lateinit var container: FrameLayout

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        System.loadLibrary("leptoskit_android")
        host = WidgetHost(this)
        val image = File(filesDir, "local.png")
        val bitmap = Bitmap.createBitmap(32, 32, Bitmap.Config.ARGB_8888)
        bitmap.eraseColor(0xff2563eb.toInt())
        image.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        container = FrameLayout(this)
        setContentView(container)
        container.post {
            runCatching {
                val id = nativeMount(host, container.width / host.density, container.height / host.density, image.path)
                mounted = true
                root = checkNotNull(host.view(id))
                host.mount(id, container)
                handler.postDelayed({ verifyRootFrames(id) }, 300)
            }.onFailure(::failed)
        }
    }

    override fun onStart() {
        super.onStart()
        if (::host.isInitialized && mounted) host.setForeground(true)
    }

    override fun onStop() {
        if (::host.isInitialized && mounted) host.setForeground(false)
        super.onStop()
    }

    override fun onDestroy() {
        handler.removeCallbacksAndMessages(null)
        if (mounted) {
            nativeDestroy()
            mounted = false
        }
        super.onDestroy()
    }

    private fun controls(): List<View> {
        val result = mutableListOf<View>()

        fun visit(view: View) {
            result.add(view)
            if (view is ViewGroup) repeat(view.childCount) { visit(view.getChildAt(it)) }
        }
        visit(root)
        return result
    }

    private fun verifyRootFrames(id: Long) {
        runCatching {
            val identity = root
            val controlIdentity = named("Action")
            nativeConfigure(14)
            nativeResize(container.width / host.density - 40f, container.height / host.density - 40f)
            nativeConfigure(17)
            handler.postDelayed({
                runCatching {
                    val margin = (12 * host.density).roundToInt()
                    assertion(
                        root.left == margin && root.top == margin &&
                            kotlin.math.abs(root.width - 288 * host.density) <= 1 && kotlin.math.abs(root.height - 800 * host.density) <= 1,
                        "Mounted fixed root preserves exact committed dimensions and offsets",
                    )
                    nativeConfigure(15)
                    host.mount(id, container)
                    handler.postDelayed({
                        runCatching {
                            assertion(
                                kotlin.math.abs(root.width - (container.width - 40 * host.density)) <= 1,
                                "Mounted percent root responds to logical viewport resize",
                            )
                            nativeResize(container.width / host.density, container.height / host.density)
                            handler.postDelayed({
                                runCatching {
                                    assertion(
                                        root === identity && named("Action") === controlIdentity && root.width == container.width &&
                                            root.height == container.height,
                                        "Viewport resize retains the mounted root and native control identity",
                                    )
                                    verify()
                                }.onFailure(::failed)
                            }, 100)
                        }.onFailure(::failed)
                    }, 100)
                }.onFailure(::failed)
            }, 100)
        }.onFailure(::failed)
    }

    private fun named(label: String): View = controls().first { it.contentDescription?.toString() == label }

    private fun count(index: Int): Int = JSONObject(nativeStatus()).getJSONArray("events").getInt(index)

    private fun key(
        view: View,
        code: Int,
    ) {
        view.dispatchKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, code))
        view.dispatchKeyEvent(KeyEvent(KeyEvent.ACTION_UP, code))
    }

    // A tap that never crosses the touch slop seeks to its position.
    private fun tap(
        view: View,
        fraction: Float,
    ) {
        val time = android.os.SystemClock.uptimeMillis()
        val x = view.paddingLeft + fraction * (view.width - view.paddingLeft - view.paddingRight)
        val y = view.height / 2f
        for (action in listOf(android.view.MotionEvent.ACTION_DOWN, android.view.MotionEvent.ACTION_UP)) {
            val event = android.view.MotionEvent.obtain(time, time, action, x, y, 0)
            view.dispatchTouchEvent(event)
            event.recycle()
        }
    }

    // NativeSurface commits everything through apply(), so drive each
    // single-mutation NativeBackend method directly on a second host.
    private fun verifyBackendContract() {
        val contractHost = WidgetHost(this)
        val contract = JSONObject(nativeContractMount(contractHost))
        val buttonId = contract.getLong("button")
        val contractRoot = checkNotNull(contractHost.view(contract.getLong("root"))) as ViewGroup
        val outer = checkNotNull(contractHost.view(buttonId)) as ViewGroup
        val control = outer.getChildAt(0) as Button
        assertion(
            contract.getBoolean("view") && outer.parent === contractRoot && control.text.toString() == "Contract" &&
                contract.getString("debug").startsWith("AndroidBackend"),
            "Single create, insert and layout mutations build native Views",
        )
        control.performClick()
        assertion(nativeContractStep(0) == 1, "A directly driven backend delivers native presses")
        nativeContractStep(1)
        assertion(
            control.text.toString() == "Updated" && contractHost.view(buttonId) === outer,
            "A single prop update keeps the native View",
        )
        nativeContractStep(2)
        control.performClick()
        assertion(nativeContractStep(0) == 1, "A cleared listener receives no further proposals")
        nativeContractStep(3)
        assertion(outer.parent == null && contractRoot.childCount == 0, "A single remove detaches the native child")
        nativeContractStep(4)
        assertion(contractHost.view(buttonId) == null, "A single delete releases the native node")
        nativeContractStep(5)
    }

    private fun assertion(
        value: Boolean,
        detail: String,
    ) {
        check(value) { detail }
        Log.i(TAG, "PASS $detail")
    }

    private fun verify() {
        runCatching {
            assertion(controls().any { it is TextView } && named("Action") is Button, "Text and Button are platform widgets")
            for (label in listOf("Amara Okafor", "Following", "Projects", "Overview")) {
                val view = controls().filterIsInstance<TextView>().first { it.text.toString() == label }
                assertion(
                    view.layout.lineCount == 1 && view.layout.getLineEnd(0) == label.length,
                    "Intrinsic native text preserves every glyph in $label",
                )
            }
            val input = named("Editor") as EditText
            val password = named("Secure editor") as EditText
            val toggle = named("Notifications") as Switch
            val checkbox = named("Preview") as CheckBox
            val radio = named("Comfortable") as RadioButton
            val compact = named("Compact") as RadioButton
            val slider = named("Volume") as SeekBar
            val progress = named("Completion") as ProgressBar
            val spinner = named("Loading") as ProgressBar
            val image = named("Local image") as ImageView
            val scroll = named("Scroll region") as HorizontalScrollView
            val pressable = named("Rich action")
            val action = named("Action") as Button
            assertion(
                action.gravity and android.view.Gravity.HORIZONTAL_GRAVITY_MASK == android.view.Gravity.CENTER_HORIZONTAL,
                "Button titles default to centered native alignment",
            )
            nativeConfigure(11)
            assertion(
                android.view.Gravity.getAbsoluteGravity(action.gravity, action.layoutDirection) and
                    android.view.Gravity.HORIZONTAL_GRAVITY_MASK ==
                    android.view.Gravity.RIGHT,
                "Explicit Button start alignment follows RTL writing direction",
            )
            nativeConfigure(12)
            assertion(
                android.view.Gravity.getAbsoluteGravity(action.gravity, action.layoutDirection) and
                    android.view.Gravity.HORIZONTAL_GRAVITY_MASK ==
                    android.view.Gravity.LEFT,
                "Explicit Button end alignment follows RTL writing direction",
            )
            nativeConfigure(13)
            assertion(
                input.text.toString() == "kept" && password.text.toString() == "fixture-only",
                "TextInput and PasswordInput render text when controlled value is unset",
            )
            password.setText("rejected secure edit")
            assertion(password.text.toString() == "fixture-only", "Rejected secure edit restores the text fallback")
            val painted = action.parent as View
            val buttonPixels = Bitmap.createBitmap(painted.width, painted.height, Bitmap.Config.ARGB_8888)
            painted.draw(Canvas(buttonPixels))
            assertion(
                action.background == null && input.background == null &&
                    buttonPixels.getPixel(painted.width / 2, painted.height - (3 * host.density).toInt()) == 0xff155dfc.toInt(),
                "Explicit shared Button/Input paint replaces opaque Android chrome",
            )
            val styledScreenshot = Bitmap.createBitmap(root.width, root.height, Bitmap.Config.ARGB_8888)
            root.draw(Canvas(styledScreenshot))
            File(filesDir, "styled-controls.png").outputStream().use { styledScreenshot.compress(Bitmap.CompressFormat.PNG, 100, it) }
            val wrapped = named("Wrapped text") as TextView
            val wrappedSize = WidgetHost.measureText(wrapped.text.toString(), 14f, 400, 192f, host.density, true)
            assertion(
                wrapped.layout.lineCount > 1 && kotlin.math.abs(wrapped.layout.height - wrappedSize[1] * host.density) <= 1 &&
                    kotlin.math.abs((wrapped.parent as View).height - wrapped.layout.height) <= 1,
                "Wrapped multilingual TextView matches the native Yoga text measurer",
            )
            assertion(
                wrapped.layoutDirection == View.LAYOUT_DIRECTION_RTL && wrapped.textDirection == View.TEXT_DIRECTION_RTL &&
                    wrapped.layout.getParagraphDirection(0) == -1,
                "Native text uses the shared RTL paragraph direction and logical start alignment",
            )
            assertion(
                controls().filterIsInstance<TextView>().any { it.text.toString().contains("Platform widgets") },
                "TextRun renders native text",
            )
            assertion(
                password.inputType and 0x80 != 0 && password.transformationMethod is android.text.method.PasswordTransformationMethod,
                "PasswordInput uses secure Android editing",
            )
            assertion(image.drawable != null, "Image loads a real local bitmap")
            assertion(
                progress.progress == 600000 && !progress.isIndeterminate && spinner.isIndeterminate,
                "Progress widgets render committed state",
            )
            nativeConfigure(9)
            assertion(spinner.visibility == View.INVISIBLE, "Stopped loading state hides native animation")
            nativeConfigure(10)
            assertion(
                toggle.parent is ViewGroup && (toggle.parent as View).width == (144 * host.density).toInt(),
                "Yoga owns exact outer control allocation",
            )
            val before = count(5)
            toggle.performClick()
            assertion(!toggle.isChecked && count(5) == before + 1, "Rejected switch proposal restores committed state")
            checkbox.performClick()
            assertion(!checkbox.isChecked, "Rejected checkbox proposal restores committed state")
            compact.performClick()
            assertion(!compact.isChecked && radio.isChecked, "Rejected radio proposal restores scoped selection")
            val originalPosition = slider.progress
            key(slider, KeyEvent.KEYCODE_DPAD_RIGHT)
            assertion(slider.progress == originalPosition, "Rejected slider keyboard proposal restores committed position")
            input.setText("rejected")
            assertion(input.text.toString() == "kept", "Rejected text edit restores committed value")
            nativeConfigure(1)
            toggle.performClick()
            assertion(toggle.isChecked && checkbox.isChecked, "Accepted switch updates controlled native siblings")
            radio.requestFocusFromTouch()
            key(radio, KeyEvent.KEYCODE_DPAD_RIGHT)
            assertion(compact.isChecked && !radio.isChecked && compact.hasFocus(), "Radio arrows traverse explicit group and retain focus")
            key(slider, KeyEvent.KEYCODE_DPAD_RIGHT)
            val numeric = JSONObject(nativeStatus()).getDouble("slider")
            assertion(
                numeric > 1e100 && numeric < 1.2e100 && slider.progress > originalPosition,
                "Slider normalizes Android positions through the full f64 grid",
            )
            slider.performAccessibilityAction(AccessibilityNodeInfo.ACTION_SCROLL_FORWARD, null)
            val accessibleValue = JSONObject(nativeStatus()).getDouble("slider")
            assertion(
                accessibleValue > 1.15e100 && accessibleValue < 1.25e100 &&
                    slider.createAccessibilityNodeInfo().text.toString() == accessibleValue.toString(),
                "Native slider accessibility announces and increments the full shared f64 value",
            )
            slider.performAccessibilityAction(AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD, null)
            val decremented = JSONObject(nativeStatus()).getDouble("slider")
            assertion(
                decremented < accessibleValue && decremented > 1.05e100,
                "Native slider accessibility decrements the full shared f64 value",
            )
            tap(slider, 0.9f)
            val tapped = JSONObject(nativeStatus()).getDouble("slider")
            assertion(
                tapped > 1.85e100 && tapped < 1.95e100,
                "A tap on the native slider proposes its position across the shared range",
            )
            input.requestFocus()
            input.setText("accepted")
            assertion(
                input === named("Editor") && input.text.toString() == "accepted" && input.hasFocus(),
                "Accepted edit preserves native identity and focus",
            )
            val submitted = count(2)
            input.onEditorAction(android.view.inputmethod.EditorInfo.IME_ACTION_DONE)
            assertion(count(2) == submitted + 1 && count(3) > 0, "Editing emits native focus and submit actions")
            nativeConfigure(6)
            nativeConfigure(7)
            assertion(input.text.toString() == "accepted" && input.hasFocus(), "Theme changes preserve the live editor")
            nativeConfigure(8)
            assertion(
                action === named("Action") && input === named("Editor") && action.background != null && input.background != null &&
                    input.hasFocus(),
                "Removing shared paint restores native chrome without replacing controls",
            )
            nativeConfigure(2)
            val disabled = count(5)
            toggle.performClick()
            assertion(!toggle.isEnabled && count(5) == disabled, "Disabled ancestor blocks control proposals")
            nativeConfigure(3)
            nativeConfigure(4)
            toggle.performClick()
            assertion(count(5) == disabled && !toggle.isEnabled, "Hidden ancestor blocks control proposals")
            assertion(!spinner.isShown && spinner.visibility == View.INVISIBLE, "Hidden ancestors stop and hide native loading chrome")
            nativeConfigure(5)
            val activated = count(0)
            pressable.requestFocus()
            key(pressable, KeyEvent.KEYCODE_SPACE)
            val info = pressable.createAccessibilityNodeInfo()
            assertion(
                count(0) == activated + 1 && info.className == Button::class.java.name &&
                    info.actionList.contains(AccessibilityNodeInfo.AccessibilityAction.ACTION_CLICK),
                "Pressable supports keyboard and native button accessibility",
            )
            pressable.performAccessibilityAction(AccessibilityNodeInfo.ACTION_CLICK, null)
            assertion(count(0) == activated + 2, "Pressable native accessibility activation dispatches the shared action")
            val inner = (pressable as ViewGroup).getChildAt(0)
            assertion(
                inner.importantForAccessibility == View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS,
                "Pressable children remain decorative",
            )
            val removable = named("Remove") as Button
            removable.performClick()
            assertion(
                JSONObject(nativeStatus()).getBoolean("removed") && removable.parent?.parent == null,
                "Reentrant callback safely deletes the firing native widget",
            )
            scroll.scrollTo(0, 0)
            val vertical = scroll.getChildAt(0) as android.widget.ScrollView
            vertical.scrollTo(0, (120 * host.density).toInt())
            root.postDelayed({ verifyScrollAndLifecycle(vertical, spinner) }, 250)
        }.onFailure(::failed)
    }

    private fun verifyScrollAndLifecycle(
        vertical: android.widget.ScrollView,
        spinner: ProgressBar,
    ) {
        runCatching {
            val documentHeight = vertical.getChildAt(0).height
            assertion(
                vertical.scrollY > 0 && count(7) > 0,
                "Real ScrollView sizes its document and emits logical offsets (${vertical.scrollY}, ${vertical.height}, $documentHeight)",
            )
            nativeConfigure(2)
            val disabledOffset = vertical.scrollY
            val disabledScrollEvents = count(7)
            val accessibilityScrolled = vertical.performAccessibilityAction(AccessibilityNodeInfo.ACTION_SCROLL_FORWARD, null)
            val now = android.os.SystemClock.uptimeMillis()
            val down = android.view.MotionEvent.obtain(now, now, android.view.MotionEvent.ACTION_DOWN, 30f, 120f, 0)
            val move = android.view.MotionEvent.obtain(now, now + 50, android.view.MotionEvent.ACTION_MOVE, 30f, 20f, 0)
            val up = android.view.MotionEvent.obtain(now, now + 100, android.view.MotionEvent.ACTION_UP, 30f, 20f, 0)
            vertical.dispatchTouchEvent(down)
            vertical.dispatchTouchEvent(move)
            vertical.dispatchTouchEvent(up)
            down.recycle()
            move.recycle()
            up.recycle()
            assertion(
                !accessibilityScrolled && !vertical.isEnabled && vertical.scrollY == disabledOffset && count(7) == disabledScrollEvents,
                "Disabled ancestors gate native scroll touch, keyboard, and accessibility",
            )
            nativeConfigure(3)
            val backgroundToggle = named("Notifications") as Switch
            val backgroundInput = named("Editor") as EditText
            backgroundInput.requestFocus()
            host.setForeground(false)
            val committedToggle = backgroundToggle.isChecked
            val committedText = backgroundInput.text.toString()
            val backgroundEvents = nativeStatus()
            backgroundToggle.performClick()
            backgroundInput.setText("queued background edit")
            assertion(
                backgroundToggle.isChecked == committedToggle && backgroundInput.text.toString() == committedText &&
                    nativeStatus() == backgroundEvents,
                "Background proposals restore committed values without dispatch",
            )
            awaitAnimation(spinner, false) {
                assertion(
                    spinner.visibility == View.INVISIBLE &&
                        !(spinner.indeterminateDrawable as android.graphics.drawable.Animatable).isRunning,
                    "Background lifecycle stops loading animation",
                )
                host.setForeground(true)
                assertion(
                    backgroundToggle.isChecked == committedToggle && backgroundInput.text.toString() == committedText &&
                        backgroundInput.hasFocus(),
                    "Foreground resumes committed controlled state",
                )
                awaitAnimation(spinner, true) { verifyResumed(spinner) }
            }
        }.onFailure(::failed)
    }

    private fun verifyResumed(spinner: ProgressBar) {
        assertion(
            spinner.visibility == View.VISIBLE && spinner.indeterminateDrawable.isVisible &&
                (!animatorsEnabled() || (spinner.indeterminateDrawable as android.graphics.drawable.Animatable).isRunning),
            "Foreground lifecycle resumes loading animation",
        )
        val activeEditor = named("Editor") as EditText
        activeEditor.requestFocus()
        activeEditor.setText("remove-me")
        assertion(
            activeEditor.parent == null && controls().none { it === activeEditor },
            "A native editing callback can synchronously remove its active editor",
        )
        nativeConfigure(16)
        assertion((named("Editor") as EditText).text.toString() == "accepted", "A removed editor remounts from current controlled state")
        handler.postDelayed({ runCatching { captureHardwareWindow(spinner) }.onFailure(::failed) }, 150)
    }

    private fun captureHardwareWindow(spinner: ProgressBar) {
        val screenshot = Bitmap.createBitmap(root.width, root.height, Bitmap.Config.ARGB_8888)
        val location = IntArray(2)
        root.getLocationInWindow(location)
        val bounds = android.graphics.Rect(location[0], location[1], location[0] + root.width, location[1] + root.height)
        android.view.PixelCopy.request(window, bounds, screenshot, { result ->
            runCatching {
                assertion(result == android.view.PixelCopy.SUCCESS, "Hardware-rendered native window screenshot is available")
                File(filesDir, "widgets.png").outputStream().use { screenshot.compress(Bitmap.CompressFormat.PNG, 100, it) }
                nativeDestroy()
                mounted = false
                awaitAnimation(spinner, false) {
                    assertion(
                        !(spinner.indeterminateDrawable as android.graphics.drawable.Animatable).isRunning &&
                            (root as ViewGroup).childCount == 0,
                        "Backend teardown detaches all native children and stops animation",
                    )
                    verifyBackendContract()
                    // The verifier stops the app after the verdict, so coverage builds
                    // write their profile first; other builds ignore this.
                    nativeFlushCoverage(File(filesDir, "coverage.profraw").path)
                    File(filesDir, "verdict.txt").writeText("VERIFIED\nAndroid native widget assertions passed\n")
                    Log.i(TAG, "VERIFIED Android native widget assertions passed")
                    finish()
                }
            }.onFailure(::failed)
        }, handler)
    }

    // With the animator duration scale at zero, as on CI emulators, Android
    // deliberately shows a static spinner frame that never reports running.
    private fun animatorsEnabled(): Boolean = android.animation.ValueAnimator.areAnimatorsEnabled()

    private fun awaitAnimation(
        spinner: ProgressBar,
        expected: Boolean,
        attempts: Int = 30,
        operation: () -> Unit,
    ) {
        handler.postDelayed({
            runCatching {
                if ((spinner.indeterminateDrawable as android.graphics.drawable.Animatable).isRunning == expected || attempts == 0 ||
                    !animatorsEnabled()
                ) {
                    operation()
                } else {
                    awaitAnimation(spinner, expected, attempts - 1, operation)
                }
            }.onFailure(::failed)
        }, 100)
    }

    private fun failed(error: Throwable) {
        if (::root.isInitialized && root.width > 0 && root.height > 0) {
            val screenshot = Bitmap.createBitmap(root.width, root.height, Bitmap.Config.ARGB_8888)
            root.draw(Canvas(screenshot))
            File(filesDir, "failed.png").outputStream().use { screenshot.compress(Bitmap.CompressFormat.PNG, 100, it) }
        }
        Log.e(TAG, "NOT VERIFIED", error)
        File(filesDir, "verdict.txt").writeText("NOT VERIFIED\n${error.stackTraceToString()}")
    }

    companion object {
        private const val TAG = "LeptoskitWidgets"

        @JvmStatic private external fun nativeMount(
            host: WidgetHost,
            width: Float,
            height: Float,
            image: String,
        ): Long

        @JvmStatic private external fun nativeConfigure(mode: Int)

        @JvmStatic private external fun nativeResize(
            width: Float,
            height: Float,
        )

        @JvmStatic private external fun nativeStatus(): String

        @JvmStatic private external fun nativeDestroy()

        @JvmStatic private external fun nativeContractMount(host: WidgetHost): String

        @JvmStatic private external fun nativeContractStep(step: Int): Int

        @JvmStatic private external fun nativeFlushCoverage(path: String)
    }
}
