package dev.leptoskit.mobile

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.view.View
import android.widget.FrameLayout
import android.widget.ImageView
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.abs

/** Real Android views rendered from the `Vector` wire form, drawn into an offscreen bitmap. */
class NativeVectorViewTest {
    @Test
    fun pathsMapOntoTheFrameAndStrokesKeepLayoutUnitWidths() =
        onMain {
            val (root, vector) =
                render(
                    frame = "0, 0, 48, 24",
                    drawing =
                        """
                        {"view_box":{"min_x":0,"min_y":0,"width":24,"height":12},"scaling":"stretch","paths":[
                          {"data":"M 0 0 L 24 0 L 24 12 Z","fill":$RED,
                           "stroke":{"color":$INK,"width":1.5,"cap":"round","join":"bevel"}},
                          {"data":"M 0 0 L 4 0 L 0 4 Z","fill":$INK}
                        ]}
                        """,
                )
            assertFalse(root.clipChildren)
            assertEquals(2, vector.framePaths.size)
            assertBounds(RectF(0f, 0f, 48f, 24f), vector, 0)
            assertBounds(RectF(0f, 0f, 8f, 8f), vector, 1)

            val first = checkNotNull(vector.drawing).paths.first()
            val stroke = checkNotNull(first.stroke)
            assertEquals(1.5f, stroke.width)
            assertEquals(Paint.Cap.ROUND, stroke.cap)
            assertEquals(Paint.Join.BEVEL, stroke.join)
            assertEquals(Color.RED, first.fill)
        }

    @Test
    fun fitCentresTheViewBoxAndLayoutChangesRemap() =
        onMain {
            val (root, vector) =
                render(
                    frame = "0, 0, 40, 20",
                    drawing =
                        """
                        {"view_box":{"min_x":-5,"min_y":-5,"width":10,"height":10},
                         "paths":[{"data":"M -5 -5 L 5 -5 L 5 5 L -5 5 Z","fill":$INK}]}
                        """,
                )
            assertBounds(RectF(10f, 0f, 30f, 20f), vector, 0)

            vector.layoutParams = FrameLayout.LayoutParams(px(30f).toInt(), px(60f).toInt())
            layout(root)
            assertBounds(RectF(0f, 15f, 30f, 45f), vector, 0)
        }

    @Test
    fun labelledDrawingsAreImagesAndUnlabelledOnesAreDecorative() =
        onMain {
            val (_, labelled) = render("0, 0, 10, 10", EMPTY_DRAWING, label = "Logo")
            assertEquals(View.IMPORTANT_FOR_ACCESSIBILITY_YES, labelled.importantForAccessibility)
            assertEquals("Logo", labelled.contentDescription)
            assertEquals(ImageView::class.java.name, labelled.accessibilityClassName)

            val (_, decorative) = render("0, 0, 10, 10", EMPTY_DRAWING)
            assertEquals(View.IMPORTANT_FOR_ACCESSIBILITY_NO, decorative.importantForAccessibility)
        }

    @Test
    fun strokesPaintFromTheTopAndOverhangTheFrame() =
        onMain {
            val (root, _) =
                render(
                    frame = "10, 10, 20, 20",
                    drawing =
                        """
                        {"view_box":{"min_x":0,"min_y":0,"width":10,"height":10},"scaling":"stretch",
                         "paths":[{"data":"M 0 0 L 10 0","stroke":{"color":$RED,"width":4}}]}
                        """,
                )
            val bitmap = Bitmap.createBitmap(root.width, root.height, Bitmap.Config.ARGB_8888)
            root.draw(Canvas(bitmap))

            assertEquals(Color.RED, bitmap.getPixel(px(20f).toInt(), px(9f).toInt()))
            assertEquals(Color.RED, bitmap.getPixel(px(20f).toInt(), px(11f).toInt()))
            assertNotEquals(Color.RED, bitmap.getPixel(px(20f).toInt(), px(28f).toInt()))
        }

    @Test
    fun malformedPathDataFailsTheBatch() =
        onMain {
            for (data in listOf("M 0 0 A 1 1 0 0 0 2 2", "L 0 0", "M 0", "M 0 0 L x 1")) {
                val drawing = """{"view_box":{"min_x":0,"min_y":0,"width":1,"height":1},"paths":[{"data":"$data"}]}"""
                assertThrows(IllegalArgumentException::class.java) { render("0, 0, 1, 1", drawing) }
            }
        }

    private fun render(
        frame: String,
        drawing: String,
        label: String? = null,
    ): Pair<FrameLayout, NativeVectorView> {
        val (x, y, width, height) = frame.split(", ")
        val accessibility = label?.let { ""","accessibility_label":"$it"""" } ?: ""
        val root = FrameLayout(context())
        NativeWidgetRenderer(root, event = { _, _, _ -> }).render(
            JSONObject(
                """
                {"version":1,"root":0,"mutations":[
                  {"Create":{"id":0,"kind":"View","props":{}}},
                  {"Create":{"id":1,"kind":"vector","props":{"drawing":$drawing$accessibility}}},
                  {"InsertChild":{"parent":0,"child":1,"index":0}},
                  {"UpdateLayout":{"id":1,"x":$x,"y":$y,"width":$width,"height":$height}}
                ]}
                """,
            ),
        )
        val parent = root.getChildAt(0) as FrameLayout
        val vector = parent.getChildAt(0) as NativeVectorView
        layout(root)
        assertTrue(vector.width > 0 && vector.height > 0)

        return parent to vector
    }

    private fun layout(root: FrameLayout) {
        val size = View.MeasureSpec.makeMeasureSpec(px(CANVAS).toInt(), View.MeasureSpec.EXACTLY)
        root.measure(size, size)
        root.layout(0, 0, root.measuredWidth, root.measuredHeight)
    }

    // The one-argument replacement needs API 34; generated apps support API 26.
    @Suppress("DEPRECATION")
    private fun assertBounds(
        expected: RectF,
        vector: NativeVectorView,
        index: Int,
    ) {
        val bounds = RectF()
        vector.framePaths[index].computeBounds(bounds, true)
        val pixels = RectF(px(expected.left), px(expected.top), px(expected.right), px(expected.bottom))
        val edges = listOf(bounds.left to pixels.left, bounds.top to pixels.top, bounds.right to pixels.right)
        for ((actual, wanted) in edges + (bounds.bottom to pixels.bottom)) {
            assertTrue("expected $pixels, got $bounds", abs(actual - wanted) <= 1f)
        }
    }

    private fun px(dp: Float): Float = dp * context().resources.displayMetrics.density

    private fun context() = InstrumentationRegistry.getInstrumentation().targetContext

    /** Views are built and drawn on the main thread; failures are rethrown on the test thread. */
    private fun <T> onMain(block: () -> T): T {
        var result: Result<T>? = null
        InstrumentationRegistry.getInstrumentation().runOnMainSync { result = runCatching(block) }

        return checkNotNull(result).getOrThrow()
    }

    private companion object {
        const val RED = 0xFF0000FFL
        const val INK = 0x112233FFL
        const val CANVAS = 200f
        const val EMPTY_DRAWING = """{"view_box":{"min_x":0,"min_y":0,"width":1,"height":1},"paths":[]}"""
    }
}
