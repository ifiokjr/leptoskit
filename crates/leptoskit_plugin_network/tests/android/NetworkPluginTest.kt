package dev.leptoskit.plugins.network

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Deferred
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

/** The snapshot call, which takes no arguments. */
private const val SNAPSHOT = 0

/** The change call, which takes the caller's last reading. */
private const val WAIT_FOR_CHANGE = 1

/** Eight little-endian u32 fields. */
private const val SNAPSHOT_BYTES = 32

/** The change call must answer well within the test's patience. */
private const val CHANGE_TIMEOUT_MS = 2_000L

/** Codes the wire contract allows for the connection field. */
private val CONNECTION_CODES = 0..7

@RunWith(AndroidJUnit4::class)
class NetworkPluginTest {
    @Test
    fun snapshotAndChangedSnapshotUseTheRealConnectivityService() =
        runBlocking {
            withContext(Dispatchers.Main) {
                withPlugin { plugin ->
                    validate(plugin.invoke(SNAPSHOT, ByteBuffer.allocate(0)))
                    val changed = awaitChange(plugin)
                    validate(changed)
                    assertFalse(changed.contentEquals(ByteArray(SNAPSHOT_BYTES)))
                }
            }
        }

    @Test
    fun cancellationReleasesObserverBeforeImmediateReplacementAndClose() =
        runBlocking {
            withContext(Dispatchers.Main) {
                withPlugin { plugin ->
                    val current = plugin.invoke(SNAPSHOT, ByteBuffer.allocate(0))
                    val first = startChange(plugin, current)
                    first.cancelAndJoin()
                    // A cancelled request must not leave a stale Busy waiter.
                    val replacement = startChange(plugin, current)
                    assertFalse(replacement.isCancelled)
                    plugin.close()
                    replacement.cancelAndJoin()
                    assertTrue(replacement.isCompleted)
                }
            }
        }

    /** Runs the block against a plugin bound to the target context, closing it after. */
    private suspend fun withPlugin(block: suspend (NetworkPlugin) -> Unit) {
        val plugin = NetworkPlugin(InstrumentationRegistry.getInstrumentation().targetContext)
        try {
            block(plugin)
        } finally {
            plugin.close()
        }
    }

    /** Waits for the next reading the real connectivity service reports. */
    private suspend fun awaitChange(plugin: NetworkPlugin): ByteArray =
        withTimeout(CHANGE_TIMEOUT_MS) {
            plugin.invoke(WAIT_FOR_CHANGE, ByteBuffer.allocate(SNAPSHOT_BYTES))
        }

    /** Starts one change request undispatched, so a cancel races its registration. */
    private fun CoroutineScope.startChange(
        plugin: NetworkPlugin,
        previous: ByteArray,
    ): Deferred<ByteArray> =
        async(start = CoroutineStart.UNDISPATCHED) {
            plugin.invoke(WAIT_FOR_CHANGE, ByteBuffer.wrap(previous))
        }

    private fun validate(bytes: ByteArray) {
        assertEquals(SNAPSHOT_BYTES, bytes.size)
        val input = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        assertTrue(input.int in CONNECTION_CODES)
        repeat(4) { assertTrue(input.int in 0..2) }
        input.int
        input.int
        assertTrue(input.int in 0..4)
    }
}
