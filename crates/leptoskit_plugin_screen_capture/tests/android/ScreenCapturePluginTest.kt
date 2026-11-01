package dev.leptoskit.plugins.screencapture

import android.Manifest
import android.app.Activity
import android.app.Dialog
import android.content.pm.PackageManager
import android.os.Build
import android.view.WindowManager
import androidx.lifecycle.Lifecycle
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.yield
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.nio.ByteBuffer

/** Only owned fixture windows are touched. These tests never take screenshots. */
class ScreenCapturePluginTest {
    /** An activity that reports the capture permission as missing. */
    private class DeniedCaptureActivity : Activity() {
        override fun checkSelfPermission(permission: String): Int =
            if (permission == Manifest.permission.DETECT_SCREEN_CAPTURE) {
                PackageManager.PERMISSION_DENIED
            } else {
                super.checkSelfPermission(permission)
            }
    }

    @Test
    fun sharedLeasesRestoreSecureBitWithoutClobberingUnrelatedFlags() =
        withScenario { activity ->
            val original = activity.window.attributes.flags
            val first = ScreenCapturePlugin(activity)
            val second = ScreenCapturePlugin(activity)

            try {
                activity.window.clearFlags(SECURE)
                val one = hold(first)
                val two = hold(second)

                yield()
                assertTrue(secure(activity))
                activity.window.addFlags(KEEP_SCREEN_ON)
                one.cancelAndJoin()

                assertTrue(secure(activity))
                two.cancelAndJoin()
                assertFalse(secure(activity))
                assertTrue(keptScreenOn(activity))
                activity.window.addFlags(SECURE)
                val existing = hold(first)

                yield()
                first.close()
                assertEquals(PluginException.CANCELLED, existing.await())
                assertTrue(secure(activity))
            } finally {
                first.close()
                second.close()
                activity.window.setFlags(original, ALL_FLAGS)
            }
        }

    @Test
    fun leasesRemainSecureWhileStoppedAndRestoreOnDestruction() =
        runBlocking {
            val scenario = launch()
            val activity = activity(scenario)
            awaitFocus(activity)
            val plugin = withContext(Dispatchers.Main) { ScreenCapturePlugin(activity) }
            val original =
                withContext(Dispatchers.Main) {
                    activity.window.attributes.flags
                        .also { activity.window.clearFlags(SECURE) }
                }

            try {
                val lease = hold(plugin)
                withContext(Dispatchers.Main) { assertTrue(secure(activity)) }
                scenario.moveToState(Lifecycle.State.CREATED)
                withContext(Dispatchers.Main) { assertTrue(secure(activity)) }
                assertFalse(lease.isCompleted)
                scenario.close()
                assertEquals(PluginException.CANCELLED, withTimeout(WAIT_TIMEOUT_MS) { lease.await() })
                withContext(Dispatchers.Main) { assertFalse(secure(activity)) }
            } finally {
                withContext(Dispatchers.Main) {
                    plugin.close()
                    activity.window.setFlags(original, ALL_FLAGS)
                }

                scenario.close()
            }
        }

    @Test
    fun protectionAndObservationContinueBehindADialog() =
        runBlocking {
            launch().use { scenario ->
                val activity = activity(scenario)
                awaitFocus(activity)
                val dialog = withContext(Dispatchers.Main) { Dialog(activity).apply { show() } }

                try {
                    awaitDialogFocusLoss(activity)
                    withContext(Dispatchers.Main) { protectionBehindDialog(activity) }
                } finally {
                    withContext(Dispatchers.Main) { dialog.dismiss() }
                }
            }
        }

    @Test
    fun boundedLeasesAndMalformedRequestsDoNotLeakProtection() =
        withScenario { activity ->
            val original = activity.window.attributes.flags
            val plugin = ScreenCapturePlugin(activity)

            try {
                activity.window.clearFlags(SECURE)
                assertMalformedRejected(plugin)
                assertFalse(secure(activity))
                assertCancelledBeforeLease(plugin)
                assertFalse(secure(activity))
                assertLeaseLimit(plugin)
                assertFalse(secure(activity))
            } finally {
                plugin.close()
                activity.window.setFlags(original, ALL_FLAGS)
            }
        }

