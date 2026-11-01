package dev.leptoskit.mobile

import dev.leptoskit.bridge.NativeDispatcher
import dev.leptoskit.bridge.NativeRequest
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.async
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.yield
import sun.misc.Unsafe
import java.lang.reflect.InvocationTargetException
import java.nio.ByteBuffer
import java.util.concurrent.ConcurrentLinkedQueue
import kotlin.coroutines.intrinsics.suspendCoroutineUninterceptedOrReturn

private val allocator =
    Unsafe::class.java.getDeclaredField("theUnsafe").let {
        it.isAccessible = true
        it.get(null) as Unsafe
    }

private fun <T : Any> allocate(type: Class<T>): T = checkNotNull(type.cast(allocator.allocateInstance(type)))

private fun assign(
    target: Any,
    name: String,
    value: Any?,
) {
    target.javaClass
        .getDeclaredField(name)
        .apply { isAccessible = true }
        .set(target, value)
}

private suspend fun execute(
    jobs: NativePluginJobs,
    request: NativeRequest,
    operation: NativeRustOperation,
) {
    suspendCoroutineUninterceptedOrReturn<Unit> { continuation ->
        val method = jobs.javaClass.declaredMethods.single { it.name == "execute" }
        method.isAccessible = true
        try {
            method.invoke(jobs, request, operation, continuation)
        } catch (error: InvocationTargetException) {
            throw checkNotNull(error.cause)
        }
    }
}

private fun register(
    jobs: NativePluginJobs,
    operation: NativeRustOperation,
    task: kotlinx.coroutines.Job,
) {
    val type = Class.forName("dev.leptoskit.mobile.NativePluginJobs\$PluginJob")
    val constructor = type.declaredConstructors.single().apply { isAccessible = true }
    assign(jobs, "jobs", mutableMapOf(1L to constructor.newInstance(task, operation)))
}

private fun assertPendingCapacity() {
    check(AsyncTest.cancelled() == 1 && AsyncTest.liveTokens() == 1)
    check(AsyncTest.liveJobs() == 1 && AsyncTest.liveReplies() == 0)
}

private fun assertReleasedCapacity() {
    check(AsyncTest.liveJobs() == 0 && AsyncTest.liveReplies() == 0 && AsyncTest.liveTokens() == 0)
}

private fun pluginJobs(completed: () -> Unit): NativePluginJobs =
    NativePluginJobs(
        { 1L },
        NativeDispatcher(),
        allocate(android.os.Handler::class.java),
        { it() },
        { completed() },
        {},
    )

/** Actual job cancellation and View join retain a callback while service close finishes. */
fun main() =
    runBlocking {
        System.loadLibrary("async_host_test")
        var bridgeCompletions = 0
        val jobs = pluginJobs { bridgeCompletions++ }
        val request = NativeRequest(1, 1, 1, ByteBuffer.allocate(0), 1)
        val operation = checkNotNull(NativeRustAsync.retain(1, request))
        val task =
            launch(start = CoroutineStart.UNDISPATCHED) {
                operation.use { execute(jobs, request, it) }
            }
        try {
            verifyClose(jobs, operation, task) { bridgeCompletions }
        } finally {
            AsyncTest.finish()
            task.join()
        }
    }

private suspend fun kotlinx.coroutines.CoroutineScope.verifyClose(
    jobs: NativePluginJobs,
    operation: NativeRustOperation,
    task: kotlinx.coroutines.Job,
    bridgeCompletions: () -> Int,
) {
    register(jobs, operation, task)
    val closing = jobs.close()
    check(closing.single() === task && !task.isCompleted)
    assertPendingCapacity()

    val turns = ConcurrentLinkedQueue<() -> Unit>()
    var releases = 0
    val service = NativeServiceOwner({ turns.add(it) }, { 1 }, { releases++ }, null)
    check(NativeApp.activateService(0, service) == 0)
    ServiceJniControl.closeAndWake()
    service.finishAdmission()
    val view = allocate(LeptosKitView::class.java)
    assign(view, "closeStarted", true)

    assign(view, "closingJobs", closing)
    assign(view, "services", service)

    val joined = async(start = CoroutineStart.UNDISPATCHED) { view.closeAndJoin() }

    while (true) (turns.poll() ?: break).invoke()
    service.awaitClosed()

    yield()

    check(releases == 1 && !joined.isCompleted && !task.isCompleted)
    assertPendingCapacity()
    AsyncTest.finish()
    joined.await()

    check(task.isCompleted && task.isCancelled && bridgeCompletions() == 0)
    assertReleasedCapacity()
    println("PASS service join cannot release cancelled JNI async capacity before its real callback and View join")
}
