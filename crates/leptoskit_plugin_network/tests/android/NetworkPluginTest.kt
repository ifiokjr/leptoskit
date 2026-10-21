package dev.leptoskit.plugins.network

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.nio.ByteBuffer
import java.nio.ByteOrder

@RunWith(AndroidJUnit4::class)
class NetworkPluginTest {
    @Test
    fun snapshotAndChangedSnapshotUseTheRealConnectivityService() =
        runBlocking {
            withContext(Dispatchers.Main) {
                val plugin = NetworkPlugin(InstrumentationRegistry.getInstrumentation().targetContext)
                try {
                    validate(plugin.invoke(0, ByteBuffer.allocate(0)))
                    val changed = withTimeout(2000) { plugin.invoke(1, ByteBuffer.allocate(32)) }
                    validate(changed)
                    assertFalse(changed.contentEquals(ByteArray(32)))
                } finally {
                    plugin.close()
                }
            }
        }

    @Test
    fun cancellationReleasesObserverBeforeImmediateReplacementAndClose() =
        runBlocking {
            withContext(Dispatchers.Main) {
                val plugin = NetworkPlugin(InstrumentationRegistry.getInstrumentation().targetContext)
                try {
                    val current = plugin.invoke(0, ByteBuffer.allocate(0))
                    val first =
                        async(start = CoroutineStart.UNDISPATCHED) {
                            plugin.invoke(1, ByteBuffer.wrap(current))
                        }
                    first.cancelAndJoin()
                    // A cancelled request must not leave a stale Busy waiter.
                    val replacement =
                        async(start = CoroutineStart.UNDISPATCHED) {
                            plugin.invoke(1, ByteBuffer.wrap(current))
                        }
                    assertFalse(replacement.isCancelled)
                    plugin.close()
                    replacement.cancelAndJoin()
                    assertTrue(replacement.isCompleted)
                } finally {
                    plugin.close()
                }
            }
        }

    private fun validate(bytes: ByteArray) {
        assertEquals(32, bytes.size)
        val input = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        assertTrue(input.int in 0..7)
        repeat(4) { assertTrue(input.int in 0..2) }
        input.int
        input.int
        assertTrue(input.int in 0..4)
    }
}
