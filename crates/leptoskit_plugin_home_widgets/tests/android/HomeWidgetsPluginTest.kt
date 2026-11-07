package dev.leptoskit.plugins.homewidgets

import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Test
import org.junit.runner.RunWith
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Exercises storage and wire validation without pinning a real app widget. */
@RunWith(AndroidJUnit4::class)
class HomeWidgetsPluginTest {
    @Test
    fun setGetClearAndCapabilityRoundTrip() =
        runBlocking {
            withContext(Dispatchers.Main) {
                val context = ApplicationProvider.getApplicationContext<android.content.Context>()
                val plugin = HomeWidgetsPlugin(context)
                try {
                    val capability = plugin.invoke(CAPABILITY, empty())
                    assertEquals(4, capability.size)
                    val flags =
                        ByteBuffer
                            .wrap(capability)
                            .order(ByteOrder.LITTLE_ENDIAN)
                            .int
                    assertEquals(1 or (1 shl 1) or (1 shl 2) or (1 shl 3), flags)

                    val snapshot = encodeSnapshot()
                    assertArrayEquals(byteArrayOf(), plugin.invoke(SET, buffer(snapshot)))
                    assertArrayEquals(snapshot, plugin.invoke(GET, buffer(kindOnly("status"))))
                    assertArrayEquals(byteArrayOf(), plugin.invoke(CLEAR, buffer(kindOnly("status"))))
                    expectError(PluginException.UNAVAILABLE) {
                        plugin.invoke(GET, buffer(kindOnly("status")))
                    }
                } finally {
                    plugin.close()
                }
            }
        }

    @Test
    fun malformedSnapshotsAreRejected() =
        runBlocking {
            withContext(Dispatchers.Main) {
                val context = ApplicationProvider.getApplicationContext<android.content.Context>()
                val plugin = HomeWidgetsPlugin(context)
                try {
                    expectError(PluginException.INVALID_ARGUMENT) {
                        plugin.invoke(SET, buffer(byteArrayOf(0, 0, 0, 1, 0)))
                    }
                    expectError(PluginException.INVALID_ARGUMENT) {
                        plugin.invoke(CAPABILITY, buffer(byteArrayOf(0)))
                    }
                } finally {
                    plugin.close()
                }
            }
        }

    private suspend fun expectError(
        code: Int,
        block: suspend () -> Unit,
    ) {
        try {
            block()
            fail("expected PluginException($code)")
        } catch (error: PluginException) {
            assertEquals(code, error.code)
        }
    }

    private companion object {
        const val CAPABILITY = 0
        const val SET = 1
        const val GET = 2
        const val CLEAR = 3

        fun empty(): ByteBuffer = ByteBuffer.allocate(0)

        fun buffer(bytes: ByteArray): ByteBuffer = ByteBuffer.wrap(bytes)

        fun kindOnly(kind: String): ByteArray = lengthPrefixed(kind)

        fun encodeSnapshot(
            kind: String = "status",
            family: Int = 2,
            title: String = "Inbox",
            body: String = "3 unread",
            url: String = "myapp://inbox",
        ): ByteArray {
            val out = ArrayList<Byte>()
            fun append(value: String) {
                val utf8 = value.toByteArray(Charsets.UTF_8)
                val length =
                    ByteBuffer
                        .allocate(4)
                        .order(ByteOrder.LITTLE_ENDIAN)
                        .putInt(utf8.size)
                        .array()
                out.addAll(length.toList())
                out.addAll(utf8.toList())
            }
            append(kind)
            out.addAll(
                ByteBuffer
                    .allocate(4)
                    .order(ByteOrder.LITTLE_ENDIAN)
                    .putInt(family)
                    .array()
                    .toList(),
            )
            append(title)
            append(body)
            append(url)
            return out.toByteArray()
        }

        fun lengthPrefixed(value: String): ByteArray {
            val utf8 = value.toByteArray(Charsets.UTF_8)
            return ByteBuffer
                .allocate(4 + utf8.size)
                .order(ByteOrder.LITTLE_ENDIAN)
                .putInt(utf8.size)
                .put(utf8)
                .array()
        }
    }
}
