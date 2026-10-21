package dev.leptoskit.plugins.screenorientation

import android.app.Activity
import android.app.Dialog
import android.content.pm.ActivityInfo
import android.content.res.Configuration
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Assert.fail
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

class ScreenOrientationPluginTest {
    @Test
    fun cancellationAndCloseRestoreTheExactActivityPolicy() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val intent = requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName))
            ActivityScenario.launch<Activity>(intent).use { scenario ->
                lateinit var activity: Activity
                scenario.onActivity { activity = it }
                withTimeout(5000) { while (!withContext(Dispatchers.Main) { activity.hasWindowFocus() }) delay(10) }
                val plugin = withContext(Dispatchers.Main) { ScreenOrientationPlugin(activity) }
                val original = withContext(Dispatchers.Main) { activity.requestedOrientation }
                // The generated host handles orientation/screenSize without recreating this Activity.
                val policy =
                    withContext(Dispatchers.Main) {
                        if (activity.resources.configuration.orientation ==
                            Configuration.ORIENTATION_LANDSCAPE
                        ) {
                            2
                        } else {
                            4
                        }
                    }
                val expected = if (policy == 2) Configuration.ORIENTATION_PORTRAIT else Configuration.ORIENTATION_LANDSCAPE
                try {
                    val first = async(Dispatchers.Main) { plugin.invoke(1, payload(policy)) }
                    withContext(Dispatchers.Main) {
                        assertEquals(ScreenOrientationPlugin.decodePolicy(policy), activity.requestedOrientation)
                        try {
                            plugin.invoke(1, payload(policy))
                            fail("Overlapping lease accepted")
                        } catch (
                            error: PluginException,
                        ) {
                            assertEquals(PluginException.BUSY, error.code)
                        }
                    }
                    withTimeout(15000) {
                        while (withContext(Dispatchers.Main) { activity.resources.configuration.orientation } != expected) delay(10)
                    }
                    scenario.onActivity { assertSame("Orientation change recreated the native host", activity, it) }
                    withContext(Dispatchers.Main) {
                        val snapshot = ByteBuffer.wrap(plugin.invoke(0, ByteBuffer.allocate(0))).order(ByteOrder.LITTLE_ENDIAN)
                        assertEquals(if (policy == 2) 1 else 2, snapshot.int)
                    }
                    first.cancelAndJoin()
                    withContext(Dispatchers.Main) { assertEquals(original, activity.requestedOrientation) }
                    val second =
                        async(Dispatchers.Main) {
                            try {
                                plugin.invoke(1, payload(policy))
                                -1
                            } catch (error: PluginException) {
                                error.code
                            }
                        }
                    withContext(Dispatchers.Main) {
                        plugin.close()
                        assertEquals(original, activity.requestedOrientation)
                    }
                    assertEquals(PluginException.CANCELLED, withTimeout(5000) { second.await() })
                    withContext(Dispatchers.Main) {
                        plugin.close()
                        try {
                            plugin.invoke(0, ByteBuffer.allocate(0))
                            fail("Closed plugin accepted a snapshot")
                        } catch (error: PluginException) {
                            assertEquals(PluginException.CANCELLED, error.code)
                        }
                    }
                } finally {
                    withContext(Dispatchers.Main) {
                        plugin.close()
                        activity.requestedOrientation = original
                    }
                }
            }
        }

    @Test
    fun leaseStartsBehindADialogAndRestoresOnCancellation() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val intent = requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName))
            ActivityScenario.launch<Activity>(intent).use { scenario ->
                lateinit var activity: Activity
                scenario.onActivity { activity = it }
                withTimeout(5000) { while (!withContext(Dispatchers.Main) { activity.hasWindowFocus() }) delay(10) }
                val dialog = withContext(Dispatchers.Main) { Dialog(activity).apply { show() } }
                withTimeout(5000) { while (withContext(Dispatchers.Main) { activity.hasWindowFocus() }) delay(10) }
                val plugin = withContext(Dispatchers.Main) { ScreenOrientationPlugin(activity) }
                val original = withContext(Dispatchers.Main) { activity.requestedOrientation }
                try {
                    val lease = async(Dispatchers.Main) { plugin.invoke(1, payload(6)) }
                    withContext(Dispatchers.Main) {
                        assertEquals(ScreenOrientationPlugin.decodePolicy(6), activity.requestedOrientation)
                    }
                    lease.cancelAndJoin()
                    withContext(Dispatchers.Main) { assertEquals(original, activity.requestedOrientation) }
                } finally {
                    withContext(Dispatchers.Main) {
                        plugin.close()
                        dialog.dismiss()
                        activity.requestedOrientation = original
                    }
                }
            }
        }

    @Test
    fun policiesRejectUnknownValues() {
        assertEquals(ActivityInfo.SCREEN_ORIENTATION_REVERSE_PORTRAIT, ScreenOrientationPlugin.decodePolicy(3))
        assertEquals(ActivityInfo.SCREEN_ORIENTATION_FULL_SENSOR, ScreenOrientationPlugin.decodePolicy(6))
        try {
            ScreenOrientationPlugin.decodePolicy(7)
            fail("Unknown policy accepted")
        } catch (
            error: PluginException,
        ) {
            assertEquals(PluginException.INVALID_ARGUMENT, error.code)
        }
    }

    private fun payload(value: Int): ByteBuffer =
        ByteBuffer
            .allocate(4)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(value)
            .apply { flip() }
}
