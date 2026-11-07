package dev.leptoskit.plugins.pictureinpicture

import android.app.Activity
import android.os.Build
import android.provider.Settings
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Wire and permission checks against a real activity; no forced PiP entry. */
class PictureInPicturePluginTest {
    @Test
    fun capabilitiesAndSnapshotMatchEmptySession() =
        runBlocking {
            withActivity { activity ->
                val plugin = PictureInPicturePlugin(activity)

                try {
                    val capabilities =
                        ByteBuffer
                            .wrap(plugin.invoke(CAPABILITIES, empty()))
                            .order(ByteOrder.LITTLE_ENDIAN)
                    assertEquals(WIRE_VERSION, capabilities.int)
                    val pip = capabilities.int
                    val overlay = capabilities.int
                    assertTrue(pip == 0 || pip == 1)
                    assertEquals(if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) 1 else 0, overlay)

                    val snapshot =
                        ByteBuffer
                            .wrap(plugin.invoke(SNAPSHOT, empty()))
                            .order(ByteOrder.LITTLE_ENDIAN)
                    assertEquals(WIRE_VERSION, snapshot.int)
                    assertEquals(SESSION_NONE, snapshot.int)
                } finally {
                    plugin.close()
                }
            }
        }

    @Test
    fun overlayStatusTracksSystemGrantWithoutPrompting() =
        runBlocking {
            withActivity { activity ->
                val plugin = PictureInPicturePlugin(activity)

                try {
                    val status =
                        ByteBuffer
                            .wrap(plugin.invoke(OVERLAY_STATUS, empty()))
                            .order(ByteOrder.LITTLE_ENDIAN)
                            .int

                    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) {
                        assertEquals(OVERLAY_UNSUPPORTED, status)
                    } else if (Settings.canDrawOverlays(activity)) {
                        assertEquals(OVERLAY_GRANTED, status)
                    } else {
                        assertEquals(OVERLAY_DENIED, status)
                        expectError(PluginException.PERMISSION_DENIED) {
                            plugin.invoke(ENTER_OVERLAY, empty())
                        }
                    }
                } finally {
                    plugin.close()
                }
            }
        }

    @Test
    fun invalidMethodsAndPayloadsAreRejected() =
        runBlocking {
            withActivity { activity ->
                val plugin = PictureInPicturePlugin(activity)

                try {
                    expectError(PluginException.UNAVAILABLE) {
                        plugin.invoke(UNKNOWN_METHOD, empty())
                    }
                    expectError(PluginException.INVALID_ARGUMENT) {
                        plugin.invoke(
                            CAPABILITIES,
                            ByteBuffer.allocate(1),
                        )
                    }

                    plugin.close()
                    expectError(PluginException.CANCELLED) {
                        plugin.invoke(SNAPSHOT, empty())
                    }
                } finally {
                    plugin.close()
                }
            }
        }

    private suspend fun withActivity(body: suspend (Activity) -> Unit) {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val intent = requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName))

        ActivityScenario.launch<Activity>(intent).use { scenario ->
            lateinit var activity: Activity

            scenario.onActivity { activity = it }

            withContext(Dispatchers.Main) { body(activity) }
        }
    }

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

    private companion object {
        const val CAPABILITIES = 0
        const val SNAPSHOT = 1
        const val ENTER_OVERLAY = 3
        const val OVERLAY_STATUS = 4
        const val UNKNOWN_METHOD = 9

        const val WIRE_VERSION = 1
        const val SESSION_NONE = 0
        const val OVERLAY_UNSUPPORTED = 0
        const val OVERLAY_DENIED = 1
        const val OVERLAY_GRANTED = 2
    }
}
