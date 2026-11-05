package dev.leptoskit.mobile

import android.view.MotionEvent
import android.view.View
import android.view.inputmethod.EditorInfo
import android.widget.CompoundButton
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.ProgressBar
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/** Exercises the same renderer generated APKs use, with real Android controls. */
class NativeControlsTest {
    private fun onMain(block: () -> Unit) {
        InstrumentationRegistry.getInstrumentation().runOnMainSync(block)
    }

    @Test
    fun backAvailabilityFollowsRootAndEmitsGuestKey() =
        onMain {
            val root = FrameLayout(InstrumentationRegistry.getInstrumentation().targetContext)
            val events = mutableListOf<Triple<Long, Int, String>>()
            val renderer = NativeWidgetRenderer(root, { id, kind, payload -> events.add(Triple(id, kind, payload)) })
            renderer.render(RenderBatch(1, listOf(RenderMutation.Create(1, "view", props("{}")))))
            assertFalse(renderer.canGoBack)
            assertFalse(renderer.back())
            renderer.render(RenderBatch(null, listOf(RenderMutation.UpdateProps(1, props("{\"can_go_back\":true}")))))
            assertTrue(renderer.canGoBack)
            assertTrue(renderer.back())
            assertEquals(1L, events.single().first)
            assertEquals(10, events.single().second)
            assertEquals("BrowserBack", JSONObject(events.single().third).getString("key"))
            renderer.render(RenderBatch(null, listOf(RenderMutation.UpdateProps(1, props("{}")))))
            assertFalse(renderer.back())
            renderer.clear()
            assertFalse(renderer.canGoBack)
        }

    @Test
    fun togglesEmitGuestPayloadsAndReconciliationIsSilent() =
        onMain {
            for (kind in listOf("switch", "checkbox", "radio")) {
                val root = FrameLayout(InstrumentationRegistry.getInstrumentation().targetContext)
                val events = mutableListOf<Pair<Int, String>>()
                val renderer = NativeWidgetRenderer(root, { _, kind, payload -> events.add(kind to payload) })
                renderer.render(RenderBatch(1, listOf(RenderMutation.Create(1, kind, props("{\"checked\":false}")))))
                val control = root.getChildAt(0) as CompoundButton
                assertTrue(events.isEmpty())
                control.performClick()
                assertEquals(listOf(6 to "{\"checked\":true}"), events)
                assertFalse("Unacknowledged changes must not override guest state", control.isChecked)
                events.clear()
                renderer.render(RenderBatch(null, listOf(RenderMutation.UpdateProps(1, props("{\"checked\":true}")))))
                assertTrue(events.isEmpty())
                control.performClick()
                if (kind == "radio") {
                    assertTrue(control.isChecked)
                    assertTrue(events.isEmpty())
                } else {
                    assertEquals(listOf(6 to "{\"checked\":false}"), events)
                }
                renderer.render(RenderBatch(null, listOf(RenderMutation.UpdateProps(1, props("{\"checked\":false,\"enabled\":false}")))))
                assertFalse(control.isEnabled)
                if (kind != "switch") {
                    control.layout(0, 0, 40, 40)
                    val bitmap = android.graphics.Bitmap.createBitmap(40, 40, android.graphics.Bitmap.Config.ARGB_8888)
                    control.draw(android.graphics.Canvas(bitmap))
                    assertTrue(checkNotNull(control.buttonDrawable).bounds.right <= control.width)
                    assertTrue(checkNotNull(control.buttonDrawable).bounds.bottom <= control.height)
                    bitmap.recycle()
                }
            }
        }

