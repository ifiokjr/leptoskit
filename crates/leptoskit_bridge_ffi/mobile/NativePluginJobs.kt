package dev.leptoskit.mobile

import android.os.Handler
import android.util.Log
import dev.leptoskit.bridge.NativeBridge
import dev.leptoskit.bridge.NativeDispatcher
import dev.leptoskit.bridge.NativeJobBudget
import dev.leptoskit.bridge.NativeRequest
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch
import kotlin.coroutines.coroutineContext

internal class NativePluginJobs(
    private val app: () -> Long,
    private val dispatcher: NativeDispatcher,
    private val handler: Handler,
    private val runUi: (() -> Unit) -> Unit,
    private val checkStatus: (Int) -> Unit,
    private val schedulePump: () -> Unit,
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

    fun launch(request: NativeRequest) {
        try {
            budget.acquire(request.id, request.bytes().remaining())
        } catch (error: Exception) {
            request.close()
            throw error
        }
        val worker: NativeRustOperation? =
            NativeRustWorker.retain(app(), request) ?: NativeRustAsync.retain(app(), request)
        // Main queues execution until the registry owns the job.
        val task = scope.launch { execute(request, worker) }
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
        if (app() == 0L) return
        runUi { complete(worker.complete(NativeApp.plugins(app()), id)) }
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

        if (app() == 0L) return
        runUi { complete(NativeBridge.complete(NativeApp.plugins(app()), request.id, code, payload)) }
    }

    private fun complete(status: Int) {
        if (status == -PluginException.INVALID_CALL) return
        checkStatus(status)
        checkStatus(NativeApp.wake(app()))
        schedulePump()
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
            schedulePump()
        }
    }
}
