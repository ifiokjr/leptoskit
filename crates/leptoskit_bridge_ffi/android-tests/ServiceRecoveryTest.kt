package dev.leptoskit.mobile

import android.widget.Button
import android.widget.FrameLayout
import android.widget.TextView
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.yield
import sun.misc.Unsafe
import java.lang.reflect.InvocationTargetException
import java.util.concurrent.ConcurrentLinkedQueue
import kotlin.coroutines.intrinsics.suspendCoroutineUninterceptedOrReturn

private const val JOINED_HANDLE = 17L
private const val STALE_HANDLE = 42L
private const val CANCELLED_HANDLE = 99L
private const val OLD_CLOSE_ERROR = -21
private const val FAILED_CLOSE_ERROR = -13

private val allocator =
    Unsafe::class.java.getDeclaredField("theUnsafe").let {
        it.isAccessible = true
        it.get(null) as Unsafe
    }

private fun <T : Any> allocate(type: Class<T>): T = checkNotNull(type.cast(allocator.allocateInstance(type)))

private fun put(
    target: Any,
    name: String,
    value: Any?,
) {
    target.javaClass
        .getDeclaredField(name)
        .apply { isAccessible = true }
        .set(target, value)
}

private fun get(
    target: Any,
    name: String,
): Any? =
    target.javaClass
        .getDeclaredField(name)
        .apply { isAccessible = true }
        .get(target)

private suspend fun invokeSuspend(
    target: Any,
    name: String,
    vararg args: Any?,
) {
    suspendCoroutineUninterceptedOrReturn<Unit> { continuation ->
        val method = target.javaClass.declaredMethods.single { it.name == name }
        method.isAccessible = true
        try {
            method.invoke(target, *args, continuation)
        } catch (error: InvocationTargetException) {
            throw checkNotNull(error.cause)
        }
    }
}

private fun guest(status: Int): LeptosKitView {
    val turns = ConcurrentLinkedQueue<() -> Unit>()
    val service = NativeServiceOwner({ turns.add(it) }, { status }, {}, null)
    service.close(JOINED_HANDLE)
    service.finishAdmission()

    while (true) (turns.poll() ?: break).invoke()
    val view = allocate(LeptosKitView::class.java)
    put(view, "closeStarted", true)
    put(view, "closingJobs", emptyList<Job>())

    put(view, "services", service)

    return view
}

private fun application(): LeptosKitApplicationView {
    val outer = allocate(LeptosKitApplicationView::class.java)
    put(outer, "guests", allocate(FrameLayout::class.java))
    put(outer, "status", allocate(TextView::class.java))
    put(outer, "action", allocate(Button::class.java))
    put(outer, "retiring", mutableSetOf<LeptosKitView>())
    put(outer, "transitions", mutableSetOf<Job>())
    put(outer, "lifecycle", CoroutineScope(SupervisorJob() + Dispatchers.Unconfined))
    put(outer, "closeCompletion", CompletableDeferred<Unit>())

    put(outer, "generation", 1L)

    return outer
}

private val launch = NativeLaunch(byteArrayOf(), byteArrayOf(), "control", 1, true)

private suspend fun promotion() {
    val outer = application()
    val old = guest(OLD_CLOSE_ERROR)
    val next = guest(1)
    put(outer, "current", old)
    put(outer, "pending", next)
    FrameLayout.removed.clear()
    invokeSuspend(outer, "promote", launch, 1L, next)
    check(get(outer, "current") === next)

    check(get(outer, "pending") == null)
    check(old in FrameLayout.removed && next !in FrameLayout.removed)
    check(runCatching { old.closeAndJoin() }.exceptionOrNull()?.message == "Service close failed: -21")
    check((get(outer, "status") as TextView).text.toString().contains("Service close failed: -21"))
    println("PASS promotion retires joined-error current and exposes the healthy candidate with its cleanup diagnostic")
}

