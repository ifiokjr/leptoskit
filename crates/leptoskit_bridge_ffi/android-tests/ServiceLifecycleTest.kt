package dev.leptoskit.mobile

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.async
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.runBlocking
import java.util.concurrent.ConcurrentLinkedQueue

/** JVM controls for the actual retained scheduler; Android rendering is separate. */
fun main() =
    runBlocking {
        pendingJoinScenario()
        joinedErrorScenario()
        multipleOwnersScenario()
        widgetColorControl()
    }

private suspend fun CoroutineScope.pendingJoinScenario() {
    val control = LifecycleControl()
    val owner = control.owner
    assertOwnerThread(owner)
    Thread { owner.wake() }.apply {
        start()
        join()
    }
    check(control.active == 0)
    control.drain()
    check(control.active == 1)
    owner.stop()
    owner.close(PENDING_HANDLE)

    owner.finishAdmission()
    val cancelled = async(start = CoroutineStart.UNDISPATCHED) { owner.awaitClosed() }
    control.drain()
    check(control.polls == 1 && control.releases == 0 && !cancelled.isCompleted)
    cancelled.cancelAndJoin()
    control.joined = true
    val replacement = mutableListOf<String>()
    val waiting =
        async(start = CoroutineStart.UNDISPATCHED) {
            owner.awaitClosed()
            replacement.add("mounted")
        }
    check(replacement.isEmpty())
    Thread { owner.wake() }.apply {
        start()
        join()
    }
    control.drain()
    waiting.await()
    check(replacement == listOf("mounted"))
    check(control.releases == 1 && control.polls == 2 && control.active == 1)

    owner.wake()
    owner.stop()
    owner.finishAdmission()
    control.drain()
    owner.awaitClosed()
    check(control.releases == 1 && control.polls == 2 && control.active == 1)
    println("PASS pending/wake, main-owner polling, cancelled await, retained join, late wake, repeated close")
}

private class LifecycleControl {
    private val main = Thread.currentThread()
    private val turns = ConcurrentLinkedQueue<() -> Unit>()
    var polls = 0
    var releases = 0
    var active = 0
    var joined = false
    val owner =
        NativeServiceOwner(
            post = { turns.add(it) },
            poll = {
                check(Thread.currentThread() === main)
                check(it == PENDING_HANDLE)
                polls++
                if (joined) 1 else 0
            },
            release = {
                check(it == PENDING_HANDLE)
                releases++
            },
            active = { active++ },
        )

    fun drain() {
        drainOwnerTurns(turns)
    }
}

private suspend fun joinedErrorScenario() {
    val turns = ConcurrentLinkedQueue<() -> Unit>()

    fun drain() {
        drainOwnerTurns(turns)
    }
    var errorReleases = 0
    val failed =
        NativeServiceOwner(
            post = { turns.add(it) },
            poll = { JOINED_ERROR },
            release = { errorReleases++ },
            active = null,
        )
    failed.stop()
    failed.close(FAILED_HANDLE)
    failed.finishAdmission()
    drain()
    repeat(2) {
        val result = runCatching { failed.awaitClosed() }
        check(result.exceptionOrNull()?.message == "Service close failed: $JOINED_ERROR")
    }
    failed.wake()
    drain()
    check(errorReleases == 1)
    println("PASS joined close error, stable repeated result, exactly-once release")
}

private const val PENDING_HANDLE = 17L
private const val FAILED_HANDLE = 23L
private const val JOINED_ERROR = -9

private fun assertOwnerThread(owner: NativeServiceOwner) {
    var failure: Throwable? = null
    Thread { failure = runCatching { owner.close(PENDING_HANDLE) }.exceptionOrNull() }.apply {
        start()
        join()
    }
    check(failure is IllegalStateException)
    check(failure?.message == "Service lifecycle requires its owner thread")
}

/** Two handles must share retention until both native joins complete. */
private suspend fun CoroutineScope.multipleOwnersScenario() {
    val turns = ConcurrentLinkedQueue<() -> Unit>()
    val joined = mutableSetOf(PENDING_HANDLE)
    val released = mutableListOf<Long>()
    val owner =
        NativeServiceOwner(
            post = { turns.add(it) },
            poll = { if (it in joined) 1 else 0 },
            release = { released.add(it) },
            active = { error("Closing owner delivered active work") },
        )
    owner.close(PENDING_HANDLE)
    owner.close(FAILED_HANDLE)
    owner.finishAdmission()
    val waiting = async(start = CoroutineStart.UNDISPATCHED) { owner.awaitClosed() }

    drainOwnerTurns(turns)
    check(released == listOf(PENDING_HANDLE))
    check(!waiting.isCompleted)
    joined.add(FAILED_HANDLE)
    owner.wake()

    drainOwnerTurns(turns)
    waiting.await()
    check(released == listOf(PENDING_HANDLE, FAILED_HANDLE))

    owner.wake()

    drainOwnerTurns(turns)
    check(released == listOf(PENDING_HANDLE, FAILED_HANDLE))
    println("PASS multiple closing handles retain admission until every join and release once")
}

private fun widgetColorControl() {
    check(NativeWidgetColor.argb(TRANSPARENT_RGBA) == TRANSPARENT_ARGB)
    check(NativeWidgetColor.argb(OPAQUE_RGBA) == OPAQUE_ARGB)
    check(NativeWidgetColor.argb(MIXED_RGBA).toUInt() == MIXED_ARGB)
    println("PASS extracted widget RGBA channel order and alpha boundaries")
}

private const val TRANSPARENT_RGBA = 0x12345600L
private const val TRANSPARENT_ARGB = 0x00123456
private const val OPAQUE_RGBA = 0xFFFFFFFFL
private const val OPAQUE_ARGB = -1
private const val MIXED_RGBA = 0x12345678L
private const val MIXED_ARGB = 0x78123456U

private fun drainOwnerTurns(turns: ConcurrentLinkedQueue<() -> Unit>) {
    while (true) (turns.poll() ?: break).invoke()
}
