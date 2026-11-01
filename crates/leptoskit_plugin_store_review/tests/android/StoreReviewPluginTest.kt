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
import kotlinx.coroutines.CoroutineScope
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
            withActivity { owner -> checkFlowLifecycle(owner) }
        }

    @Test
    fun malformedMessagesAndMissingActivityNeverStartStoreWork() =
        runBlocking {
            val context = InstrumentationRegistry.getInstrumentation().targetContext

            withContext(Dispatchers.Main) {
                val manager = ControlledManager(context)
                val plugin = StoreReviewPlugin(context, manager)

                try {
                    assertEquals(STATUS_INACTIVE, availability(plugin))
                    checkPayloadsRejected(plugin)
                    expectError(PluginException.UNAVAILABLE) { plugin.invoke(REQUEST, empty()) }
                    assertEquals(0, manager.requests)
                } finally {
                    plugin.close()
                }
            }
        }

    /** A cancelled preparation, a cancelled flow and a close each keep the slot honest. */
    private suspend fun CoroutineScope.checkFlowLifecycle(owner: Activity) {
        val manager = ControlledManager(owner)
        val plugin = StoreReviewPlugin(owner, manager)

        try {
            supervisorScope {
                checkCancelledPreparation(plugin, manager)
                checkCancelledFlow(plugin, manager, owner)
                checkAcceptedFlow(plugin, manager)
                checkCloseCancelsTheCaller(plugin, manager)
            }
        } finally {
            withContext(Dispatchers.Main) { plugin.close() }
        }
    }

    /** A second request is busy, and a cancelled preparation never launches. */
    private suspend fun CoroutineScope.checkCancelledPreparation(
        plugin: StoreReviewPlugin,
        manager: ControlledManager,
    ) {
        val preparing = async(Dispatchers.Main) { plugin.invoke(REQUEST, empty()) }

        awaitCondition { manager.requests == 1 }
        assertEquals(STATUS_BUSY, availability(plugin))
        expectError(PluginException.BUSY) { plugin.invoke(REQUEST, empty()) }

        preparing.cancel()
        preparing.join()

        withContext(Dispatchers.Main) { manager.prepared() }
        awaitCondition { availability(plugin) == STATUS_READY }
        assertEquals(0, manager.launches)
    }

    /** Cancelling after launch keeps the slot busy until the store flow ends. */
    private suspend fun CoroutineScope.checkCancelledFlow(
        plugin: StoreReviewPlugin,
        manager: ControlledManager,
        owner: Activity,
    ) {
        val launched = async(Dispatchers.Main) { plugin.invoke(REQUEST, empty()) }

        awaitCondition { manager.requests == 2 }
        withContext(Dispatchers.Main) { manager.prepared() }
        awaitCondition { manager.launches == 1 }
        assertSame(owner, manager.launchedOwner)

        launched.cancel()
        launched.join()

        assertEquals(STATUS_BUSY, availability(plugin))

        withContext(Dispatchers.Main) { manager.completed() }
        awaitCondition { availability(plugin) == STATUS_READY }
    }

    /** A flow that completes acknowledges the request and nothing more. */
    private suspend fun CoroutineScope.checkAcceptedFlow(
        plugin: StoreReviewPlugin,
        manager: ControlledManager,
    ) {
        val accepted = async(Dispatchers.Main) { plugin.invoke(REQUEST, empty()) }

        awaitCondition { manager.requests == 3 }
        withContext(Dispatchers.Main) { manager.prepared() }
        awaitCondition { manager.launches == 2 }
        withContext(Dispatchers.Main) { manager.completed() }

        assertEquals(ACCEPTED_REPLY, accepted.await().toList())
    }

    /** Closing during preparation cancels the caller and never launches. */
    private suspend fun CoroutineScope.checkCloseCancelsTheCaller(
        plugin: StoreReviewPlugin,
        manager: ControlledManager,
    ) {
        val closing = async(Dispatchers.Main) { plugin.invoke(REQUEST, empty()) }

        awaitCondition { manager.requests == 4 }
        withContext(Dispatchers.Main) { plugin.close() }
        expectError(PluginException.CANCELLED) { closing.await() }

        withContext(Dispatchers.Main) { manager.prepared() }
        // Flush the posted task completion without permitting a real store implementation.
        InstrumentationRegistry.getInstrumentation().waitForIdleSync()
        assertEquals(2, manager.launches)
    }

    /** Both methods refuse a payload they never read. */
    private suspend fun checkPayloadsRejected(plugin: StoreReviewPlugin) {
        for (method in listOf(AVAILABILITY, REQUEST)) {
            expectError(PluginException.INVALID_ARGUMENT) { plugin.invoke(method, ByteBuffer.wrap(byteArrayOf(0))) }
        }
    }

    /** Launches the generated host, waits for its window and runs the body. */
    private suspend fun CoroutineScope.withActivity(body: suspend CoroutineScope.(Activity) -> Unit) {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val intent = requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName))

        ActivityScenario.launch<Activity>(intent).use { scenario ->
            val owner = CompletableDeferred<Activity>()

            scenario.onActivity { owner.complete(it) }

            val activity = owner.await()

            awaitCondition { activity.hasWindowFocus() }
            body(activity)
        }
    }

    /** The plugin's availability status, read on the main thread. */
    private suspend fun availability(plugin: StoreReviewPlugin): Int =
        withContext(Dispatchers.Main) {
            plugin.invoke(AVAILABILITY, empty())[0].toInt()
        }

    private suspend fun awaitCondition(condition: suspend () -> Boolean) =
        withTimeout(CONDITION_TIMEOUT_MS) {
            while (!withContext(Dispatchers.Main) { condition() }) delay(POLL_INTERVAL_MS)
        }

    private suspend fun expectError(
        code: Int,
        action: suspend () -> Unit,
    ) {
        try {
            action()

            fail("Expected plugin error $code")
        } catch (error: PluginException) {
            assertEquals(code, error.code)
        }
    }

    private fun empty(): ByteBuffer = ByteBuffer.allocate(0)

    private companion object {
        /** Method ids the plugin exposes. */
        const val AVAILABILITY = 0
        const val REQUEST = 1

        /** Availability statuses from the wire format. */
        const val STATUS_READY = 1
        const val STATUS_INACTIVE = 2

        /** The accepted request reply. */
        val ACCEPTED_REPLY = listOf<Byte>(1, 0, 0, 0)

        /** How long a posted store callback may take to arrive. */
        const val CONDITION_TIMEOUT_MS = 5_000L
        const val POLL_INTERVAL_MS = 10L
    }
}
