package dev.leptoskit.plugins.localauthentication

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.content.pm.PackageManager
import android.os.Build
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** No test invokes a system authentication prompt. */
class LocalAuthenticationPluginTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    private class NoBiometricPermission(
        context: Context,
    ) : ContextWrapper(context) {
        override fun checkSelfPermission(permission: String): Int =
            if (permission == Manifest.permission.USE_BIOMETRIC) {
                PackageManager.PERMISSION_DENIED
            } else {
                super.checkSelfPermission(permission)
            }
    }

    /** Unattached and never shown: even a regression cannot create real authentication UI. */
    private class InvisibleActivity : Activity() {
        override fun checkSelfPermission(permission: String): Int =
            if (permission == Manifest.permission.USE_BIOMETRIC) {
                PackageManager.PERMISSION_GRANTED
            } else {
                PackageManager.PERMISSION_DENIED
            }
    }

    private fun request(): ByteBuffer {
        val reason = "Unlock notes".toByteArray()
        val cancel = "Cancel".toByteArray()
        return ByteBuffer
            .allocate(16 + reason.size + cancel.size)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(1)
            .putInt(0)
            .putInt(reason.size)
            .put(reason)
            .putInt(cancel.size)
            .put(cancel)
            .apply { flip() }
    }

    @Test
    fun missingPermissionAndCancellationNeverPrompt() =
        runBlocking {
            withContext(Dispatchers.Main) {
                val plugin = LocalAuthenticationPlugin(NoBiometricPermission(context))
                try {
                    val bytes = plugin.invoke(0, ByteBuffer.wrap(authenticationFields(0)))
                    val status = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN).getInt(8)
                    assertEquals(if (Build.VERSION.SDK_INT >= 30) 5 else 1, status)
                    try {
                        plugin.invoke(1, request())
                        fail("Missing manifest permission cannot authenticate")
                    } catch (error: PluginException) {
                        assertEquals(
                            if (Build.VERSION.SDK_INT >=
                                30
                            ) {
                                PluginException.PERMISSION_DENIED
                            } else {
                                PluginException.UNAVAILABLE
                            },
                            error.code,
                        )
                    }
                    val cancelled =
                        async(start = CoroutineStart.UNDISPATCHED) {
                            currentCoroutineContext().cancel()
                            plugin.invoke(1, request())
                        }
                    try {
                        cancelled.await()
                        fail("Cancelled request completed")
                    } catch (
                        _: CancellationException,
                    ) {
                        // Expected before native UI construction.
                    }
                    assertArrayEquals(ByteArray(0), plugin.invoke(2, ByteBuffer.allocate(0)))
                } finally {
                    plugin.close()
                }
                try {
                    plugin.invoke(0, ByteBuffer.wrap(authenticationFields(0)))
                    fail("Closed plugin accepted work")
                } catch (error: PluginException) {
                    assertEquals(PluginException.CANCELLED, error.code)
                }
            }
        }

    @Test
    fun invisibleActivityCannotStartAuthentication() =
        runBlocking {
            withContext(Dispatchers.Main) {
                val plugin = LocalAuthenticationPlugin(InvisibleActivity())
                try {
                    try {
                        plugin.invoke(1, request())
                        fail("Invisible activity accepted authentication")
                    } catch (error: PluginException) {
                        assertEquals(PluginException.UNAVAILABLE, error.code)
                    }
                } finally {
                    plugin.close()
                }
            }
        }

    @Test
    fun destroyingTheOwningActivityClosesThePluginWithoutPrompting() =
        runBlocking {
            val intent = requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName))
            lateinit var plugin: LocalAuthenticationPlugin
            ActivityScenario.launch<Activity>(intent).use { scenario ->
                scenario.onActivity { activity -> plugin = LocalAuthenticationPlugin(activity) }
            }
            withContext(Dispatchers.Main) {
                try {
                    try {
                        plugin.invoke(0, ByteBuffer.wrap(authenticationFields(0)))
                        fail("Destroyed activity's authentication plugin accepted work")
                    } catch (error: PluginException) {
                        assertEquals(PluginException.CANCELLED, error.code)
                    }
                } finally {
                    plugin.close()
                }
            }
        }

    @Test
    fun invalidPoliciesAndTextAreRejectedByDecoder() {
        val input = request()
        assertEquals("Unlock notes", AuthenticationRequest.decode(input).reason)
        input.putInt(4, 2)
        try {
            AuthenticationRequest.decode(input)
            fail("Unknown policy accepted")
        } catch (
            error: PluginException,
        ) {
            assertEquals(PluginException.INVALID_ARGUMENT, error.code)
        }
        input.putInt(4, 0)
        input.put(12, 255.toByte())
        try {
            AuthenticationRequest.decode(input)
            fail("Invalid UTF-8 accepted")
        } catch (
            error: PluginException,
        ) {
            assertEquals(PluginException.INVALID_ARGUMENT, error.code)
        }
    }
}
