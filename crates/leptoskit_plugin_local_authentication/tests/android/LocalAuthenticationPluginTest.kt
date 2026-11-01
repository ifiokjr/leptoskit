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
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** No test invokes a system authentication prompt. */
class LocalAuthenticationPluginTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val evaluatesPolicy = Build.VERSION.SDK_INT >= BIOMETRIC_API

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

    @Test
    fun missingPermissionAndCancellationNeverPrompt() =
        withPlugin(NoBiometricPermission(context)) { plugin ->
            val report = ByteBuffer.wrap(plugin.invoke(CAPABILITIES, policy())).order(ByteOrder.LITTLE_ENDIAN)
            val readiness = if (evaluatesPolicy) READINESS_PERMISSION_DENIED else READINESS_UNAVAILABLE

            assertEquals(readiness, report.getInt(READINESS_OFFSET))
            assertRejects(
                if (evaluatesPolicy) PluginException.PERMISSION_DENIED else PluginException.UNAVAILABLE,
                "Missing manifest permission cannot authenticate",
            ) {
                plugin.invoke(AUTHENTICATE, request())
            }
            assertCancelledBeforePrompt(plugin)
            assertArrayEquals(ByteArray(0), plugin.invoke(CANCEL, ByteBuffer.allocate(0)))
        }

    @Test
    fun closedPluginRejectsNewWork() =
        withPlugin(NoBiometricPermission(context)) { plugin ->
            plugin.close()

            assertRejects(PluginException.CANCELLED, "Closed plugin accepted work") {
                plugin.invoke(CAPABILITIES, policy())
            }
        }

    @Test
    fun invisibleActivityCannotStartAuthentication() =
        withPlugin(InvisibleActivity()) { plugin ->
            assertRejects(PluginException.UNAVAILABLE, "Invisible activity accepted authentication") {
                plugin.invoke(AUTHENTICATE, request())
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
                    assertRejects(
                        PluginException.CANCELLED,
                        "Destroyed activity's authentication plugin accepted work",
                    ) {
                        plugin.invoke(CAPABILITIES, policy())
                    }
                } finally {
                    plugin.close()
                }
            }
        }

    @Test
    fun invalidPoliciesAndTextAreRejectedByDecoder() =
        runBlocking {
            val input = request()
            assertEquals("Unlock notes", AuthenticationRequest.decode(input).reason)

            input.putInt(POLICY_OFFSET, UNKNOWN_POLICY)
            assertRejects(PluginException.INVALID_ARGUMENT, "Unknown policy accepted") {
                AuthenticationRequest.decode(input)
            }

            input.putInt(POLICY_OFFSET, BIOMETRIC_POLICY)
            input.put(REASON_OFFSET, INVALID_UTF8)
            assertRejects(PluginException.INVALID_ARGUMENT, "Invalid UTF-8 accepted") {
                AuthenticationRequest.decode(input)
            }
        }

    /** Runs the body on the UI thread against a plugin over the given context, then closes it. */
    private fun withPlugin(
        base: Context,
        body: suspend CoroutineScope.(LocalAuthenticationPlugin) -> Unit,
    ) = runBlocking {
        withContext(Dispatchers.Main) {
            val plugin = LocalAuthenticationPlugin(base)

            try {
                body(plugin)
            } finally {
                plugin.close()
            }
        }
    }

    /** Expects the call to fail with the given plugin code. */
    private suspend fun assertRejects(
        code: Int,
        message: String,
        call: suspend () -> Unit,
    ) {
        try {
            call()
            fail(message)
        } catch (error: PluginException) {
            assertEquals(code, error.code)
        }
    }

    /** Starts a request that is cancelled before the OS prompt could be built. */
    private suspend fun CoroutineScope.assertCancelledBeforePrompt(plugin: LocalAuthenticationPlugin) {
        val cancelled =
            async(start = CoroutineStart.UNDISPATCHED) {
                currentCoroutineContext().cancel()
                plugin.invoke(AUTHENTICATE, request())
            }
        val failure = runCatching { cancelled.await() }.exceptionOrNull()

        assertTrue("Expected cancellation before native UI, got $failure", failure is CancellationException)
    }

    private fun policy(): ByteBuffer = ByteBuffer.wrap(authenticationFields(BIOMETRIC_POLICY))

    /** A minimal request for the biometric policy. */
    private fun request(): ByteBuffer {
        val reason = "Unlock notes".toByteArray()
        val cancel = "Cancel".toByteArray()

        return ByteBuffer
            .allocate(REQUEST_HEADER_BYTES + reason.size + cancel.size)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(WIRE_VERSION)
            .putInt(BIOMETRIC_POLICY)
            .putInt(reason.size)
            .put(reason)
            .putInt(cancel.size)
            .put(cancel)
            .apply { flip() }
    }

    private companion object {
        /** The first API level that answers readiness for a policy at all. */
        const val BIOMETRIC_API = 30

        /** Where the readiness field starts in a capabilities reply. */
        const val READINESS_OFFSET = 8

        /** Where the policy and the reason's first byte sit in an encoded request. */
        const val POLICY_OFFSET = 4
        const val REASON_OFFSET = 12

        /** Version, policy, and the two length prefixes. */
        const val REQUEST_HEADER_BYTES = 16

        /** Values the decoder must reject. */
        const val UNKNOWN_POLICY = 2
        val INVALID_UTF8 = 255.toByte()

        /** Readiness codes for an unaskable permission and an unsupported API level. */
        const val READINESS_UNAVAILABLE = 1
        const val READINESS_PERMISSION_DENIED = 5
    }
}
