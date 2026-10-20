package dev.leptoskit.plugins.screencapture

import android.Manifest
import android.app.Activity
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
    @Test
    fun sharedLeasesRestoreSecureBitWithoutClobberingUnrelatedFlags() =
        runBlocking {
            launch().use { scenario ->
                val activity = activity(scenario)
                awaitFocus(activity)
                withContext(Dispatchers.Main) {
                    val original = activity.window.attributes.flags
                    val first = ScreenCapturePlugin(activity)
                    val second = ScreenCapturePlugin(activity)
                    try {
                        activity.window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
                        val one = hold(first)
                        val two = hold(second)
                        yield()
                        assertTrue(secure(activity))
                        activity.window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                        one.cancelAndJoin()
                        assertTrue(secure(activity))
                        two.cancelAndJoin()
                        assertFalse(secure(activity))
                        assertTrue(activity.window.attributes.flags and WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON != 0)
                        activity.window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
                        val existing = hold(first)
                        yield()
                        first.close()
                        assertEquals(PluginException.CANCELLED, existing.await())
                        assertTrue(secure(activity))
                    } finally {
                        first.close()
                        second.close()
                        activity.window.setFlags(original, -1)
                    }
                }
            }
        }

    @Test
    fun leasesRemainSecureWhileStoppedAndRestoreOnDestruction() =
        runBlocking {
            val scenario = launch()
            val activity = activity(scenario)
            awaitFocus(activity)
            lateinit var plugin: ScreenCapturePlugin
            val original =
                withContext(Dispatchers.Main) {
                    plugin = ScreenCapturePlugin(activity)
                    activity.window.attributes.flags
                        .also { activity.window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE) }
                }
            try {
                val lease = hold(plugin)
                withContext(Dispatchers.Main) { assertTrue(secure(activity)) }
                scenario.moveToState(Lifecycle.State.CREATED)
                withContext(Dispatchers.Main) { assertTrue(secure(activity)) }
                assertFalse(lease.isCompleted)
                scenario.close()
                assertEquals(PluginException.CANCELLED, withTimeout(5000) { lease.await() })
                withContext(Dispatchers.Main) { assertFalse(secure(activity)) }
            } finally {
                withContext(Dispatchers.Main) {
                    plugin.close()
                    activity.window.setFlags(original, -1)
                }
                scenario.close()
            }
        }

    @Test
    fun boundedLeasesAndMalformedRequestsDoNotLeakProtection() =
        runBlocking {
            launch().use { scenario ->
                val activity = activity(scenario)
                awaitFocus(activity)
                withContext(Dispatchers.Main) {
                    val original = activity.window.attributes.flags
                    val plugin = ScreenCapturePlugin(activity)
                    try {
                        activity.window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
                        for ((method, bytes) in listOf(3 to byteArrayOf(1), 2 to fields(1, 1), 2 to fields(2, 0))) {
                            try {
                                plugin.invoke(method, ByteBuffer.wrap(bytes))
                                fail("Invalid request accepted")
                            } catch (error: PluginException) {
                                assertEquals(PluginException.INVALID_ARGUMENT, error.code)
                            }
                        }
                        assertFalse(secure(activity))
                        val cancelled =
                            async(start = CoroutineStart.UNDISPATCHED) {
                                currentCoroutineContext().cancel()
                                plugin.invoke(3, ByteBuffer.allocate(0))
                            }
                        cancelled.cancelAndJoin()
                        assertFalse(secure(activity))
                        val leases = List(64) { hold(plugin) }
                        yield()
                        assertEquals(PluginException.BUSY, hold(plugin).await())
                        plugin.close()
                        leases.forEach { assertEquals(PluginException.CANCELLED, it.await()) }
                        assertFalse(secure(activity))
                    } finally {
                        plugin.close()
                        activity.window.setFlags(original, -1)
                    }
                }
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
                    val pending =
                        async(Dispatchers.Main) {
                            try {
                                plugin.invoke(2, ByteBuffer.wrap(fields(1, 0)))
                                -1
                            } catch (error: PluginException) {
                                error.code
                            }
                        }
                    withContext(Dispatchers.Main) {}
                    if (Build.VERSION.SDK_INT < 34) {
                        assertEquals(PluginException.UNAVAILABLE, pending.await())
                    } else {
                        scenario.moveToState(Lifecycle.State.CREATED)
                        assertEquals(PluginException.CANCELLED, withTimeout(5000) { pending.await() })
                    }
                } finally {
                    withContext(Dispatchers.Main) { plugin.close() }
                }
            }
        }

    @Test
    fun missingScreenshotPermissionFailsBeforeRegistration() =
        runBlocking {
            if (Build.VERSION.SDK_INT < 34) return@runBlocking
            withContext(Dispatchers.Main) {
                val plugin =
                    ScreenCapturePlugin(
                        object : Activity() {
                            override fun checkSelfPermission(permission: String): Int =
                                if (permission ==
                                    Manifest.permission.DETECT_SCREEN_CAPTURE
                                ) {
                                    PackageManager.PERMISSION_DENIED
                                } else {
                                    super.checkSelfPermission(permission)
                                }
                        },
                    )
                try {
                    plugin.invoke(2, ByteBuffer.wrap(fields(1, 0)))
                    fail("Missing manifest permission was accepted")
                } catch (error: PluginException) {
                    assertEquals(PluginException.PERMISSION_DENIED, error.code)
                } finally {
                    plugin.close()
                }
            }
        }

    private fun CoroutineScope.hold(plugin: ScreenCapturePlugin): Deferred<Int> =
        async(Dispatchers.Main) {
            try {
                plugin.invoke(3, ByteBuffer.allocate(0))
                error("Prevention lease cannot succeed")
            } catch (
                error: PluginException,
            ) {
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

    private fun secure(activity: Activity): Boolean = activity.window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE != 0
}
