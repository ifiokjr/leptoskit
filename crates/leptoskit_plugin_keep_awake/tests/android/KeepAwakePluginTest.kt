package dev.leptoskit.plugins.keepawake

import android.app.Activity
import android.view.WindowManager
import androidx.lifecycle.Lifecycle
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Test
import java.nio.ByteBuffer

/** Reads the actual Activity window flag through acquire/cancel/stop/close. */
class KeepAwakePluginTest {
    @Test
    fun independentCancellationAndCloseReleaseOnlyOwnedReferences() =
        runBlocking {
            launch().use { scenario ->
                val activity = activity(scenario)
                awaitFocus(activity)
                val baseline = withContext(Dispatchers.Main) { enabled(activity) }
                val plugin =
                    withContext(Dispatchers.Main) {
                        setFlag(activity, false)
                        KeepAwakePlugin(activity)
                    }
                try {
                    val first = hold(plugin)
                    awaitFlag(activity, true)
                    val second = hold(plugin)
                    // FIFO Main dispatch ensures the second invocation has started.
                    withContext(Dispatchers.Main) { assertEquals(true, enabled(activity)) }
                    first.cancelAndJoin()
                    withContext(Dispatchers.Main) { assertEquals(true, enabled(activity)) }
                    second.cancelAndJoin()
                    awaitFlag(activity, false)
                    val third = hold(plugin)
                    awaitFlag(activity, true)
                    withContext(Dispatchers.Main) { plugin.close() }
                    assertEquals(PluginException.CANCELLED, withTimeout(5000) { third.await() })
                    awaitFlag(activity, false)
                    withContext(Dispatchers.Main) {
                        plugin.close()
                        try {
                            plugin.invoke(0, ByteBuffer.allocate(0))
                            fail("Closed plugin accepted another lease")
                        } catch (error: PluginException) {
                            assertEquals(PluginException.CANCELLED, error.code)
                        }
                    }
                } finally {
                    withContext(Dispatchers.Main) {
                        plugin.close()
                        setFlag(activity, baseline)
                    }
                }
            }
        }

    @Test
    fun backgroundStopsTheLeaseAndForegroundCanExplicitlyReacquire() =
        runBlocking {
            launch().use { scenario ->
                val activity = activity(scenario)
                awaitFocus(activity)
                val baseline = withContext(Dispatchers.Main) { enabled(activity) }
                val plugin =
                    withContext(Dispatchers.Main) {
                        setFlag(activity, false)
                        KeepAwakePlugin(activity)
                    }
                try {
                    val first = hold(plugin)
                    awaitFlag(activity, true)
                    scenario.moveToState(Lifecycle.State.CREATED)
                    assertEquals(PluginException.CANCELLED, withTimeout(5000) { first.await() })
                    awaitFlag(activity, false)
                    scenario.moveToState(Lifecycle.State.RESUMED)
                    awaitFocus(activity)
                    val second = hold(plugin)
                    awaitFlag(activity, true)
                    second.cancelAndJoin()
                    awaitFlag(activity, false)
                } finally {
                    withContext(Dispatchers.Main) {
                        plugin.close()
                        setFlag(activity, baseline)
                    }
                }
            }
        }

    @Test
    fun preexistingKeepScreenOnFlagSurvivesPluginClose() =
        runBlocking {
            launch().use { scenario ->
                val activity = activity(scenario)
                awaitFocus(activity)
                val baseline = withContext(Dispatchers.Main) { enabled(activity) }
                val plugin =
                    withContext(Dispatchers.Main) {
                        setFlag(activity, true)
                        KeepAwakePlugin(activity)
                    }
                try {
                    val lease = hold(plugin)
                    withContext(Dispatchers.Main) {
                        assertEquals(true, enabled(activity))
                        plugin.close()
                    }
                    assertEquals(PluginException.CANCELLED, withTimeout(5000) { lease.await() })
                    withContext(Dispatchers.Main) { assertEquals(true, enabled(activity)) }
                } finally {
                    withContext(Dispatchers.Main) {
                        plugin.close()
                        setFlag(activity, baseline)
                    }
                }
            }
        }

    private fun CoroutineScope.hold(plugin: KeepAwakePlugin): Deferred<Int> =
        async(Dispatchers.Main) {
            try {
                plugin.invoke(0, ByteBuffer.allocate(0))
                error("A holding call cannot complete successfully")
            } catch (error: PluginException) {
                error.code
            }
        }

    private fun launch(): ActivityScenario<Activity> {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        return ActivityScenario.launch(requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName)))
    }

    private fun activity(scenario: ActivityScenario<Activity>): Activity {
        lateinit var activity: Activity
        scenario.onActivity { activity = it }
        return activity
    }

    private suspend fun awaitFocus(activity: Activity) =
        withTimeout(5000) {
            while (!withContext(Dispatchers.Main) { activity.hasWindowFocus() }) delay(10)
        }

    private suspend fun awaitFlag(
        activity: Activity,
        expected: Boolean,
    ) = withTimeout(5000) {
        while (withContext(Dispatchers.Main) { enabled(activity) } != expected) delay(10)
    }

    private fun enabled(activity: Activity): Boolean =
        activity.window.attributes.flags and WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON != 0

    private fun setFlag(
        activity: Activity,
        value: Boolean,
    ) {
        if (value) {
            activity.window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        } else {
            activity.window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        }
    }
}
