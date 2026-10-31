package dev.leptoskit.mobile

import android.os.Handler
import android.util.Log
import dev.leptoskit.bridge.NativeDispatcher
import dev.leptoskit.bridge.NativeJobBudget
import dev.leptoskit.bridge.NativeRequest
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch
import kotlin.coroutines.coroutineContext

/** Runs the platform calls the guest thread posts, and hands their replies back to it. */
internal class NativePluginJobs(
    private val guest: () -> Long,
    private val dispatcher: NativeDispatcher,
    private val handler: Handler,
    private val onCapacity: () -> Unit,
) {
    private class PluginJob(
        val task: Job,
        val worker: NativeRustOperation?,
    ) {
        fun cancel() {
            worker?.cancel()
            task.cancel()
        }
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val jobs = mutableMapOf<Long, PluginJob>()
    private val budget = NativeJobBudget()
    val canTakeRequest get() = budget.canTakeRequest

    fun cancel(id: Long) {
        jobs[id]?.cancel()
    }

    fun close(): List<Job> {
        val joining = jobs.values.map { it.task }

        for (job in jobs.values) job.cancel()
        scope.cancel()

        return joining
    }

    /** Runs [request], with the Rust [worker] or async job the guest resolved for it, if any. */
    fun launch(
        request: NativeRequest,
        worker: NativeRustOperation?,
    ) {
        try {
            budget.acquire(request.id, request.bytes().remaining())
        } catch (error: Exception) {
            request.close()
            worker?.close()
            throw error
        }
        // Starts in place: a plugin that answers at once replies in this turn
        // rather than one main-looper message later. Completion only posts
        // the registry's cleanup, so it never runs before the job is stored.
        val task = scope.launch(start = CoroutineStart.UNDISPATCHED) { execute(request, worker) }
        task.invokeOnCompletion { completed(request, worker) }

        jobs[request.id] = PluginJob(task, worker)
    }

    private suspend fun execute(
        request: NativeRequest,
        worker: NativeRustOperation?,
    ) {
        if (worker != null) {
            worker.execute()
            coroutineContext.ensureActive()
            completeWorker(worker, request.id)

            return
        }

        dispatch(request)
    }

    private fun completeWorker(
        worker: NativeRustOperation,
        id: Long,
    ) {
        val guest = guest()

        if (guest != 0L) worker.complete(guest, id)
    }

    /** Preserve cancellation while translating only plugin failures into ABI replies. */
    private suspend fun dispatch(request: NativeRequest) {
        var code = 0
        val payload =
            try {
                dispatcher.dispatch(request)
            } catch (error: CancellationException) {
                throw error
            } catch (error: PluginException) {
                code = error.code

                byteArrayOf()
            } catch (error: Exception) {
                Log.e("LeptosKit", "Native plugin call failed", error)
                code = PluginException.NATIVE
                byteArrayOf()
            }

        val guest = guest()

        if (guest != 0L) NativeApp.guestComplete(guest, request.id, code, payload)
    }

    private fun completed(
        request: NativeRequest,
        worker: NativeRustOperation?,
    ) {
        // Completion may precede the coroutine body or run on a worker thread.
        request.close()
        worker?.close()
        handler.post {
            jobs.remove(request.id)
            budget.release(request.id)
            onCapacity()
        }
    }
}
