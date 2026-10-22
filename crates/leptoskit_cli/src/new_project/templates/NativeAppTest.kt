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
    fun sharedCounterUpdates() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val device = UiDevice.getInstance(instrumentation)
        val launch = checkNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName))
        launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK)
        context.startActivity(launch)
        val button = device.wait(Until.findObject(By.desc("Add one")), 15000)
        assertNotNull("Shared app shows its counter button", button)
        button.click()
        assertNotNull(device.wait(Until.findObject(By.text("Count: 1")), 10000))
    }
}
