package dev.leptoskit.mobile

import dev.leptoskit.bridge.NativeJobBudget
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.joinAll
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread

object AsyncTest {
    @JvmStatic external fun started(): Int

    @JvmStatic external fun cancelled(): Int

    @JvmStatic external fun liveJobs(): Int

    @JvmStatic external fun liveReplies(): Int

    @JvmStatic external fun liveTokens(): Int

    @JvmStatic external fun finish()

    /** An async job, as the guest thread resolves one for a request. */
    @JvmStatic external fun job(mode: Int): Long
}

private fun operation(mode: Int): NativeRustAsync = checkNotNull(NativeRustAsync.adopt(AsyncTest.job(mode)))

fun main() {
    System.loadLibrary("async_host_test")
    val finished = AtomicBoolean(false)
    thread(isDaemon = true) {
        Thread.sleep(10_000)

        if (!finished.get()) Runtime.getRuntime().halt(1)
    }
    runBlocking {
        operation(0).use { immediate ->
            immediate.execute()
            check(immediate.complete(1, 1) == 0)
        }
        // An app can close before the coroutine gets its first turn.
        operation(1).use { unstarted -> unstarted.cancel() }
        check(AsyncTest.liveJobs() == 0)
        val budget = NativeJobBudget()
        val operations = (1..128).map { operation(1) }
        val jobs =
            operations.mapIndexed { index, operation ->
                val id = index.toLong() + 1
                budget.acquire(id, 0)
                launch(start = CoroutineStart.UNDISPATCHED) {
                    operation.execute()
                    coroutineContext.ensureActive()
                    error("Cancelled callback should never complete its bridge")
                }.also { job ->
                    job.invokeOnCompletion {
                        operation.close()
                        budget.release(id)
                    }
                }
            }
        check(AsyncTest.started() == 129)
        operations.zip(jobs).forEach { (operation, job) ->
            operation.cancel()
            job.cancel()
        }
        check(AsyncTest.cancelled() == 129)
        check(jobs.none { it.isCompleted })
        check(!budget.canTakeRequest)
        // Race token cancellation against the completion handler's release.
        val cancelling = AtomicBoolean(true)
        val canceller =
            thread {
                while (cancelling.get()) operations.forEach { it.cancel() }
            }
        AsyncTest.finish()
        jobs.joinAll()
        cancelling.set(false)
        canceller.join()
        check(budget.canTakeRequest)
        check(AsyncTest.liveJobs() == 0)
        check(AsyncTest.liveReplies() == 0)
        check(AsyncTest.liveTokens() == 0)
    }

    finished.set(true)
    println("JNI async callbacks retain cancelled capacity and release every owned handle")
}
