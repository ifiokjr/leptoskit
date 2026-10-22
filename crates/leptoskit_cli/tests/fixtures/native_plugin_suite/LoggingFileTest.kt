package dev.leptoskit.generated

import android.content.Intent
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

class LoggingFileTest {
    @Test
    fun guestWritesStructuredRecordsToTheApplicationDirectory() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val device = UiDevice.getInstance(instrumentation)
        val launch = requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName))
        launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK)
        context.startActivity(launch)
        val button = device.wait(Until.findObject(By.desc("Check app files")), 15000)
        assertNotNull(button)
        button.click()
        assertNotNull(device.wait(Until.findObject(By.text("File roundtrip passed")), 10000))
        val records =
            File(context.filesDir, "logs/current.jsonl")
                .readLines()
                .filter(String::isNotBlank)
                .map(::JSONObject)
                .filter { it.getString("target") == "integration.worker" }
        assertTrue("Guest logging reached the application's actual log file", records.isNotEmpty())
        for (record in records) {
            assertEquals("info", record.getString("level"))
            assertEquals("Nonsecret Android plugin fixture", record.getString("message"))
            assertEquals("native_plugin_suite", record.getJSONObject("fields").getString("fixture"))
        }
    }
}
