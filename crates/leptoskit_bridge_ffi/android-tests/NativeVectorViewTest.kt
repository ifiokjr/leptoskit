package dev.leptoskit.mobile

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.view.View
import android.widget.FrameLayout
import android.widget.ImageView
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONException
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNotSame
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
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
            assertEquals(2, vector.frames.size)
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

    @Test
    fun strokeStylesAndFillRulesReachTheFrames() =
        onMain {
            val (_, vector) =
                render(
                    frame = "0, 0, 40, 10",
                    drawing =
                        """
                        {"view_box":{"min_x":0,"min_y":0,"width":10,"height":10},"scaling":"stretch","paths":[
                          {"data":"M 0 0 L 10 0 L 10 10 Z","fill":$RED,"fill_rule":"evenodd",
                           "stroke":{"color":$INK,"width":1.5,"miter_limit":10,"dashes":[2,1],"dash_offset":1,
                                     "scaling":"drawing"}},
                          {"data":"M 0 0 L 10 10","stroke":{"color":$INK,"width":2,"dashes":[3],"dash_offset":0.5}},
                          {"data":"M 0 0 L 10 10","stroke":{"color":$INK,"width":2,"dashes":[0,0],"dash_offset":3}}
                        ]}
                        """,
                )
            val (scaled, fixed, solid) = vector.frames
            assertEquals(Path.FillType.EVEN_ODD, scaled.outline.fillType)
            assertEquals(Path.FillType.WINDING, fixed.outline.fillType)

            // Stretching 10 by 10 onto 40 by 10 scales the area by four, so a scaling stroke doubles.
            assertEquals(px(3f), scaled.strokeWidth, TOLERANCE)
            assertEquals(10f, checkNotNull(scaled.path.stroke).miterLimit)
            assertNotNull(scaled.dashes)

            val fixedStroke = checkNotNull(fixed.path.stroke)
            assertEquals(px(2f), fixed.strokeWidth, TOLERANCE)
            assertEquals(NativeVectorStroke.MITER_LIMIT, fixedStroke.miterLimit)
            assertEquals("an odd dash list repeats to make it even", listOf(3f, 3f), fixedStroke.dashes)
            assertEquals(0.5f, fixedStroke.dashOffset)
            assertNotNull(fixed.dashes)

            assertEquals("an all-zero dash list is solid", emptyList<Float>(), checkNotNull(solid.path.stroke).dashes)
            assertEquals(0f, checkNotNull(solid.path.stroke).dashOffset)
            assertNull(solid.dashes)
        }

    @Test
    fun dashesAndTheirOffsetScaleWithTheDisplayDensity() =
        onMain {
            val (root, _) =
                render(
                    frame = "0, 0, 40, 10",
                    drawing =
                        """
                        {"view_box":{"min_x":0,"min_y":0,"width":40,"height":10},"scaling":"stretch",
                         "paths":[{"data":"M 0 5 L 40 5",
                                   "stroke":{"color":$RED,"width":4,"dashes":[10,10],"dash_offset":5}}]}
                        """,
                )
            val bitmap = Bitmap.createBitmap(root.width, root.height, Bitmap.Config.ARGB_8888)
            root.draw(Canvas(bitmap))

            // Starting five units in, the pattern paints 0 to 5, skips 5 to 15, and paints 15 to 25.
            assertEquals(Color.RED, bitmap.getPixel(px(2f).toInt(), px(5f).toInt()))
            assertNotEquals(Color.RED, bitmap.getPixel(px(10f).toInt(), px(5f).toInt()))
            assertEquals(Color.RED, bitmap.getPixel(px(20f).toInt(), px(5f).toInt()))
        }

    @Test
    fun fillRulesDecideWhetherNestedOutlinesPunchHoles() =
        onMain {
            for ((rule, centreFilled) in listOf("evenodd" to false, "nonzero" to true)) {
                val (root, _) =
                    render(
                        frame = "0, 0, 40, 40",
                        drawing =
                            """
                            {"view_box":{"min_x":0,"min_y":0,"width":40,"height":40},"paths":[{"fill":$RED,
                             "fill_rule":"$rule","data":"M 0 0 L 40 0 L 40 40 L 0 40 Z M 10 10 L 30 10 L 30 30 L 10 30 Z"}]}
                            """,
                    )
                val bitmap = Bitmap.createBitmap(root.width, root.height, Bitmap.Config.ARGB_8888)
                root.draw(Canvas(bitmap))

                assertEquals(rule, Color.RED, bitmap.getPixel(px(5f).toInt(), px(5f).toInt()))
                assertEquals(rule, centreFilled, bitmap.getPixel(px(20f).toInt(), px(20f).toInt()) == Color.RED)
            }
        }

    @Test
    fun scalingAndAlignmentPlaceTheViewBox() =
        onMain {
            val cases =
                listOf(
                    """"scaling":"fill","align":{"x":"center","y":"start"}""" to RectF(0f, 0f, 40f, 40f),
                    """"scaling":"fill"""" to RectF(0f, -10f, 40f, 30f),
                    """"scaling":"fill","align":{"x":"center","y":"end"}""" to RectF(0f, -20f, 40f, 20f),
                    """"scaling":"fit","align":{"x":"end","y":"start"}""" to RectF(20f, 0f, 40f, 20f),
                    """"align":{"x":"start","y":"end"}""" to RectF(0f, 0f, 20f, 20f),
                )
            for ((fields, bounds) in cases) {
                val (_, vector) =
                    render(
                        frame = "0, 0, 40, 20",
                        drawing =
                            """
                            {"view_box":{"min_x":-5,"min_y":-5,"width":10,"height":10},$fields,
                             "paths":[{"data":"M -5 -5 L 5 -5 L 5 5 L -5 5 Z","fill":$INK}]}
                            """,
                    )
                assertBounds(bounds, vector, 0)
            }
        }

    @Test
    fun unchangedDrawingsKeepTheirFrames() =
        onMain {
            val drawing =
                """
                {"view_box":{"min_x":0,"min_y":0,"width":10,"height":10},
                 "paths":[{"data":"M 0 0 L 10 10","stroke":{"color":$INK,"width":1,"dashes":[1,1]}}]}
                """
            val (_, vector) = render("0, 0, 10, 10", drawing)
            val frames = vector.frames

            vector.drawing = NativeVectorDrawing.decode(JSONObject(drawing))
            assertSame(frames, vector.frames)

            vector.drawing = NativeVectorDrawing.decode(JSONObject(drawing.replace("[1,1]", "[1,2]")))
            assertNotSame(frames, vector.frames)
        }

    @Test
    fun invalidDrawingFieldsFailTheBatch(): Unit =
        onMain {
            val invalid =
                listOf(
                    """"scaling":"slice"""",
                    """"align":{"x":"middle","y":"start"}""",
                    """"paths":[{"data":"M 0 0","fill_rule":"winding"}]""",
                    """"paths":[{"data":"M 0 0","stroke":{"color":0,"width":1,"scaling":"view"}}]""",
                    """"paths":[{"data":"M 0 0","stroke":{"color":0,"width":1,"miter_limit":0.5}}]""",
                    """"paths":[{"data":"M 0 0","stroke":{"color":0,"width":1,"dashes":[-1,2]}}]""",
                    """"paths":[{"data":"M 0 0","stroke":{"color":0,"width":0}}]""",
                )
            for (fields in invalid) {
                val drawing = """{"view_box":{"min_x":0,"min_y":0,"width":1,"height":1},$fields}"""
                assertThrows(fields, IllegalArgumentException::class.java) { render("0, 0, 1, 1", drawing) }
            }

            val unaligned = """{"view_box":{"min_x":0,"min_y":0,"width":1,"height":1},"align":{"x":"start"}}"""
            assertThrows(JSONException::class.java) { render("0, 0, 1, 1", unaligned) }
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
        vector.frames[index].outline.computeBounds(bounds, true)
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

        /** Frame sizes round to whole pixels, so pixel lengths derived from them differ slightly. */
        const val TOLERANCE = 0.1f
        const val EMPTY_DRAWING = """{"view_box":{"min_x":0,"min_y":0,"width":1,"height":1},"paths":[]}"""
    }
}
