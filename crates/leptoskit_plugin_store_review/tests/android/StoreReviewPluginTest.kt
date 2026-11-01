package dev.leptoskit.plugins.storereview

import android.app.Activity
import android.content.Context
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import com.google.android.gms.tasks.Task
import com.google.android.gms.tasks.TaskCompletionSource
import com.google.android.play.core.review.ReviewInfo
import com.google.android.play.core.review.ReviewManager
import com.google.android.play.core.review.testing.FakeReviewManager
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.supervisorScope
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Assert.fail
import org.junit.Test
import java.nio.ByteBuffer

/** Uses Google's fake and controlled completion tasks; never opens Play review UI. */
class StoreReviewPluginTest {
    private class ControlledManager(
        context: Context,
    ) : ReviewManager {
        private val fake = FakeReviewManager(context)
        var requests = 0
        var launches = 0
        var launchedOwner: Activity? = null
        private var preparation = TaskCompletionSource<ReviewInfo>()
        private var flow = TaskCompletionSource<Void>()

        override fun requestReviewFlow(): Task<ReviewInfo> {
            requests++
            preparation = TaskCompletionSource()
            flow = TaskCompletionSource()

            return preparation.task
        }

        override fun launchReviewFlow(
            activity: Activity,
            reviewInfo: ReviewInfo,
        ): Task<Void> {
            launches++
            launchedOwner = activity

            return flow.task
        }

        fun prepared() {
            val pending = preparation
            fake.requestReviewFlow().addOnCompleteListener { pending.setResult(it.result) }
        }

        fun completed() {
            flow.setResult(null)
        }
    }

    @Test
    fun cancellationBeforeLaunchAndDuringFlowPreservesExclusiveOwnership() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val intent = requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName))
            ActivityScenario.launch<Activity>(intent).use { scenario ->
                val ownerResult = CompletableDeferred<Activity>()
                scenario.onActivity { ownerResult.complete(it) }
                val owner = ownerResult.await()
                awaitCondition { owner.hasWindowFocus() }
                val manager = ControlledManager(context)
                val plugin = withContext(Dispatchers.Main) { StoreReviewPlugin(owner, manager) }
                try {
                    supervisorScope {
                        val preparing = async(Dispatchers.Main) { plugin.invoke(1, empty()) }
                        awaitCondition { manager.requests == 1 }
                        assertEquals(3, withContext(Dispatchers.Main) { plugin.invoke(0, empty())[0].toInt() })
                        withContext(Dispatchers.Main) {
                            try {
                                plugin.invoke(1, empty())
                                fail("A concurrent flow was accepted")
                            } catch (error: PluginException) {
                                assertEquals(PluginException.BUSY, error.code)
                            }
                        }
                        preparing.cancel()
                        preparing.join()
                        withContext(Dispatchers.Main) { manager.prepared() }
                        awaitCondition { plugin.invoke(0, empty())[0].toInt() == 1 }
                        assertEquals(0, manager.launches)

                        val launched = async(Dispatchers.Main) { plugin.invoke(1, empty()) }
                        awaitCondition { manager.requests == 2 }
                        withContext(Dispatchers.Main) { manager.prepared() }
                        awaitCondition { manager.launches == 1 }
                        assertSame(owner, manager.launchedOwner)
                        launched.cancel()
                        launched.join()
                        assertEquals(3, withContext(Dispatchers.Main) { plugin.invoke(0, empty())[0].toInt() })
                        withContext(Dispatchers.Main) { manager.completed() }
                        awaitCondition { plugin.invoke(0, empty())[0].toInt() == 1 }

                        val accepted = async(Dispatchers.Main) { plugin.invoke(1, empty()) }
                        awaitCondition { manager.requests == 3 }
                        withContext(Dispatchers.Main) { manager.prepared() }
                        awaitCondition { manager.launches == 2 }
                        withContext(Dispatchers.Main) { manager.completed() }
                        assertEquals(listOf<Byte>(1, 0, 0, 0), accepted.await().toList())

                        val closing = async(Dispatchers.Main) { plugin.invoke(1, empty()) }
                        awaitCondition { manager.requests == 4 }
                        withContext(Dispatchers.Main) { plugin.close() }
                        try {
                            closing.await()
                            fail("Closing should cancel the caller")
                        } catch (error: PluginException) {
                            assertEquals(PluginException.CANCELLED, error.code)
                        }
                        withContext(Dispatchers.Main) { manager.prepared() }
                        // Flush the posted task completion without permitting a real store implementation.
                        InstrumentationRegistry.getInstrumentation().waitForIdleSync()
                        assertEquals(2, manager.launches)
                    }
                } finally {
                    withContext(Dispatchers.Main) { plugin.close() }
                }
            }
        }

    @Test
    fun malformedMessagesAndMissingActivityNeverStartStoreWork() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val manager = ControlledManager(context)
            withContext(Dispatchers.Main) {
                val plugin = StoreReviewPlugin(context, manager)
                try {
                    assertEquals(2, plugin.invoke(0, empty())[0].toInt())

                    for (method in listOf(0, 1)) {
                        try {
                            plugin.invoke(method, ByteBuffer.wrap(byteArrayOf(0)))
                            fail("Payload must be empty")
                        } catch (error: PluginException) {
                            assertEquals(PluginException.INVALID_ARGUMENT, error.code)
                        }
                    }

                    try {
                        plugin.invoke(1, empty())
                        fail("An application context cannot present a review")
                    } catch (error: PluginException) {
                        assertEquals(PluginException.UNAVAILABLE, error.code)
                    }

                    assertEquals(0, manager.requests)
                } finally {
                    plugin.close()
                }
            }
        }

    private suspend fun awaitCondition(condition: suspend () -> Boolean) =
        withTimeout(5000) {
            while (!withContext(Dispatchers.Main) { condition() }) delay(10)
        }

    private fun empty(): ByteBuffer = ByteBuffer.allocate(0)
}
