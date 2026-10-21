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
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val intent = requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName))
            ActivityScenario.launch<Activity>(intent).use { scenario ->
                lateinit var activity: Activity
                scenario.onActivity { activity = it }
                withContext(Dispatchers.Main) {
                    val original = activity.window.attributes.screenBrightness
                    val plugin = BrightnessPlugin(activity)
                    try {
                        setWindow(activity, -1f)
                        assertEquals(-1, read(plugin))
                        plugin.invoke(1, level(375))
                        assertEquals(0.375f, activity.window.attributes.screenBrightness, 0.0001f)
                        assertEquals(375, read(plugin))
                        plugin.invoke(1, level(800))
                        assertEquals(0.8f, activity.window.attributes.screenBrightness, 0.0001f)
                        plugin.invoke(2, empty())
                        assertEquals(-1f, activity.window.attributes.screenBrightness, 0f)
                        plugin.invoke(2, empty())
                        assertEquals(-1f, activity.window.attributes.screenBrightness, 0f)
                        setWindow(activity, 0.6f)
                        plugin.invoke(1, level(200))
                        plugin.close()
                        assertEquals(0.6f, activity.window.attributes.screenBrightness, 0.0001f)
                        plugin.close()
                        expectError(PluginException.CANCELLED) { plugin.invoke(0, empty()) }
                    } finally {
                        plugin.close()
                        setWindow(activity, original)
                    }
                }
            }
        }

    @Test
    fun invalidLevelsDoNotMutateTheWindow() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val intent = requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName))
            ActivityScenario.launch<Activity>(intent).use { scenario ->
                lateinit var activity: Activity
                scenario.onActivity { activity = it }
                withContext(Dispatchers.Main) {
                    val original = activity.window.attributes.screenBrightness
                    val plugin = BrightnessPlugin(activity)
                    try {
                        for (value in listOf(-1, 1001, Int.MAX_VALUE)) {
                            expectError(PluginException.INVALID_ARGUMENT) { plugin.invoke(1, level(value)) }
                            assertEquals(original, activity.window.attributes.screenBrightness, 0f)
                        }
                        expectError(PluginException.INVALID_ARGUMENT) { plugin.invoke(1, ByteBuffer.allocate(3)) }
                        expectError(PluginException.INVALID_ARGUMENT) { plugin.invoke(2, level(0)) }
                        assertEquals(original, activity.window.attributes.screenBrightness, 0f)
                    } finally {
                        plugin.close()
                        setWindow(activity, original)
                    }
                }
            }
        }

    private suspend fun read(plugin: BrightnessPlugin): Int = ByteBuffer.wrap(plugin.invoke(0, empty())).order(ByteOrder.LITTLE_ENDIAN).int

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
            .allocate(4)
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
}
