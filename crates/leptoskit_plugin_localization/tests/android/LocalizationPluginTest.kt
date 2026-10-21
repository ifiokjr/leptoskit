package dev.leptoskit.plugins.localization

import android.content.res.Configuration
import android.os.LocaleList
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.Locale
import java.util.TimeZone

class LocalizationPluginTest {
    @Test
    fun respectsApplicationLocalesAndTheirOrder() =
        runBlocking {
            val original = InstrumentationRegistry.getInstrumentation().targetContext
            val configuration =
                Configuration(original.resources.configuration).apply {
                    setLocales(LocaleList(Locale.FRANCE, Locale.US))
                }
            val plugin = LocalizationPlugin(original.createConfigurationContext(configuration))
            val bytes = ByteBuffer.wrap(plugin.invoke(0, ByteBuffer.allocate(0))).order(ByteOrder.LITTLE_ENDIAN)

            fun text(): String = ByteArray(bytes.int).also { bytes.get(it) }.toString(Charsets.UTF_8)

            assertEquals(1, bytes.int)
            assertEquals(2, bytes.int)
            assertEquals("fr-FR", text())
            assertEquals("fr", text())
            assertEquals("FR", text())
            assertEquals("en-US", text())
            assertEquals("en", text())
            assertEquals("US", text())
            assertEquals("gregory", text())
            assertEquals(TimeZone.getDefault().id, text())
            assertTrue(bytes.int in 1..2)
            assertEquals(1, bytes.int) // France starts its week on Monday.
            assertFalse(bytes.hasRemaining())
            plugin.close()
        }
}
