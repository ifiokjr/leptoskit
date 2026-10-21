package dev.leptoskit.plugins.application

import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

class ApplicationPluginTest {
    @Test
    fun reportsTheInstalledApplication() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val plugin = ApplicationPlugin(context)

            @Suppress("DEPRECATION")
            val installed = context.packageManager.getPackageInfo(context.packageName, 0)
            val bytes = ByteBuffer.wrap(plugin.invoke(0, ByteBuffer.allocate(0))).order(ByteOrder.LITTLE_ENDIAN)

            fun text(): String = ByteArray(bytes.int).also { bytes.get(it) }.toString(Charsets.UTF_8)

            assertEquals(1, bytes.int)
            assertEquals(context.packageName, text())
            assertEquals(context.applicationInfo.loadLabel(context.packageManager).toString(), text())
            assertEquals(installed.versionName ?: "", text())
            @Suppress("DEPRECATION")
            val build = installed.versionCode.toString()
            assertEquals(build, text())
            assertFalse(bytes.hasRemaining())
            plugin.close()
        }
}
