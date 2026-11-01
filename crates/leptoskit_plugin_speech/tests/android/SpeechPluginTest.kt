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
    @Test
    fun invalidRequestsNeverStartEngineAndClosedPluginRejectsWork() =
        runBlocking {
            withContext(Dispatchers.Main) {
                val plugin = SpeechPlugin(InstrumentationRegistry.getInstrumentation().targetContext)
                try {
                    assertArrayEquals(byteArrayOf(0, 0, 0, 0), plugin.invoke(3, ByteBuffer.allocate(0)))
                    assertArrayEquals(ByteArray(0), plugin.invoke(2, ByteBuffer.allocate(0)))
                    for (bytes in listOf(ByteArray(0), ByteArray(15), byteArrayOf(2, 0, 0, 0))) {
                        try {
                            plugin.invoke(1, ByteBuffer.wrap(bytes))
                            fail("Malformed speech must fail before initializing an engine")
                        } catch (error: PluginException) {
                            assertEquals(PluginException.INVALID_ARGUMENT, error.code)
                        }
                    }
                } finally {
                    plugin.close()
                }
                try {
                    plugin.invoke(0, ByteBuffer.allocate(0))

                    fail("Closed speech must reject voice discovery")
                } catch (error: PluginException) {
                    assertEquals(PluginException.CANCELLED, error.code)
                }
                plugin.close()
            }
        }

    @Test
    fun binaryOptionsAreValidatedWithoutProducingAudio() {
        val input =
            ByteBuffer
                .allocate(31)
                .order(ByteOrder.LITTLE_ENDIAN)
                .putInt(1)
                .putInt(1000)
                .putInt(1000)
                .putInt(0)
                .putInt(3)
                .put("hey".toByteArray())
                .putInt(0)
                .putInt(0)
                .apply { flip() }
        assertEquals(0f, SpeechRequest.decode(input).volume)
        input.putInt(4, 499)
        try {
            SpeechRequest.decode(input)
            fail("Invalid rate accepted")
        } catch (
            error: PluginException,
        ) {

            assertEquals(PluginException.INVALID_ARGUMENT, error.code)
        }
    }
}
