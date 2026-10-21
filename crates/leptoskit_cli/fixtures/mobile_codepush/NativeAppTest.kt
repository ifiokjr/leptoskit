package dev.leptoskit.generated

import android.content.Intent
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.net.InetSocketAddress
import java.net.Socket
import java.net.URI

@RunWith(AndroidJUnit4::class)
class NativeAppTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val device = UiDevice.getInstance(instrumentation)

    @Test
    fun bundledAndDownloadedGuest() {
        launch()
        text("Counter 1")
        increment()
        text("Count: 1")
        capture("01-bundled-counter")

        control("replacement")
        activate()
        text("Counter 2")
        increment()
        text("Count: 10")
        capture("02-downloaded-counter")

        control("offline")
    }

    @Test
    fun offlineGuestAndStartupTrap() {
        launch()
        text("Counter 2")
        increment()
        text("Count: 10")
        capture("03-offline-relaunch")

        control("broken")
        activate()
        status("Update could not start")
        text("Counter 2")
        text("Count: 10")
        increment()
        text("Count: 20")
        capture("04-startup-trap-rollback")

        control("offline")
    }

    @Test
    fun persistedRollback() {
        launch()
        text("Counter 2")
        increment()
        text("Count: 10")
        capture("05-persisted-rollback")

        // A rejected release must not block the next good one.
        control("second")
        activate()
        text("Counter 3")
        increment()
        text("Count: 100")
        capture("06-second-update")

        control("offline")
    }

    @Test
    fun secondUpdateSurvivesRelaunch() {
        launch()
        text("Counter 3")
        increment()
        text("Count: 100")
        capture("07-second-update-relaunch")
    }

    private fun text(value: String) {
        assertNotNull(value, device.wait(Until.findObject(By.text(value)), 20000))
    }

    private fun status(prefix: String) {
        assertNotNull(prefix, device.wait(Until.findObject(By.textStartsWith(prefix)), 30000))
    }

    private fun increment() {
        checkNotNull(device.wait(Until.findObject(By.desc("Increment")), 15000)).click()
    }

    private fun activate() {
        val action = By.desc("leptoskit-update-action")
        checkNotNull(device.wait(Until.findObject(action), 15000)).click()
        val downloaded = device.wait(Until.findObject(By.textEndsWith(" downloaded")), 30000)
        val status = device.findObject(By.desc("leptoskit-update-status"))?.text
        assertNotNull("download did not finish; status: $status", downloaded)
        checkNotNull(device.wait(Until.findObject(By.copy(action).text("Activate update")), 15000)).click()
    }

    private fun launch() {
        val intent = checkNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName))
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK)
        context.startActivity(intent)
    }

    private fun capture(name: String) {
        val directory = checkNotNull(context.getExternalFilesDir("leptoskit-codepush"))
        assertTrue(directory.isDirectory || directory.mkdirs())
        assertTrue(device.takeScreenshot(File(directory, "$name.png")))
    }

    // A raw socket keeps the app's cleartext policy unchanged: only Rust code,
    // opted in through codepush.allow_http, talks plain HTTP in this test.
    private fun control(operation: String) {
        val server = URI("__SERVER__")
        Socket().use { socket ->
            socket.connect(InetSocketAddress(server.host, server.port), 15000)
            socket.soTimeout = 15000
            val request = "GET /control/$operation HTTP/1.1\r\nHost: ${server.host}\r\nConnection: close\r\n\r\n"
            socket.getOutputStream().apply {
                write(request.toByteArray(Charsets.US_ASCII))
                flush()
            }
            val status = socket.getInputStream().bufferedReader(Charsets.US_ASCII).readLine()
            assertEquals("HTTP/1.1 200 OK", status)
        }
    }
}
