package dev.leptoskit.plugins.permissions

import android.app.Activity
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
     * Taps a system permission button until Android dismisses its prompt. A tap that lands while
     * the prompt is still animating in is dropped, which leaves the request unanswered, so check
     * that the button is gone before waiting for the plugin's result.
     */
    private fun answerPrompt(
        button: String,
        permission: String,
    ) {
        val selector = By.res(Pattern.compile(button))
        assertNotNull("Expected Android's $permission permission prompt", device.wait(Until.findObject(selector), 10_000))
        repeat(5) {
            val target = device.findObject(selector) ?: return
            target.click()
            if (device.wait(Until.gone(selector), 2_000) == true) return
        }
        fail("Android's $permission permission prompt ignored every tap")
    }

    private fun camera(): ByteBuffer = ByteBuffer.wrap(byteArrayOf(0, 0, 0, 0))

    private fun microphone(): ByteBuffer = ByteBuffer.wrap(byteArrayOf(1, 0, 0, 0))
}
