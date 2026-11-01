package dev.leptoskit.plugins.brightness

import android.app.Activity
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Real generated-application window attributes, never framework stubs. */
class BrightnessPluginTest {
    @Test
    fun overrideRestoreAndClosePreserveTheWindowBaseline() =
        runBlocking {
            withActivity { activity -> overrideRestoreAndClose(activity) }
        }

    @Test
    fun invalidLevelsDoNotMutateTheWindow() =
        runBlocking {
            withActivity { activity -> invalidLevelsDoNotMutateTheWindow(activity) }
        }

    /** An override, an explicit restore and a close each end where they began. */
    private suspend fun overrideRestoreAndClose(activity: Activity) {
        val baseline = activity.window.attributes.screenBrightness
        val plugin = BrightnessPlugin(activity)

        try {
            setWindow(activity, INHERITED)
            assertEquals(INHERITED_LEVEL, read(plugin))

            plugin.invoke(WRITE, level(THREE_EIGHTHS))
            assertEquals(THREE_EIGHTHS / THOUSANDTHS_PER_UNIT, activity.window.attributes.screenBrightness, TOLERANCE)
            assertEquals(THREE_EIGHTHS, read(plugin))

            plugin.invoke(WRITE, level(FOUR_FIFTHS))
            assertEquals(FOUR_FIFTHS / THOUSANDTHS_PER_UNIT, activity.window.attributes.screenBrightness, TOLERANCE)

            // Restoring twice is safe: the second call has nothing left to undo.
            plugin.invoke(RESTORE, empty())
            assertEquals(INHERITED, activity.window.attributes.screenBrightness, 0f)

            plugin.invoke(RESTORE, empty())
            assertEquals(INHERITED, activity.window.attributes.screenBrightness, 0f)

            // Close restores the baseline current at the override, not the first one.
            setWindow(activity, LATER_BASELINE)
            plugin.invoke(WRITE, level(ONE_FIFTH))
            plugin.close()

            assertEquals(LATER_BASELINE, activity.window.attributes.screenBrightness, TOLERANCE)

            plugin.close()
            expectError(PluginException.CANCELLED) { plugin.invoke(READ, empty()) }
        } finally {
            plugin.close()
            setWindow(activity, baseline)
        }
    }

    /** A level the plugin refuses leaves the window exactly as it was. */
    private suspend fun invalidLevelsDoNotMutateTheWindow(activity: Activity) {
        val baseline = activity.window.attributes.screenBrightness
        val plugin = BrightnessPlugin(activity)

        try {
            for (value in listOf(INHERITED_LEVEL, ABOVE_MAXIMUM, Int.MAX_VALUE)) {
                expectError(PluginException.INVALID_ARGUMENT) { plugin.invoke(WRITE, level(value)) }

                assertEquals(baseline, activity.window.attributes.screenBrightness, 0f)
            }

            expectError(PluginException.INVALID_ARGUMENT) { plugin.invoke(WRITE, ByteBuffer.allocate(TRUNCATED)) }
            expectError(PluginException.INVALID_ARGUMENT) { plugin.invoke(RESTORE, level(0)) }

            assertEquals(baseline, activity.window.attributes.screenBrightness, 0f)
        } finally {
            plugin.close()
            setWindow(activity, baseline)
        }
    }

    /** Launches the generated app and runs the body on its main thread. */
    private suspend fun withActivity(body: suspend (Activity) -> Unit) {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val intent = requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName))

        ActivityScenario.launch<Activity>(intent).use { scenario ->
            lateinit var activity: Activity

            scenario.onActivity { activity = it }

            withContext(Dispatchers.Main) { body(activity) }
        }
    }

    private suspend fun read(plugin: BrightnessPlugin): Int =
        ByteBuffer
            .wrap(plugin.invoke(READ, empty()))
            .order(ByteOrder.LITTLE_ENDIAN)
            .int

    private fun setWindow(
        activity: Activity,
        brightness: Float,
    ) {
        val attributes = activity.window.attributes

        attributes.screenBrightness = brightness
        activity.window.attributes = attributes
    }

    private fun level(value: Int): ByteBuffer =
        ByteBuffer
            .allocate(LEVEL_BYTES)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(value)
            .apply { flip() }

    private fun empty(): ByteBuffer = ByteBuffer.allocate(0)

    private suspend fun expectError(
        code: Int,
        action: suspend () -> Unit,
    ) {
        try {
            action()

            fail("Expected plugin error $code")
        } catch (error: PluginException) {
            assertEquals(code, error.code)
        }
    }

    private companion object {
        /** Level in thousandths, and the wire sentinel for an inherited level. */
        const val INHERITED_LEVEL = -1

        /** The window attribute behind that sentinel. */
        const val INHERITED = -1f

        /** Wire ids the plugin exposes. */
        const val READ = 0
        const val WRITE = 1
        const val RESTORE = 2

        /** Levels that round-trip exactly through thousandths. */
        const val THREE_EIGHTHS = 375
        const val FOUR_FIFTHS = 800
        const val ONE_FIFTH = 200

        /** A window baseline set after the first override. */
        const val LATER_BASELINE = 0.6f

        /** The first level past the accepted range. */
        const val ABOVE_MAXIMUM = 1001

        /** A payload one byte short of the u32 a level needs. */
        const val TRUNCATED = 3

        /** A level crosses the wire as one little-endian u32. */
        const val LEVEL_BYTES = 4

        /** Levels and windows are floats; allow the last digit of rounding. */
        const val TOLERANCE = 0.0001f

        /** The wire unit: brightness levels are thousandths of a fraction. */
        const val THOUSANDTHS_PER_UNIT = 1000f
    }
}
