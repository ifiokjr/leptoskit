package dev.leptoskit.plugins.devicepolicy

import android.app.Activity
import android.content.Intent
import androidx.test.core.app.ActivityScenario
import dev.leptoskit.bridge.NativeActivityResult
import dev.leptoskit.bridge.NativeActivityResultHost
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** No test activates device admin, locks the device, or wipes data. */
class DevicePolicyPluginTest {
    @Test
    fun statusWithoutDeclaredReceiverIsUnavailableAndNeverPrompts() =
        withPlugin { plugin ->
            val status = ByteBuffer.wrap(plugin.invoke(STATUS, empty())).order(ByteOrder.LITTLE_ENDIAN)
            assertEquals(1, status.getInt(0))
            assertEquals(STATE_UNAVAILABLE, status.getInt(4))
            assertEquals(CAPABILITIES_NONE, status.getInt(8))
            assertEquals(0, status.getInt(12))
            assertEquals(0, status.getInt(16))
            assertEquals(LENGTH_UNKNOWN, status.getInt(20))
            assertRejects(PluginException.UNAVAILABLE, "Activation without a receiver must fail") {
                plugin.invoke(REQUEST_ACTIVATION, explanation("Protect lost devices"))
            }
            assertRejects(PluginException.PERMISSION_DENIED, "Lock without admin must fail") {
                plugin.invoke(LOCK_NOW, empty())
            }
            assertRejects(PluginException.INVALID_ARGUMENT, "Wipe without confirmation must fail") {
                plugin.invoke(WIPE_DATA, words(1, 0, 0))
            }
            assertRejects(PluginException.PERMISSION_DENIED, "Confirmed wipe without admin must fail") {
                plugin.invoke(WIPE_DATA, words(1, WIPE_CONFIRMATION_TOKEN, 0))
            }
        }

    @Test
    fun closedPluginRejectsNewWork() =
        withPlugin { plugin ->
            plugin.close()
            assertRejects(PluginException.CANCELLED, "Closed plugin accepted work") {
                plugin.invoke(STATUS, empty())
            }
        }

    @Test
    fun malformedPolicyRequestsAreRejectedBeforeTouchingTheOs() =
        withPlugin { plugin ->
            assertRejects(PluginException.INVALID_ARGUMENT, "Blank explanation") {
                plugin.invoke(REQUEST_ACTIVATION, explanation(" \n"))
            }
            assertRejects(PluginException.INVALID_ARGUMENT, "Unknown password quality") {
                plugin.invoke(SET_PASSWORD_QUALITY, words(6))
            }
            assertRejects(PluginException.INVALID_ARGUMENT, "Unknown camera flag") {
                plugin.invoke(SET_CAMERA_DISABLED, words(2))
            }
            assertRejects(PluginException.INVALID_ARGUMENT, "Unknown wipe flags") {
                plugin.invoke(WIPE_DATA, words(1, WIPE_CONFIRMATION_TOKEN, 2))
            }
            assertRejects(PluginException.INVALID_ARGUMENT, "Trailing status bytes") {
                plugin.invoke(STATUS, ByteBuffer.wrap(byteArrayOf(0)))
            }
        }

    private fun withPlugin(body: suspend CoroutineScope.(DevicePolicyPlugin) -> Unit) =
        runBlocking {
            ActivityScenario.launch(InvisibleActivity::class.java).use { scenario ->
                lateinit var activity: InvisibleActivity
                scenario.onActivity { activity = it as InvisibleActivity }
                withContext(Dispatchers.Main) {
                    val plugin = DevicePolicyPlugin(activity)
                    try {
                        body(plugin)
                    } finally {
                        plugin.close()
                    }
                }
            }
        }

    private suspend fun assertRejects(
        code: Int,
        message: String,
        call: suspend () -> Unit,
    ) {
        try {
            call()
            fail(message)
        } catch (error: PluginException) {
            assertEquals(message, code, error.code)
        }
    }

    private fun empty(): ByteBuffer = ByteBuffer.allocate(0)

    private fun explanation(text: String): ByteBuffer {
        val bytes = text.toByteArray(Charsets.UTF_8)
        return ByteBuffer.allocate(4 + bytes.size).order(ByteOrder.LITTLE_ENDIAN).apply {
            putInt(bytes.size)
            put(bytes)
            flip()
        }
    }

    private fun words(vararg values: Int): ByteBuffer =
        ByteBuffer.allocate(values.size * 4).order(ByteOrder.LITTLE_ENDIAN).apply {
            values.forEach(::putInt)
            flip()
        }

    /** Unattached activity used only as a Context; never shown. */
    private class InvisibleActivity :
        Activity(),
        NativeActivityResultHost {
        override suspend fun launchPluginActivity(intent: Intent): NativeActivityResult =
            throw PluginException(PluginException.UNAVAILABLE)
    }

    private companion object {
        const val STATUS = 0
        const val REQUEST_ACTIVATION = 1
        const val LOCK_NOW = 2
        const val SET_PASSWORD_QUALITY = 3
        const val SET_CAMERA_DISABLED = 5
        const val WIPE_DATA = 6
        const val STATE_UNAVAILABLE = 2
        const val CAPABILITIES_NONE = 0
        const val WIPE_CONFIRMATION_TOKEN = 0x57495045
        const val LENGTH_UNKNOWN = -1
    }
}
