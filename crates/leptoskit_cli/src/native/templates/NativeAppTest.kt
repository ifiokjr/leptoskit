package dev.leptoskit.generated

import android.content.Intent
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.junit.Assert.assertNotNull
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class NativeAppTest {
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
    }
}
