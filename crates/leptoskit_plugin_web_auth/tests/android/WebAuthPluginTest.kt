package dev.leptoskit.plugins.webauth

import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** No test opens a browser: the plugin's context is not an activity-result host. */
class WebAuthPluginTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    private fun authenticate(
        session: Int,
        url: String,
    ): ByteBuffer {
        val bytes = url.toByteArray()
        return ByteBuffer
            .allocate(4 + bytes.size)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(session)
            .put(bytes)
            .flip() as ByteBuffer
    }

    private fun code(block: () -> Unit): Int {
        try {
            block()
        } catch (error: PluginException) {
            return error.code
        }
        fail("expected a plugin failure")
        return 0
    }

    @Test
    fun returnAddressesAndPagesAreChecked() {
        assertEquals("com.example.app:/auth/native/return", returnAddress("com.Example.App"))
        assertNull(returnAddress("app"))
        assertNull(returnAddress("com.my_app"))
        val request = WebAuthRequest.decode(authenticate(7, "https://accounts.example/authorize?state=x"))
        assertEquals(7, request.session)
        for (page in listOf("http://example.com/", "file:///etc/passwd", "javascript:alert(1)", "https://a:b@example.com/")) {
            assertEquals(PluginException.INVALID_ARGUMENT, code { WebAuthRequest.decode(authenticate(1, page)) })
        }
        assertTrue(cameBack("com.example.app:/auth/native/return?state=a", "com.example.app:/auth/native/return"))
        assertFalse(cameBack("com.evil.app:/auth/native/return?state=a", "com.example.app:/auth/native/return"))
        assertFalse(cameBack("com.example.app:/auth/native/return#x", "com.example.app:/auth/native/return"))
    }

    @Test
    fun sessionsBeginBeforeTheyOpenAndNeedAnActivity() =
        runBlocking {
            withContext(Dispatchers.Main) {
                val plugin = WebAuthPlugin(context)
                assertEquals(
                    PluginException.INVALID_CALL,
                    code { runBlocking { plugin.invoke(1, authenticate(1, "https://example.com/")) } },
                )
                val begun = ByteBuffer.wrap(plugin.invoke(0, ByteBuffer.allocate(0))).order(ByteOrder.LITTLE_ENDIAN)
                val session = begun.int
                assertTrue(String(begun.array(), 4, begun.remaining(), Charsets.UTF_8).endsWith(":/auth/native/return"))
                assertEquals(
                    PluginException.UNAVAILABLE,
                    code { runBlocking { plugin.invoke(1, authenticate(session, "https://example.com/")) } },
                )
                plugin.close()
                assertEquals(PluginException.CANCELLED, code { runBlocking { plugin.invoke(0, ByteBuffer.allocate(0)) } })
            }
        }
}