    @Test
    fun sliderMapsRangeAndStepsAndProgressCanStop() =
        onMain {
            val root = FrameLayout(InstrumentationRegistry.getInstrumentation().targetContext)
            val events = mutableListOf<String>()
            val renderer = NativeWidgetRenderer(root, { _, _, payload -> events.add(payload) })
            renderer.render(
                RenderBatch(
                    1,
                    listOf(
                        RenderMutation.Create(
                            1,
                            "slider",
                            props(
                                """
            {"slider":{"min":10,"max":20,"value":14,"step":2},"accessibility_label":"Volume"}
        """,
                            ),
                        ),
                    ),
                ),
            )
            val slider = root.getChildAt(0) as NativeSlider
            assertEquals(4000, slider.progress)
            assertEquals(16.0, slider.proposedValue(5500), 0.0)
            assertEquals(20.0, slider.proposedValue(10000), 0.0)
            assertTrue(events.isEmpty())
            val arguments =
                android.os.Bundle().apply {
                    putFloat(android.view.accessibility.AccessibilityNodeInfo.ACTION_ARGUMENT_PROGRESS_VALUE, 8000f)
                }
            assertTrue(
                slider.performAccessibilityAction(
                    android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction.ACTION_SET_PROGRESS.id,
                    arguments,
                ),
            )
            assertEquals(18.0, JSONObject(events.single()).getDouble("value"), 0.0)
            val progressRoot = FrameLayout(root.context)
            val progressRenderer = NativeWidgetRenderer(progressRoot, { _, _, _ -> fail("Progress is not interactive") })
            progressRenderer.render(
                RenderBatch(
                    1,
                    listOf(RenderMutation.Create(1, "linear_progress_indicator", props("{\"progress\":{\"determinate\":0.6}}"))),
                ),
            )
            val progress = progressRoot.getChildAt(0) as ProgressBar
            assertFalse(progress.isIndeterminate)
            assertEquals(6000, progress.progress)
            progressRenderer.render(RenderBatch(null, listOf(RenderMutation.UpdateProps(1, props("{\"running\":false}")))))
            assertTrue(progress.isIndeterminate)
            assertEquals(View.INVISIBLE, progress.visibility)
        }

    @Test
    fun outsideTouchAndSubmitClearFocusWithoutSwallowingPresses() =
        onMain {
            val root = FrameLayout(InstrumentationRegistry.getInstrumentation().targetContext)
            val events = mutableListOf<Int>()
            val renderer = NativeWidgetRenderer(root, { _, kind, _ -> events.add(kind) })
            renderer.render(
                RenderBatch(
                    1,
                    listOf(
                        RenderMutation.Create(1, "view", WidgetProps()),
                        RenderMutation.Create(2, "text_input", WidgetProps(value = "hello")),
                        RenderMutation.InsertChild(1, 2, 0),
                    ),
                ),
            )
            val editor = (root.getChildAt(0) as FrameLayout).getChildAt(0) as EditText
            editor.isFocusableInTouchMode = true
            assertTrue(editor.requestFocus())
            events.clear()
            val down = MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, 200f, 200f, 0)
            dismissEditorOutside(root, down)
            down.recycle()
            assertFalse(editor.hasFocus())
            assertEquals(listOf(4), events)
            assertTrue(editor.requestFocus())
            events.clear()
            editor.onEditorAction(EditorInfo.IME_ACTION_DONE)
            assertFalse(editor.hasFocus())
            assertEquals(listOf(4, 2), events)
        }

    @Test
    fun nextMovesToPasswordWithoutSubmitting() =
        onMain {
            val root = FrameLayout(InstrumentationRegistry.getInstrumentation().targetContext)
            val events = mutableListOf<Int>()
            val renderer = NativeWidgetRenderer(root, { _, kind, _ -> events.add(kind) })
            renderer.render(
                RenderBatch(
                    1,
                    listOf(
                        RenderMutation.Create(1, "view", WidgetProps()),
                        RenderMutation.Create(2, "text_input", WidgetProps()),
                        RenderMutation.Create(3, "password_input", WidgetProps()),
                        RenderMutation.InsertChild(1, 2, 0),
                        RenderMutation.InsertChild(1, 3, 1),
                    ),
                ),
            )
            val content = root.getChildAt(0) as FrameLayout
            val email = content.getChildAt(0) as EditText
            val password = content.getChildAt(1) as EditText
            assertEquals(EditorInfo.IME_ACTION_NEXT, email.imeOptions)
            assertEquals(EditorInfo.IME_ACTION_DONE, password.imeOptions)
            assertTrue(email.requestFocus())
            events.clear()
            email.onEditorAction(EditorInfo.IME_ACTION_NEXT)
            assertTrue(password.hasFocus())
            assertFalse(events.contains(2))
            assertTrue(email.requestFocus())
            events.clear()
            email.dispatchKeyEvent(android.view.KeyEvent(android.view.KeyEvent.ACTION_DOWN, android.view.KeyEvent.KEYCODE_ENTER))
            email.dispatchKeyEvent(android.view.KeyEvent(android.view.KeyEvent.ACTION_UP, android.view.KeyEvent.KEYCODE_ENTER))
            assertTrue(password.hasFocus())
            assertFalse(events.contains(2))
        }

    private fun props(json: String) = WidgetProps.fromJson(JSONObject(json))
}
