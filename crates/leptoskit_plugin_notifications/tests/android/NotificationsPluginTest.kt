package dev.leptoskit.plugins.notifications

import android.Manifest
import android.app.Notification
import android.app.NotificationManager
import android.content.pm.PackageManager
import android.os.Build
import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.fail
import org.junit.Test
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Isolated emulator only: posts silent notifications into the generated test app. */
class NotificationsPluginTest {
    @Test
    fun localDeliveryReplacementAndCancellationAreNamespaceScoped() =
        runBlocking {
            val instrumentation = InstrumentationRegistry.getInstrumentation()
            val context = instrumentation.targetContext
            if (Build.VERSION.SDK_INT >= 33 &&
                context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
            ) {
                // This permission belongs to the disposable test app and lasts until fixture uninstall.
                instrumentation.uiAutomation.grantRuntimePermission(context.packageName, Manifest.permission.POST_NOTIFICATIONS)
            }
            val manager = context.getSystemService(NotificationManager::class.java)
            val id = "test-${System.nanoTime()}"
            val channel = "$id-channel"
            val tag = NotificationsPlugin.PREFIX + id
            val channelId = NotificationsPlugin.PREFIX + channel
            val unrelatedTag = "other-notification-provider-$id"
            val plugin = withContext(Dispatchers.Main) { NotificationsPlugin(context) }
            try {
                withContext(Dispatchers.Main) {
                    plugin.invoke(4, fields(listOf(channel, "Test channel", "Silent instrumentation"), listOf(1, 0)))
                    val configured = manager.getNotificationChannel(channelId)
                    assertNotNull(configured)
                    assertEquals(NotificationManager.IMPORTANCE_LOW, configured.importance)
                    assertNull(configured.sound)
                    try {
                        plugin.invoke(2, fields(listOf(id, "Missing channel", "", "$channel-missing")))
                        fail("Posting without an existing channel was accepted")

                    } catch (error: PluginException) {
                        assertEquals(PluginException.INVALID_ARGUMENT, error.code)
                    }
                    plugin.invoke(2, fields(listOf(id, "First title", "Body", channel)))
                }
                awaitNotification(manager, tag, "First title")
                withContext(Dispatchers.Main) {
                    plugin.invoke(2, fields(listOf(id, "Replacement", "Updated", channel)))
                    manager.notify(
                        unrelatedTag,
                        42,
                        Notification
                            .Builder(context, channelId)
                            .setSmallIcon(android.R.drawable.ic_dialog_info)
                            .setContentTitle("Other provider")
                            .build(),
                    )
                }
                awaitNotification(manager, tag, "Replacement")
                awaitNotification(manager, unrelatedTag, "Other provider")
                assertEquals(1, manager.activeNotifications.count { it.tag == tag })
                withContext(Dispatchers.Main) { plugin.invoke(3, fields(listOf(id))) }
                awaitAbsent(manager, tag)
                withContext(Dispatchers.Main) {
                    plugin.invoke(2, fields(listOf(id, "Cancel all", "", channel)))
                }
                awaitNotification(manager, tag, "Cancel all")
                withContext(Dispatchers.Main) { plugin.invoke(6, ByteBuffer.allocate(0)) }
                awaitAbsent(manager, tag)
                assertEquals(true, manager.activeNotifications.any { it.tag == unrelatedTag })
                withContext(Dispatchers.Main) {
                    plugin.invoke(2, fields(listOf(id, "Persists after close", "", channel)))
                    plugin.close()
                    try {
                        plugin.invoke(0, ByteBuffer.allocate(0))
                        fail("Closed handler accepted a query")
                    } catch (error: PluginException) {
                        assertEquals(PluginException.CANCELLED, error.code)
                    }
                }
                awaitNotification(manager, tag, "Persists after close")

            } finally {
                withContext(Dispatchers.Main) { plugin.close() }
                manager.cancel(tag, 0)
                manager.cancel(unrelatedTag, 42)
                manager.deleteNotificationChannel(channelId)
            }
        }

    @Test
    fun malformedPayloadsNeverPost() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val plugin = withContext(Dispatchers.Main) { NotificationsPlugin(context) }
            try {
                withContext(Dispatchers.Main) {
                    val invalid =
                        listOf(
                            fields(listOf("id", "", "body", "general")),
                            fields(listOf("id", "Title", "bad\u0000body", "general")),
                            ByteBuffer.wrap(byteArrayOf(-1, -1, -1, -1)),
                        )

                    for (payload in invalid) {
                        try {
                            plugin.invoke(2, payload)
                            fail("Invalid content must be rejected before posting")
                        } catch (error: PluginException) {
                            assertEquals(PluginException.INVALID_ARGUMENT, error.code)
                        }
                    }

                    try {
                        plugin.invoke(4, fields(listOf("invalid-channel", "Test", ""), listOf(4, 0)))
                        fail("Unknown channel importance was accepted")
                    } catch (error: PluginException) {
                        assertEquals(PluginException.INVALID_ARGUMENT, error.code)
                    }
                }
            } finally {
                withContext(Dispatchers.Main) { plugin.close() }
            }
        }

    private suspend fun awaitNotification(
        manager: NotificationManager,
        tag: String,
        title: String,
    ) = withTimeout(5000) {
        while (manager.activeNotifications.none {
                it.tag == tag && it.notification.extras
                    .getCharSequence(Notification.EXTRA_TITLE)
                    ?.toString() == title
            }
        ) {
            delay(20)
        }
    }

    private suspend fun awaitAbsent(
        manager: NotificationManager,
        tag: String,
    ) = withTimeout(5000) {
        while (manager.activeNotifications.any { it.tag == tag }) delay(20)
        assertFalse(manager.activeNotifications.any { it.tag == tag })
    }

    private fun fields(
        strings: List<String>,
        numbers: List<Int> = emptyList(),
    ): ByteBuffer {
        val output = ByteArrayOutputStream()

        for (value in strings) {
            val bytes = value.toByteArray(Charsets.UTF_8)
            output.write(
                ByteBuffer
                    .allocate(4)
                    .order(ByteOrder.LITTLE_ENDIAN)
                    .putInt(bytes.size)
                    .array(),
            )
            output.write(bytes)
        }

        for (value in numbers) {
            output.write(
                ByteBuffer
                    .allocate(4)
                    .order(ByteOrder.LITTLE_ENDIAN)
                    .putInt(value)
                    .array(),
            )
        }

        return ByteBuffer.wrap(output.toByteArray())
    }
}