    @Test
    fun screenshotWaitIsCancelledOnStopAndNeverReadsPhotos() =
        runBlocking {
            launch().use { scenario ->
                val activity = activity(scenario)
                awaitFocus(activity)
                val plugin = withContext(Dispatchers.Main) { ScreenCapturePlugin(activity) }

                try {
                    awaitCaptureEnd(plugin, scenario)
                } finally {
                    withContext(Dispatchers.Main) { plugin.close() }
                }
            }
        }

    @Test
    fun missingScreenshotPermissionFailsBeforeRegistration() =
        runBlocking {
            if (Build.VERSION.SDK_INT < DETECT_CAPTURE_API) return@runBlocking

            withContext(Dispatchers.Main) { assertMissingPermissionRefused() }
        }

    /** Proves protection and capture watching continue while a dialog holds focus. */
    private suspend fun CoroutineScope.protectionBehindDialog(activity: Activity) {
        val original = activity.window.attributes.flags
        val plugin = ScreenCapturePlugin(activity)

        try {
            activity.window.clearFlags(SECURE)
            val lease = hold(plugin)

            yield()
            assertTrue(secure(activity))
            assertFalse(lease.isCompleted)
            awaitScreenshot(plugin)
            lease.cancelAndJoin()
            assertFalse(secure(activity))
        } finally {
            plugin.close()

            activity.window.setFlags(original, ALL_FLAGS)
        }
    }

    /** Watches for a capture when the OS can report one, then cancels the watch. */
    private suspend fun CoroutineScope.awaitScreenshot(plugin: ScreenCapturePlugin) {
        if (Build.VERSION.SDK_INT < DETECT_CAPTURE_API) return

        val screenshot = async { plugin.invoke(SCREENSHOT, watch()) }

        yield()
        assertFalse(screenshot.isCompleted)
        screenshot.cancelAndJoin()
    }

    /** A capture watch fails when unsupported, and is cancelled when the activity stops. */
    private suspend fun CoroutineScope.awaitCaptureEnd(
        plugin: ScreenCapturePlugin,
        scenario: ActivityScenario<Activity>,
    ) {
        val pending = captureCode(plugin)
        withContext(Dispatchers.Main) {}

        if (Build.VERSION.SDK_INT < DETECT_CAPTURE_API) {
            assertEquals(PluginException.UNAVAILABLE, pending.await())

            return
        }

        scenario.moveToState(Lifecycle.State.CREATED)
        assertEquals(PluginException.CANCELLED, withTimeout(WAIT_TIMEOUT_MS) { pending.await() })
    }

    /** A denied capture permission fails before any observation is registered. */
    private suspend fun assertMissingPermissionRefused() {
        val plugin = ScreenCapturePlugin(DeniedCaptureActivity())

        try {
            plugin.invoke(SCREENSHOT, watch())
            fail("Missing manifest permission was accepted")
        } catch (error: PluginException) {
            assertEquals(PluginException.PERMISSION_DENIED, error.code)
        } finally {
            plugin.close()
        }
    }

    /** Requests the plugin answers as malformed before it touches a window. */
    private suspend fun assertMalformedRejected(plugin: ScreenCapturePlugin) {
        for ((method, bytes) in MALFORMED) {
            assertRejected(plugin, method, bytes)
        }
    }

    /** Expects one malformed request to be refused as an argument error. */
    private suspend fun assertRejected(
        plugin: ScreenCapturePlugin,
        method: Int,
        bytes: ByteArray,
    ) {
        try {
            plugin.invoke(method, ByteBuffer.wrap(bytes))
            fail("Invalid request accepted")
        } catch (error: PluginException) {
            assertEquals(PluginException.INVALID_ARGUMENT, error.code)
        }
    }

