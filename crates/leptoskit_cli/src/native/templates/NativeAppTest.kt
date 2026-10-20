package dev.leptoskit.generated

import android.content.Intent
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import dev.leptoskit.plugins.battery.BatteryPlugin
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.regex.Pattern

@RunWith(AndroidJUnit4::class)
class NativeAppTest {
    @Test
    fun batteryNotificationsReleaseCancelledAndClosedWaiters() =
        runBlocking {
            val instrumentation = InstrumentationRegistry.getInstrumentation()
            val device = UiDevice.getInstance(instrumentation)
            try {
                device.executeShellCommand("cmd battery set present true")
                device.executeShellCommand("cmd battery set level 42")
                withContext(Dispatchers.Main) {
                    val plugin = BatteryPlugin(instrumentation.targetContext)
                    try {
                        val previous = plugin.invoke(0, ByteBuffer.allocate(0))
                        assertEquals(420, ByteBuffer.wrap(previous).order(ByteOrder.LITTLE_ENDIAN).getInt(4))
                        val cancelled =
                            async(start = CoroutineStart.UNDISPATCHED) {
                                plugin.invoke(1, ByteBuffer.wrap(previous))
                            }
                        assertFalse(cancelled.isCompleted)
                        cancelled.cancelAndJoin()
                        val changed =
                            async(start = CoroutineStart.UNDISPATCHED) {
                                plugin.invoke(1, ByteBuffer.wrap(previous))
                            }
                        assertFalse(changed.isCompleted)
                        withContext(Dispatchers.IO) {
                            device.executeShellCommand("cmd battery set level 37")
                        }
                        val snapshot = withTimeout(10000) { changed.await() }
                        assertEquals(16, snapshot.size)
                        assertEquals(370, ByteBuffer.wrap(snapshot).order(ByteOrder.LITTLE_ENDIAN).getInt(4))
                        val closing =
                            async(start = CoroutineStart.UNDISPATCHED) {
                                plugin.invoke(1, ByteBuffer.wrap(snapshot))
                            }
                        assertFalse(closing.isCompleted)
                        plugin.close()
                        withTimeout(10000) { closing.join() }
                        assertTrue(closing.isCancelled)
                    } finally {
                        plugin.close()
                    }
                }
            } finally {
                device.executeShellCommand("cmd battery reset")
            }
        }

    @Test
    fun rustAndNativePluginsUpdateTheInterface() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val device = UiDevice.getInstance(instrumentation)
        val launch = checkNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName))
        launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK)
        context.startActivity(launch)
        val hash = device.wait(Until.findObject(By.desc("Hash with Rust")), 15000)
        assertNotNull("Generated app shows its native button", hash)
        hash.click()
        assertNotNull(device.wait(Until.findObject(By.text("SHA-256: 32 bytes")), 10000))
        device.findObject(By.desc("Copy to clipboard")).click()
        assertNotNull(device.wait(Until.findObject(By.text("Copied to clipboard")), 10000))
        device.findObject(By.desc("Play haptic feedback")).click()
        // A successful request does not imply that a simulator produced a vibration.
        assertNotNull(device.wait(Until.findObject(By.text("Haptic request accepted")), 10000))
        try {
            device.executeShellCommand("cmd battery set present true")
            device.executeShellCommand("cmd battery set level 37")
            device.findObject(By.desc("Read battery")).click()
            assertNotNull(device.wait(Until.findObject(By.text("Battery: 37%")), 10000))
        } finally {
            device.executeShellCommand("cmd battery reset")
        }
        repeat(2) {
            device.findObject(By.desc("Read device")).click()
            assertNotNull(
                device.wait(
                    Until.findObject(By.text(Pattern.compile("Device: Android; memory: [1-9][0-9]* MiB"))),
                    10000,
                ),
            )
            device.findObject(By.desc("Check secure storage")).click()
            assertNotNull(device.wait(Until.findObject(By.text("Secure storage roundtrip passed")), 10000))
            device.findObject(By.desc("Check app files")).click()
            assertNotNull(device.wait(Until.findObject(By.text("File roundtrip passed")), 10000))
        }
    }
}
