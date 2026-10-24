package dev.leptoskit.mobile

import kotlinx.coroutines.runBlocking
import java.util.concurrent.ConcurrentLinkedQueue

internal object ServiceJniControl {
    @JvmStatic external fun closeAndWake()
}

/** Actual JVM/JNI arbitrary-thread callback delivery and global-reference use. */
fun main(arguments: Array<String>) =
    runBlocking {
        System.load(arguments.single())
        val main = Thread.currentThread()
        val queue = ConcurrentLinkedQueue<() -> Unit>()
        var polls = 0
        var releases = 0
        var active = 0
        val owner =
            NativeServiceOwner(
                post = { queue.add(it) },
                poll = {
                    check(Thread.currentThread() === main)
                    check(it == 17L)
                    polls++
                    1
                },
                release = { releases++ },
                active = { active++ },
            )
        check(NativeApp.activateService(0, owner) == 0)
        ServiceJniControl.closeAndWake()
        owner.finishAdmission()
        check(polls == 0 && releases == 0 && active == 0)

        while (true) (queue.poll() ?: break).invoke()
        owner.awaitClosed()
        check(polls == 1 && releases == 1 && active == 0)
        println("PASS actual JNI activation, cross-thread wake/release, retained Java callback and queued closing owner")
    }
