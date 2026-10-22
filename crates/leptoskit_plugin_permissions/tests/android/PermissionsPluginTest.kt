package dev.leptoskit.plugins.permissions

import android.app.Activity
import android.view.accessibility.AccessibilityNodeInfo
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import dev.leptoskit.bridge.NativePermissionBroker
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.fail
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.regex.Pattern

/** Run in a generated test app with the permissions plugin selected in native.plugins. */
class PermissionsPluginTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val device = UiDevice.getInstance(instrumentation)

    @Test
    fun cameraGrantReturnsThroughActivityResultBroker() =
        runBlocking {
            val intent = requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName))
            ActivityScenario.launch<Activity>(intent).use { scenario ->
                lateinit var activity: Activity
                scenario.onActivity { activity = it }
                val plugin = withContext(Dispatchers.Main) { PermissionsPlugin(activity) }
                val initial = withContext(Dispatchers.Main) { plugin.invoke(0, camera()) }
                val result = async(Dispatchers.Main) { plugin.invoke(1, camera()) }
                if (ByteBuffer.wrap(initial).order(ByteOrder.LITTLE_ENDIAN).int != 3) {
                    answerPrompt(".*:id/permission_allow(_foreground_only|_one_time)?_button", "camera")
                }
                assertArrayEquals(byteArrayOf(3, 0, 0, 0, 1, 0, 0, 0), withTimeout(10_000) { result.await() })
                assertArrayEquals(byteArrayOf(3, 0, 0, 0, 1, 0, 0, 0), withContext(Dispatchers.Main) { plugin.invoke(0, camera()) })
                withContext(Dispatchers.Main) { plugin.close() }
                try {
                    withContext(Dispatchers.Main) { plugin.invoke(1, camera()) }
                    fail("Closed handlers cannot request permissions")
                } catch (error: PluginException) {
                    assertEquals(PluginException.CANCELLED, error.code)
                }
            }
        }

    @Test
    fun microphoneDenialIsReportedAndCanBeAskedAgain() =
        runBlocking {
            val intent = requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName))
            ActivityScenario.launch<Activity>(intent).use { scenario ->
                lateinit var activity: Activity
                scenario.onActivity { activity = it }
                val plugin = withContext(Dispatchers.Main) { PermissionsPlugin(activity) }
                val initial = withContext(Dispatchers.Main) { plugin.invoke(0, microphone()) }
                if (ByteBuffer.wrap(initial).order(ByteOrder.LITTLE_ENDIAN).int == 3) {
                    // Revoking a runtime permission kills the instrumented process, so a
                    // re-run on a device that already granted it verifies the grant instead.
                    assertArrayEquals(byteArrayOf(3, 0, 0, 0, 1, 0, 0, 0), initial)
                } else {
                    val result = async(Dispatchers.Main) { plugin.invoke(1, microphone()) }
                    answerPrompt(".*:id/permission_deny(_and_dont_ask_again)?_button", "microphone")
                    // Denied, and Android still allows asking again after a first refusal.
                    assertArrayEquals(byteArrayOf(2, 0, 0, 0, 2, 0, 0, 0), withTimeout(10_000) { result.await() })
                    assertArrayEquals(byteArrayOf(2, 0, 0, 0, 2, 0, 0, 0), withContext(Dispatchers.Main) { plugin.invoke(0, microphone()) })
                }
                withContext(Dispatchers.Main) { plugin.close() }
            }
        }

    @Test
    fun brokerRejectsInvalidRequestsAndIgnoresUnrelatedResults() =
        runBlocking {
            val intent = requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName))
            ActivityScenario.launch<Activity>(intent).use { scenario ->
                lateinit var activity: Activity
                scenario.onActivity { activity = it }
                withContext(Dispatchers.Main) {
                    val broker = NativePermissionBroker(activity)
                    assertEquals(false, broker.onResult(123, arrayOf("other"), intArrayOf(0)))
                    try {
                        broker.request(emptyArray())
                        fail("Empty requests must not open an OS prompt")
                    } catch (error: PluginException) {
                        assertEquals(PluginException.INVALID_ARGUMENT, error.code)
                    }
                    broker.close()
                    try {
                        broker.request(arrayOf("android.permission.CAMERA"))
                        fail("Closed brokers must reject new work")
                    } catch (error: PluginException) {
                        assertEquals(PluginException.CANCELLED, error.code)
                    }
                }
            }
        }

    /**
     * Presses a system permission button until Android dismisses its prompt. The press is an
     * accessibility click on the button's own node: a coordinate tap is delivered to whichever
     * window is on top at that point, which was the app rather than the prompt on CI. A coordinate
     * tap remains the fallback, and the button must disappear before the plugin's result is awaited.
     */
    private fun answerPrompt(
        button: String,
        permission: String,
    ) {
        val pattern = Pattern.compile(button)
        val selector = By.res(pattern)
        assertNotNull("Expected Android's $permission permission prompt", device.wait(Until.findObject(selector), 10_000))
        repeat(5) {
            if (!clickNode(pattern)) device.findObject(selector)?.click() ?: return
            if (device.wait(Until.gone(selector), 2_000) == true) return
        }
        fail("Android's $permission permission prompt ignored every press")
    }

    /** Performs an accessibility click on the first visible node whose resource ID matches. */
    private fun clickNode(pattern: Pattern): Boolean {
        for (window in instrumentation.uiAutomation.windows) {
            val node = window.root?.let { find(it, pattern) } ?: continue
            return node.performAction(AccessibilityNodeInfo.ACTION_CLICK)
        }
        return false
    }

    private fun find(
        node: AccessibilityNodeInfo,
        pattern: Pattern,
    ): AccessibilityNodeInfo? {
        if (node.isVisibleToUser && node.viewIdResourceName?.let { pattern.matcher(it).matches() } == true) return node
        for (index in 0 until node.childCount) {
            node.getChild(index)?.let { child -> find(child, pattern)?.let { return it } }
        }
        return null
    }

    private fun camera(): ByteBuffer = ByteBuffer.wrap(byteArrayOf(0, 0, 0, 0))

    private fun microphone(): ByteBuffer = ByteBuffer.wrap(byteArrayOf(1, 0, 0, 0))
}