    /** A cancelled coroutine never installs a lease. */
    private suspend fun CoroutineScope.assertCancelledBeforeLease(plugin: ScreenCapturePlugin) {
        val cancelled =
            async(start = CoroutineStart.UNDISPATCHED) {
                currentCoroutineContext().cancel()
                plugin.invoke(PREVENT, ByteBuffer.allocate(0))
            }

        cancelled.cancelAndJoin()
    }

    /** More leases than the host allows report busy, and closing ends every one. */
    private suspend fun CoroutineScope.assertLeaseLimit(plugin: ScreenCapturePlugin) {
        val leases = List(MAX_LEASES) { hold(plugin) }

        yield()
        assertEquals(PluginException.BUSY, hold(plugin).await())
        plugin.close()
        leases.forEach { assertEquals(PluginException.CANCELLED, it.await()) }
    }

    /** Starts a protection lease and reports the code the wait ends with. */
    private fun CoroutineScope.hold(plugin: ScreenCapturePlugin): Deferred<Int> =
        async(Dispatchers.Main) {
            try {
                plugin.invoke(PREVENT, ByteBuffer.allocate(0))
                RETURNED
            } catch (error: PluginException) {
                error.code
            }
        }

    /** Starts a capture watch and reports the code the wait ends with. */
    private fun CoroutineScope.captureCode(plugin: ScreenCapturePlugin): Deferred<Int> =
        async(Dispatchers.Main) {
            try {
                plugin.invoke(SCREENSHOT, watch())
                RETURNED
            } catch (error: PluginException) {
                error.code
            }
        }

    /** Runs the body on the UI thread against a freshly launched activity. */
    private fun withScenario(body: suspend CoroutineScope.(Activity) -> Unit) =
        runBlocking {
            launch().use { scenario ->
                val activity = activity(scenario)
                awaitFocus(activity)
                withContext(Dispatchers.Main) { body(activity) }
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

    /** Waits until the launched activity has window focus. */
    private suspend fun awaitFocus(activity: Activity) {
        withTimeout(WAIT_TIMEOUT_MS) {
            while (!withContext(Dispatchers.Main) { activity.hasWindowFocus() }) delay(FOCUS_POLL_MS)
        }
    }

    /** Waits for a dialog to take focus from the activity. */
    private suspend fun awaitDialogFocusLoss(activity: Activity) {
        withTimeout(WAIT_TIMEOUT_MS) {
            while (withContext(Dispatchers.Main) { activity.hasWindowFocus() }) delay(FOCUS_POLL_MS)
        }
    }

    /** Whether the window carries the secure bit. */
    private fun secure(activity: Activity): Boolean = activity.window.attributes.flags and SECURE != 0

    /** Whether the window still carries an unrelated flag the test set. */
    private fun keptScreenOn(activity: Activity): Boolean = activity.window.attributes.flags and KEEP_SCREEN_ON != 0

    /** A watch that observes the next capture. */
    private fun watch(): ByteBuffer = ByteBuffer.wrap(fields(1, 0))

    private companion object {
        /** Window flags the plugin installs and must leave alone. */
        val SECURE = WindowManager.LayoutParams.FLAG_SECURE
        val KEEP_SCREEN_ON = WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON

        /** Restores every flag bit. */
        const val ALL_FLAGS = -1

        /** Methods the plugin answers. */
        const val SCREENSHOT = 2
        const val PREVENT = 3

        /** The first API level that reports captures. */
        const val DETECT_CAPTURE_API = 34

        /** How many windows may hold protection at once. */
        const val MAX_LEASES = 64

        /** How long a stopping scenario may take to cancel its wait. */
        const val WAIT_TIMEOUT_MS = 5000L

        /** How often the focus waits re-check the window. */
        const val FOCUS_POLL_MS = 10L

        /** Returned when a wait answered instead of failing. */
        const val RETURNED = -1

        /** Requests the plugin must reject before it touches a window. */
        val MALFORMED = listOf(3 to byteArrayOf(1), 2 to fields(1, 1), 2 to fields(2, 0))
    }
}
