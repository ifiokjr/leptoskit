package dev.leptoskit.plugins.passkeys

import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Test
import java.nio.ByteBuffer

/** No test shows the credential sheet: the plugin's context is not an activity. */
class PasskeysPluginTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    private fun code(block: () -> Unit): Int {
        try {
            block()
        } catch (error: PluginException) {
            return error.code
        }
        fail("expected a plugin failure")

        return 0
    }

    private fun bytes(text: String) = ByteBuffer.wrap(text.toByteArray())

    @Test
    fun optionsAreBoundedJsonWithAChallenge() {
        val options = """{"challenge":"AAEC","rpId":"notes.example"}"""
        assertEquals(options, webAuthnOptions(bytes(options)))

        for (bad in listOf("", "[]", "{not json", """{"rpId":"a"}""", """{"challenge":""}""")) {
            assertEquals(PluginException.INVALID_ARGUMENT, code { webAuthnOptions(bytes(bad)) })
        }

        val huge = """{"challenge":"${"a".repeat(MAX_JSON_BYTES)}"}"""
        assertEquals(PluginException.INVALID_ARGUMENT, code { webAuthnOptions(bytes(huge)) })
        assertEquals(PluginException.NATIVE, code { webAuthnAnswer("nope") })
    }

    @Test
    fun nothingIsShownWithoutAnActivity() =
        runBlocking {
            withContext(Dispatchers.Main) {
                val plugin = PasskeysPlugin(context)
                val options = bytes("""{"challenge":"AAEC","rpId":"notes.example"}""")
                assertEquals(PluginException.UNAVAILABLE, code { runBlocking { plugin.invoke(1, options) } })
                assertEquals(PluginException.UNAVAILABLE, code { runBlocking { plugin.invoke(2, bytes("nonce")) } })
                assertEquals(PluginException.INVALID_ARGUMENT, code { runBlocking { plugin.invoke(0, bytes("{}")) } })
                plugin.close()
                assertEquals(PluginException.CANCELLED, code { runBlocking { plugin.invoke(1, options) } })
            }
        }
}