private suspend fun rejection() {
    val failed = guest(FAILED_CLOSE_ERROR)
    val recovery = application()
    val releases = allocate(NativeReleases::class.java)
    put(releases, "closed", true)
    put(recovery, "current", guest(1))
    val primary = IllegalStateException("guest failed")
    invokeSuspend(
        recovery,
        "rejectAfterJoin",
        releases,
        launch,
        1L,
        failed,
        primary,
    )
    val status = get(recovery, "status") as TextView

    check(primary.suppressed.map { it.message } == listOf("Service close failed: -13", "Update store is closed"))
    check(status.text.toString().contains("guest failed; Service close failed: -13; Update store is closed"))
    check(failed in FrameLayout.removed)
    val successRecovery = application()
    put(successRecovery, "current", guest(1))
    invokeSuspend(
        successRecovery,
        "rejectAfterJoin",
        releases,
        launch,
        1L,
        guest(1),
        IllegalStateException("guest failed"),
    )
    val controlStatus = get(successRecovery, "status") as TextView
    check(controlStatus.text.toString().contains("guest failed; Update store is closed"))

    println("PASS joined-error recovery reaches actual NativeReleases.reject and preserves primary plus cleanup diagnostics")
}

private suspend fun staleRecovery() =
    kotlinx.coroutines.coroutineScope {
        val releases = allocate(NativeReleases::class.java)
        put(releases, "closed", true)
        val stale = application()
        val turns = ConcurrentLinkedQueue<() -> Unit>()
        var joinedStatus = 0
        val pendingService = NativeServiceOwner({ turns.add(it) }, { joinedStatus }, {}, null)
        pendingService.close(STALE_HANDLE)
        pendingService.finishAdmission()

        while (true) (turns.poll() ?: break).invoke()
        val pendingGuest = guest(1)
        put(pendingGuest, "services", pendingService)
        val delayed =
            async(start = CoroutineStart.UNDISPATCHED) {
                invokeSuspend(
                    stale,
                    "rejectAfterJoin",
                    releases,
                    launch,
                    1L,
                    pendingGuest,
                    IllegalStateException("old guest"),
                )
            }
        check(!delayed.isCompleted)
        put(stale, "generation", 2L)
        joinedStatus = -31
        pendingService.wake()

        while (true) (turns.poll() ?: break).invoke()
        delayed.await()
        check((get(stale, "status") as TextView).text == null)
        check(get(stale, "recoveryFailure") == null)
        println("PASS retired generation joined error cannot overwrite current status")
    }

private suspend fun cancellation() =
    kotlinx.coroutines.coroutineScope {
        val cancelled = application()
        val queued = ConcurrentLinkedQueue<() -> Unit>()
        var nativeStatus = 0
        var released = 0
        val waitingOwner = NativeServiceOwner({ queued.add(it) }, { nativeStatus }, { released += 1 }, null)
        waitingOwner.close(CANCELLED_HANDLE)
        waitingOwner.finishAdmission()

        while (true) (queued.poll() ?: break).invoke()
        val waitingGuest = guest(1)
        put(waitingGuest, "services", waitingOwner)
        put(cancelled, "current", waitingGuest)
        val replacement = guest(1)
        put(cancelled, "pending", replacement)
        FrameLayout.removed.clear()
        val waitingPromotion =
            async(start = CoroutineStart.UNDISPATCHED) {
                invokeSuspend(cancelled, "promote", launch, 1L, replacement)
            }
        waitingPromotion.cancel()

        yield()
        check(!waitingPromotion.isCompleted && released == 0 && waitingGuest !in FrameLayout.removed)
        check(get(cancelled, "current") === waitingGuest)
        nativeStatus = -41
        waitingOwner.wake()

        while (true) (queued.poll() ?: break).invoke()
        waitingPromotion.cancelAndJoin()
        check(released == 1 && waitingGuest in FrameLayout.removed)
        check(get(cancelled, "current") === replacement)
        println("PASS cancelling a transition retains Pending ownership until true joined cleanup and releases once")
    }

private suspend fun candidateIdentity() {
    val staleCandidate = application()
    val admitted = guest(1)
    val unrelated = guest(1)
    put(staleCandidate, "pending", admitted)
    invokeSuspend(staleCandidate, "promote", launch, 1L, unrelated)
    check(get(staleCandidate, "pending") === admitted && get(staleCandidate, "current") == null)
    check((get(staleCandidate, "status") as TextView).text == null)
    println("PASS promotion rejects a mismatched candidate identity without mutations")
}

