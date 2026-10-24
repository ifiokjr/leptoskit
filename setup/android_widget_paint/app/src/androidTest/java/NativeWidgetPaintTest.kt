package dev.leptoskit.mobile

import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.os.SystemClock
import android.view.Choreographer
import android.view.MotionEvent
import android.view.View
import android.widget.Button
import android.widget.FrameLayout
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

@RunWith(AndroidJUnit4::class)
class NativeWidgetPaintTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context get() = instrumentation.targetContext

    @Test
    fun compareNativeInsetsAtObservedSize() {
        val activity =
            instrumentation.startActivitySync(
                Intent(context, PaintActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            ) as PaintActivity
        instrumentation.runOnMainSync {
            val root = FrameLayout(activity)
            root.setBackgroundColor(Color.WHITE)
            activity.setContentView(root)
            val renderer = NativeWidgetRenderer(root) { _, _, _ -> }
            renderer.render(batch("Continue journey", OBSERVED_HEIGHT))
            val button = root.getChildAt(0) as Button
            val before = draw(button, "native")
            button.setPadding(0, 0, 0, 0)
            val padding = draw(button, "no-padding")
            button.includeFontPadding = false
            val font = draw(button, "no-font-padding")
            println("PAINT density=${root.resources.displayMetrics.density} native=$before padding=$padding font=$font")
        }
        awaitFrame()
        instrumentation.uiAutomation.takeScreenshot().apply {
            File(context.filesDir, "native-screen.png").outputStream().use {
                compress(Bitmap.CompressFormat.PNG, 100, it)
            }
            recycle()
        }
        instrumentation.runOnMainSync { activity.finish() }
    }

    @Test
    fun sharedButtonPaintsAndPreservesState() {
        instrumentation.runOnMainSync {
            val root = FrameLayout(context)
            var clicks = 0
            val renderer =
                NativeWidgetRenderer(root) { id, kind, value ->
                    assertEquals(1L, id)
                    assertEquals(0, kind)
                    assertEquals("", value)
                    clicks += 1
                }
            renderer.render(batch("Continue journey", OBSERVED_HEIGHT))
            val button = root.getChildAt(0) as Button
            val painted = draw(button, "shared")
            assertTrue("Shared 16-unit button paints its label", painted > 0)
            val originalHeight = button.layoutParams.height
            button.layoutParams.height = originalHeight * 2
            assertEquals("The short frame preserves every glyph pixel", draw(button, "tall"), painted)
            button.layoutParams.height = originalHeight
            assertTrue(button.performClick())
            assertEquals(1, clicks)
            NativeWidgetProps { root.resources.displayMetrics.density }.apply(
                button,
                JSONObject()
                    .put("text", "Updated")
                    .put("enabled", false)
                    .put("accessibility_label", "Updated action")
                    .put("font_size", FONT_SIZE),
            )
            assertFalse(button.isEnabled)
            assertEquals("Updated", button.text.toString())
            assertEquals("Updated action", button.contentDescription)
            assertTrue(draw(button, "disabled-updated") > 0)
            assertEquals((OBSERVED_WIDTH * root.resources.displayMetrics.density).toInt(), button.width)
            assertEquals((OBSERVED_HEIGHT * root.resources.displayMetrics.density).toInt(), button.height)
            assertTrue(button.includeFontPadding)
        }
    }

    @Test
    fun paintedButtonReceivesRealTouch() {
        val activity =
            instrumentation.startActivitySync(
                Intent(context, PaintActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            ) as PaintActivity
        var clicks = 0
        lateinit var button: Button
        instrumentation.runOnMainSync {
            val root = FrameLayout(activity)
            root.setBackgroundColor(Color.WHITE)
            activity.setContentView(root)
            NativeWidgetRenderer(root) { _, _, _ -> clicks += 1 }
                .render(batch("Continue journey", OBSERVED_HEIGHT))
            button = root.getChildAt(0) as Button
        }
        awaitFrame()
        val location = IntArray(2)
        instrumentation.runOnMainSync { button.getLocationOnScreen(location) }

        val down = SystemClock.uptimeMillis()

        for (action in listOf(MotionEvent.ACTION_DOWN, MotionEvent.ACTION_UP)) {
            val event =
                MotionEvent.obtain(
                    down,
                    SystemClock.uptimeMillis(),
                    action,
                    location[0] + button.width / 2f,
                    location[1] + button.height / 2f,
                    0,
                )
            instrumentation.sendPointerSync(event)
            event.recycle()
        }

        instrumentation.waitForIdleSync()
        assertEquals("Real screen touch invokes the shared callback once", 1, clicks)
        instrumentation.uiAutomation.takeScreenshot().apply {
            File(context.filesDir, "touch-screen.png").outputStream().use {
                compress(Bitmap.CompressFormat.PNG, 100, it)
            }
            recycle()
        }
        instrumentation.runOnMainSync { activity.finish() }
    }

    @Test
    fun scaledFontRespectsExplicitLayout() {
        instrumentation.runOnMainSync {
            val configuration = android.content.res.Configuration(context.resources.configuration)
            configuration.fontScale = 1.5f
            val scaled = context.createConfigurationContext(configuration)
            val root = FrameLayout(scaled)
            NativeWidgetRenderer(root) { _, _, _ -> }.render(batch("Scaled label", OBSERVED_HEIGHT * 2))
            val button = root.getChildAt(0) as Button
            assertTrue(button.textSize > FONT_SIZE * root.resources.displayMetrics.density)
            assertTrue(draw(button, "font-scaled") > 0)
            assertEquals((OBSERVED_HEIGHT * 2 * root.resources.displayMetrics.density).toInt(), button.height)
        }
    }

    private fun awaitFrame() {
        val frame = CountDownLatch(1)
        instrumentation.runOnMainSync {
            Choreographer.getInstance().postFrameCallback {
                Choreographer.getInstance().postFrameCallback { frame.countDown() }
            }
        }
        assertTrue("Native window completes two frames", frame.await(5, TimeUnit.SECONDS))
        instrumentation.waitForIdleSync()
    }

    private fun batch(
        text: String,
        height: Double,
    ): JSONObject =
        JSONObject().put("root", 1).put(
            "mutations",
            JSONArray()
                .put(
                    JSONObject().put(
                        "Create",
                        JSONObject().put("id", 1).put("kind", "button").put(
                            "props",
                            JSONObject().put("text", text).put("font_size", FONT_SIZE),
                        ),
                    ),
                ).put(
                    JSONObject().put(
                        "UpdateLayout",
                        JSONObject()
                            .put("id", 1)
                            .put("x", 24)
                            .put("y", OBSERVED_TOP)
                            .put("width", OBSERVED_WIDTH)
                            .put("height", height),
                    ),
                ),
        )

    private fun draw(
        button: Button,
        name: String,
    ): Int {
        val width = button.layoutParams.width
        val height = button.layoutParams.height
        button.measure(
            View.MeasureSpec.makeMeasureSpec(width, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(height, View.MeasureSpec.EXACTLY),
        )
        button.layout(0, 0, width, height)
        val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
        bitmap.eraseColor(Color.WHITE)
        button.draw(Canvas(bitmap))
        val pixels = IntArray(width * height)
        bitmap.getPixels(pixels, 0, width, 0, 0, width, height)
        val count = pixels.count { Color.red(it) < DARK_THRESHOLD }
        File(context.filesDir, "$name.png").outputStream().use {
            bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)
        }
        bitmap.recycle()
        println(
            "PAINT $name pixels=$count frame=${width}x$height padding=${button.paddingTop},${button.paddingBottom} text=${button.textSize} fontPadding=${button.includeFontPadding}",
        )

        return count
    }

    private companion object {
        const val OBSERVED_HEIGHT = 16.0
        const val OBSERVED_WIDTH = 320
        const val OBSERVED_TOP = 188
        const val FONT_SIZE = 14
        const val DARK_THRESHOLD = 128
    }
}
