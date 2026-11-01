package dev.leptoskit.plugins.speech

import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

class SpeechPluginTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    @Test
    fun invalidRequestsNeverStartEngine() =
        withPlugin { plugin ->
            assertArrayEquals(SILENT, plugin.invoke(STATUS, ByteBuffer.allocate(0)))
            assertArrayEquals(ByteArray(0), plugin.invoke(STOP, ByteBuffer.allocate(0)))
            for (bytes in MALFORMED) {
                assertRejected(PluginException.INVALID_ARGUMENT, "Malformed speech must not start an engine") {
                    plugin.invoke(SPEAK, ByteBuffer.wrap(bytes))
                }
            }
        }

    @Test
    fun closedPluginRejectsWork() =
        withPlugin { plugin ->
            plugin.close()

            assertRejected(PluginException.CANCELLED, "Closed speech must reject voice discovery") {
                plugin.invoke(VOICES, ByteBuffer.allocate(0))
            }
        }

    @Test
    fun binaryOptionsAreValidatedWithoutProducingAudio() {
        val input = request()

        assertEquals(0f, SpeechRequest.decode(input).volume)

        input.putInt(RATE_OFFSET, THOUSANDTHS_MIN - 1)
        try {
            SpeechRequest.decode(input)
            fail("Invalid rate accepted")
        } catch (error: PluginException) {
            assertEquals(PluginException.INVALID_ARGUMENT, error.code)
        }
    }

    /** Runs the body on the UI thread against a plugin over the test context, then closes it. */
    private fun withPlugin(body: suspend (SpeechPlugin) -> Unit) =
        runBlocking {
            withContext(Dispatchers.Main) {
                val plugin = SpeechPlugin(context)

                try {
                    body(plugin)
                } finally {
                    plugin.close()
                }
            }
        }

    /** Expects one call to fail with the given plugin code. */
    private suspend fun assertRejected(
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

    /** One request: version, normal rate and pitch, silence, then "hey". */
    private fun request(): ByteBuffer =
        ByteBuffer
            .allocate(REQUEST_BYTES)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(WIRE_VERSION)
            .putInt(NORMAL)
            .putInt(NORMAL)
            .putInt(0)
            .putInt("hey".length)
            .put("hey".toByteArray())
            .putInt(0)
            .putInt(0)
            .apply { flip() }

    private companion object {
        /** Methods the plugin answers. */
        const val VOICES = 0
        const val SPEAK = 1
        const val STOP = 2
        const val STATUS = 3

        /** A request: five u32 fields, "hey", and two empty strings. */
        const val REQUEST_BYTES = 31

        /** Where the rate field starts. */
        const val RATE_OFFSET = 4

        const val WIRE_VERSION = 1
        const val NORMAL = 1000
        const val THOUSANDTHS_MIN = 500

        /** Requests the decoder must reject before an engine starts. */
        val MALFORMED = listOf(ByteArray(0), ByteArray(TRUNCATED_BYTES), byteArrayOf(2, 0, 0, 0))

        /** A request cut off before its version field ends. */
        const val TRUNCATED_BYTES = 15

        /** The speaking reply when nothing is active. */
        val SILENT = byteArrayOf(0, 0, 0, 0)
    }
}