private suspend fun stalePromotion() =
    kotlinx.coroutines.coroutineScope {
        val outer = application()
        val turns = ConcurrentLinkedQueue<() -> Unit>()
        var nativeStatus = 0
        val services = NativeServiceOwner({ turns.add(it) }, { nativeStatus }, {}, null)
        services.close(STALE_HANDLE)
        services.finishAdmission()
        val previous = guest(1)
        val candidate = guest(1)
        val newer = guest(1)
        put(previous, "services", services)
        put(outer, "current", previous)
        put(outer, "pending", candidate)
        val promotion =
            async(start = CoroutineStart.UNDISPATCHED) {
                invokeSuspend(outer, "promote", launch, 1L, candidate)
            }
        check(!promotion.isCompleted)
        put(outer, "generation", 2L)
        put(outer, "pending", newer)
        nativeStatus = 1
        services.wake()

        while (true) (turns.poll() ?: break).invoke()
        promotion.await()
        check(get(outer, "current") == null && get(outer, "pending") === newer)
        check(previous in FrameLayout.removed && newer !in FrameLayout.removed)
        check((get(outer, "status") as TextView).text == null)
        println("PASS stale successful join retires only its closed guest and cannot promote over a newer candidate")
    }

private suspend fun initializationCancellation() =
    kotlinx.coroutines.coroutineScope {
        val outer = application()
        val turns = ConcurrentLinkedQueue<() -> Unit>()
        var nativeStatus = 0
        var released = 0
        val services = NativeServiceOwner({ turns.add(it) }, { nativeStatus }, { released += 1 }, null)
        services.close(CANCELLED_HANDLE)
        services.finishAdmission()
        val failure = NativeApplicationInitializationFailure(IllegalStateException("construction failed")) { services.awaitClosed() }
        val recovery =
            async(start = CoroutineStart.UNDISPATCHED) {
                invokeSuspend(outer, "recoverInitialization", launch, 1L, failure)
            }
        recovery.cancel()

        yield()
        check(!recovery.isCompleted && released == 0 && get(outer, "generation") == 1L)
        nativeStatus = OLD_CLOSE_ERROR
        services.wake()

        while (true) (turns.poll() ?: break).invoke()
        recovery.cancelAndJoin()
        check(released == 1 && get(outer, "generation") == 2L)
        check(failure.suppressed.single().message == "Service close failed: -21")
        println("PASS cancelled failed-construction waiter still joins native cleanup before recovery and retains its diagnostic")
    }

private fun fallback() {
    val recovery = application()
    val releases = allocate(NativeReleases::class.java)
    val primary = IllegalStateException("guest failed")
    put(releases, "closed", true)
    put(recovery, "releases", releases)
    put(recovery, "recoveryFailure", primary)
    val completion = recovery.javaClass.declaredMethods.single { it.name.startsWith("reportRejection") }
    completion.isAccessible = true

    val failure = runCatching { completion.invoke(recovery, Unit, launch, 1L, primary) }.exceptionOrNull()

    check(failure is InvocationTargetException && failure.cause?.message == "Update store is closed")
    check((get(recovery, "status") as TextView).text.toString() == "Starting application…")
    check(get(recovery, "recoveryFailure") === primary)
    println("PASS successful candidate rejection reaches actual cold fallback activation with its diagnostic retained")
}

private suspend fun unload() {
    val old = guest(OLD_CLOSE_ERROR)
    val failed = guest(FAILED_CLOSE_ERROR)
    val unload = application()
    put(unload, "current", old)
    put(unload, "pending", failed)
    val unloadError = runCatching { unload.closeAndJoin() }.exceptionOrNull()
    check(unloadError?.message == "Service close failed: -13")
    check(unloadError.suppressed.single().message == "Service close failed: -21")

    check(get(unload, "current") == null && get(unload, "pending") == null)
    println("PASS actual outer unload clears both slots and aggregates both joined errors")
}

fun main(arguments: Array<String>) =
    runBlocking {
        val checks: Map<String, suspend () -> Unit> =
            linkedMapOf<String, suspend () -> Unit>(
                "promotion" to { promotion() },
                "rejection" to { rejection() },
                "stale" to { staleRecovery() },
                "cancellation" to { cancellation() },
                "identity" to { candidateIdentity() },
                "stale-promotion" to { stalePromotion() },
                "initialization" to { initializationCancellation() },
                "fallback" to { fallback() },
                "unload" to { unload() },
            )
        val selected = arguments.singleOrNull()?.let { listOf(checkNotNull(checks[it])) } ?: checks.values

        for (check in selected) check()
    }
