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
import kotlinx.coroutines.CoroutineScope
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
        withPlugin { plugin ->
            val initial = withContext(Dispatchers.Main) { plugin.invoke(QUERY, camera()) }
            val result = async(Dispatchers.Main) { plugin.invoke(REQUEST, camera()) }

            if (statusOf(initial) != GRANTED) {
                answerPrompt(".*:id/permission_allow(_foreground_only|_one_time)?_button", "camera")
            }

            assertArrayEquals(GRANTED_RESPONSE, withTimeout(PROMPT_TIMEOUT_MS) { result.await() })
            assertArrayEquals(GRANTED_RESPONSE, withContext(Dispatchers.Main) { plugin.invoke(QUERY, camera()) })
            withContext(Dispatchers.Main) { plugin.close() }
            assertCancelled { plugin.invoke(REQUEST, camera()) }
        }

    @Test
    fun microphoneDenialIsReportedAndCanBeAskedAgain() =
        withPlugin { plugin ->
            val initial = withContext(Dispatchers.Main) { plugin.invoke(QUERY, microphone()) }

            // Revoking a runtime permission kills the instrumented process, so a
            // re-run on a device that already granted it verifies the grant instead.
            if (statusOf(initial) == GRANTED) {
                assertArrayEquals(GRANTED_RESPONSE, initial)
            } else {
                assertDenialIsAskableAgain(plugin)
            }

            withContext(Dispatchers.Main) { plugin.close() }
        }

    @Test
    fun brokerRejectsInvalidRequestsAndIgnoresUnrelatedResults() =
        withActivity { activity ->
            withContext(Dispatchers.Main) {
                val broker = NativePermissionBroker(activity)

                assertEquals(false, broker.onResult(UNRELATED_CODE, arrayOf("other"), intArrayOf(0)))
                assertRejected("Empty requests must not open an OS prompt", PluginException.INVALID_ARGUMENT) {
                    broker.request(emptyArray())
                }
                broker.close()

                assertRejected("Closed brokers must reject new work", PluginException.CANCELLED) {
                    broker.request(arrayOf("android.permission.CAMERA"))
                }
            }
        }

    /** Denies the prompt and confirms Android still allows asking again. */
    private suspend fun CoroutineScope.assertDenialIsAskableAgain(plugin: PermissionsPlugin) {
        val result = async(Dispatchers.Main) { plugin.invoke(REQUEST, microphone()) }
        answerPrompt(".*:id/permission_deny(_and_dont_ask_again)?_button", "microphone")

        // Denied, and Android still allows asking again after a first refusal.
        assertArrayEquals(DENIED_ASKABLE_RESPONSE, withTimeout(PROMPT_TIMEOUT_MS) { result.await() })
        assertArrayEquals(DENIED_ASKABLE_RESPONSE, withContext(Dispatchers.Main) { plugin.invoke(QUERY, microphone()) })
    }

    /** Expects the plugin to reject the call with the cancelled code. */
    private suspend fun assertCancelled(invoke: suspend () -> ByteArray) {
        try {
            invoke()
            fail("Closed handlers cannot request permissions")
        } catch (error: PluginException) {
            assertEquals(PluginException.CANCELLED, error.code)
        }
    }

    /** Expects the broker to reject the call with the given code. */
    private fun assertRejected(
        message: String,
        code: Int,
        call: () -> Unit,
    ) {
        try {
            call()
            fail(message)
        } catch (error: PluginException) {
            assertEquals(code, error.code)
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
        val prompt = device.wait(Until.findObject(selector), PROMPT_TIMEOUT_MS)
        assertNotNull("Expected Android's $permission permission prompt", prompt)

        repeat(PROMPT_PRESSES) {
            if (!clickNode(pattern)) device.findObject(selector)?.click() ?: return
            if (device.wait(Until.gone(selector), PROMPT_DISMISS_MS) == true) return
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

    /** Launches the test app and runs the body against its activity. */
    private fun withActivity(body: suspend CoroutineScope.(Activity) -> Unit) =
        runBlocking {
            val intent = requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName))
            ActivityScenario.launch<Activity>(intent).use { scenario ->
                lateinit var activity: Activity
                scenario.onActivity { activity = it }
                body(activity)
            }
        }

    /** Launches the test app and runs the body against a plugin bound to its activity. */
    private fun withPlugin(body: suspend CoroutineScope.(PermissionsPlugin) -> Unit) =
        withActivity { activity ->
            body(withContext(Dispatchers.Main) { PermissionsPlugin(activity) })
        }

    private fun statusOf(encoded: ByteArray): Int = ByteBuffer.wrap(encoded).order(ByteOrder.LITTLE_ENDIAN).int

    private fun camera(): ByteBuffer = ByteBuffer.wrap(byteArrayOf(0, 0, 0, 0))

    private fun microphone(): ByteBuffer = ByteBuffer.wrap(byteArrayOf(1, 0, 0, 0))

    private companion object {
        /** The wire methods the plugin answers. */
        const val QUERY = 0
        const val REQUEST = 1

        /** How long the OS prompt and the plugin answer may each take. */
        const val PROMPT_TIMEOUT_MS = 10_000L

        /** How long a pressed prompt button gets to dismiss itself. */
        const val PROMPT_DISMISS_MS = 2_000L

        /** How many presses a prompt gets before the test gives up. */
        const val PROMPT_PRESSES = 5

        /** A request code the broker never handed out. */
        const val UNRELATED_CODE = 123

        /** Status `3` is granted; the request result also reports it can no longer prompt. */
        const val GRANTED = 3

        val GRANTED_RESPONSE = byteArrayOf(3, 0, 0, 0, 1, 0, 0, 0)
        val DENIED_ASKABLE_RESPONSE = byteArrayOf(2, 0, 0, 0, 2, 0, 0, 0)
    }
}
