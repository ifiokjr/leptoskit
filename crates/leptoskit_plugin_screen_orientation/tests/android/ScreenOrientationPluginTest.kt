package dev.leptoskit.plugins.screenorientation

import android.app.Activity
import android.app.Dialog
import android.content.pm.ActivityInfo
import android.content.res.Configuration
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import dev.leptoskit.plugins.screenorientation.ScreenOrientationPlugin.Companion.HOLD
import dev.leptoskit.plugins.screenorientation.ScreenOrientationPlugin.Companion.ORIENTATION_LANDSCAPE
import dev.leptoskit.plugins.screenorientation.ScreenOrientationPlugin.Companion.ORIENTATION_PORTRAIT
import dev.leptoskit.plugins.screenorientation.ScreenOrientationPlugin.Companion.POLICY_FULL_SENSOR
import dev.leptoskit.plugins.screenorientation.ScreenOrientationPlugin.Companion.POLICY_LANDSCAPE
import dev.leptoskit.plugins.screenorientation.ScreenOrientationPlugin.Companion.POLICY_PORTRAIT
import dev.leptoskit.plugins.screenorientation.ScreenOrientationPlugin.Companion.POLICY_REVERSE_PORTRAIT
import dev.leptoskit.plugins.screenorientation.ScreenOrientationPlugin.Companion.SNAPSHOT
import kotlinx.coroutines.CoroutineScope
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

/** Runs assertions against the launched host activity and its scenario. */
private typealias ActivityBody = suspend CoroutineScope.(ActivityScenario<Activity>, Activity) -> Unit

class ScreenOrientationPluginTest {
    @Test
    fun cancellationAndCloseRestoreTheExactActivityPolicy() =
        runBlocking {
            withActivity { scenario, activity -> checkCancellationAndClose(scenario, activity) }
        }

    @Test
    fun leaseStartsBehindADialogAndRestoresOnCancellation() =
        runBlocking {
            withActivity { _, activity -> checkLeaseBehindDialog(activity) }
        }

    @Test
    fun policiesRejectUnknownValues() {
        assertEquals(
            ActivityInfo.SCREEN_ORIENTATION_REVERSE_PORTRAIT,
            ScreenOrientationPlugin.decodePolicy(POLICY_REVERSE_PORTRAIT),
        )
        assertEquals(
            ActivityInfo.SCREEN_ORIENTATION_FULL_SENSOR,
            ScreenOrientationPlugin.decodePolicy(POLICY_FULL_SENSOR),
        )

        try {
            ScreenOrientationPlugin.decodePolicy(UNKNOWN_POLICY)

            fail("Unknown policy accepted")
        } catch (error: PluginException) {
            assertEquals(PluginException.INVALID_ARGUMENT, error.code)
        }
    }

    /** A cancelled lease and a closed plugin both put the original policy back. */
    private suspend fun CoroutineScope.checkCancellationAndClose(
        scenario: ActivityScenario<Activity>,
        activity: Activity,
    ) {
        val plugin = ScreenOrientationPlugin(activity)
        val original = activity.requestedOrientation
        val policy = oppositePolicy(activity)
        val expected = if (policy == POLICY_PORTRAIT) Configuration.ORIENTATION_PORTRAIT else Configuration.ORIENTATION_LANDSCAPE

        try {
            val first = async(Dispatchers.Main) { plugin.invoke(HOLD, payload(policy)) }

            withContext(Dispatchers.Main) {
                assertEquals(ScreenOrientationPlugin.decodePolicy(policy), activity.requestedOrientation)
                expectError(PluginException.BUSY) { plugin.invoke(HOLD, payload(policy)) }
            }

            awaitOrientation(activity, expected)
            scenario.onActivity { assertSame("Orientation change recreated the native host", activity, it) }

            val expectedOrientation = if (policy == POLICY_PORTRAIT) ORIENTATION_PORTRAIT else ORIENTATION_LANDSCAPE

            withContext(Dispatchers.Main) { assertEquals(expectedOrientation, snapshotOrientation(plugin)) }

            first.cancelAndJoin()

            withContext(Dispatchers.Main) { assertEquals(original, activity.requestedOrientation) }

            val second = async(Dispatchers.Main) { holdOutcome(plugin, policy) }

            withContext(Dispatchers.Main) {
                plugin.close()
                assertEquals(original, activity.requestedOrientation)
            }

            assertEquals(PluginException.CANCELLED, withTimeout(RELEASE_TIMEOUT_MS) { second.await() })

            withContext(Dispatchers.Main) {
                plugin.close()
                expectError(PluginException.CANCELLED) { plugin.invoke(SNAPSHOT, ByteBuffer.allocate(0)) }
            }
        } finally {
            withContext(Dispatchers.Main) {
                plugin.close()
                activity.requestedOrientation = original
            }
        }
    }

    /** A dialog steals focus, and the lease still starts and restores. */
    private suspend fun CoroutineScope.checkLeaseBehindDialog(activity: Activity) {
        val dialog = withContext(Dispatchers.Main) { Dialog(activity).apply { show() } }

        awaitWindowFocus(activity, focused = false)

        val plugin = ScreenOrientationPlugin(activity)
        val original = activity.requestedOrientation

        try {
            val lease = async(Dispatchers.Main) { plugin.invoke(HOLD, payload(POLICY_FULL_SENSOR)) }

            withContext(Dispatchers.Main) {
                assertEquals(
                    ScreenOrientationPlugin.decodePolicy(POLICY_FULL_SENSOR),
                    activity.requestedOrientation,
                )
            }

            lease.cancelAndJoin()
            // The plugin restores the orientation from a handler post on the main
            // thread, which cancelAndJoin does not wait for; poll like every other
            // wait in this test.
            awaitOrientationRestored(activity, original)
        } finally {
            withContext(Dispatchers.Main) {
                plugin.close()
                dialog.dismiss()
                activity.requestedOrientation = original
            }
        }
    }

    /** Launches the generated host, waits for its window and runs the body. */
    private suspend fun CoroutineScope.withActivity(body: ActivityBody) {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val intent = requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName))

        ActivityScenario.launch<Activity>(intent).use { scenario ->
            lateinit var activity: Activity

            scenario.onActivity { activity = it }
            awaitWindowFocus(activity, focused = true)
            body(scenario, activity)
        }
    }

    /** The landscape policy opposite the activity's current configuration. */
    private suspend fun oppositePolicy(activity: Activity): Int =
        withContext(Dispatchers.Main) {
            if (activity.resources.configuration.orientation == Configuration.ORIENTATION_LANDSCAPE) {
                POLICY_PORTRAIT
            } else {
                POLICY_LANDSCAPE
            }
        }

    /** The orientation code the plugin currently reports. */
    private suspend fun snapshotOrientation(plugin: ScreenOrientationPlugin): Int =
        ByteBuffer
            .wrap(plugin.invoke(SNAPSHOT, ByteBuffer.allocate(0)))
            .order(ByteOrder.LITTLE_ENDIAN)
            .int

    /** The error code a hold reported, or the sentinel when it returned instead. */
    private suspend fun holdOutcome(
        plugin: ScreenOrientationPlugin,
        policy: Int,
    ): Int =
        try {
            plugin.invoke(HOLD, payload(policy))

            NO_ERROR_CODE
        } catch (error: PluginException) {
            error.code
        }

    /** Waits for the activity to gain or lose window focus. */
    private suspend fun awaitWindowFocus(
        activity: Activity,
        focused: Boolean,
    ) {
        withTimeout(FOCUS_TIMEOUT_MS) {
            while (withContext(Dispatchers.Main) { activity.hasWindowFocus() } != focused) delay(POLL_INTERVAL_MS)
        }
    }

    /** Waits for the configuration to report the requested orientation. */
    private suspend fun awaitOrientation(
        activity: Activity,
        expected: Int,
    ) {
        withTimeout(ORIENTATION_TIMEOUT_MS) {
            while (withContext(Dispatchers.Main) { activity.resources.configuration.orientation } != expected) {
                delay(POLL_INTERVAL_MS)
            }
        }
    }

    /** Waits for the plugin to put the activity's policy back. */
    private suspend fun awaitOrientationRestored(
        activity: Activity,
        original: Int,
    ) {
        withTimeout(RESTORE_TIMEOUT_MS) {
            while (withContext(Dispatchers.Main) { activity.requestedOrientation } != original) delay(POLL_INTERVAL_MS)
        }

        withContext(Dispatchers.Main) { assertEquals(original, activity.requestedOrientation) }
    }

    private suspend fun expectError(
        code: Int,
        action: suspend () -> ByteArray,
    ) {
        try {
            action()

            fail("Expected plugin error $code")
        } catch (error: PluginException) {
            assertEquals(code, error.code)
        }
    }

    private fun payload(value: Int): ByteBuffer =
        ByteBuffer
            .allocate(POLICY_BYTES)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(value)
            .apply { flip() }

    private companion object {
        /** The wire shape of a hold request. */
        const val POLICY_BYTES = 4

        /** A policy id no build defines. */
        const val UNKNOWN_POLICY = 7

        /** The hold outcome that means the call returned instead of failing. */
        const val NO_ERROR_CODE = -1

        /** Timeouts and the poll interval for asynchronous window changes. */
        const val FOCUS_TIMEOUT_MS = 5_000L
        const val ORIENTATION_TIMEOUT_MS = 15_000L
        const val RESTORE_TIMEOUT_MS = 5_000L
        const val RELEASE_TIMEOUT_MS = 5_000L
        const val POLL_INTERVAL_MS = 10L
    }
}
