package dev.leptoskit.mobile

import kotlinx.coroutines.CompletableDeferred

/** Retained independently of a View or its coroutine scope until native join. */
class NativeServiceOwner {
    private val post: (() -> Unit) -> Unit
    private val poll: (Long) -> Int
    private val release: (Long) -> Unit
    private var active: (() -> Unit)?

    internal constructor(
        post: (() -> Unit) -> Unit,
        poll: (Long) -> Int,
        release: (Long) -> Unit,
        active: (() -> Unit)?,
    ) {
        this.post = post
        this.poll = poll
        this.release = release
        this.active = active
    }

    private companion object {
        val closing = mutableSetOf<NativeServiceOwner>()
    }

    private val handles = mutableSetOf<Long>()
    private val ownerThread = Thread.currentThread()
    private val completion = CompletableDeferred<Unit>()
    private var stopped = false
    private var failure: Exception? = null

    /** JNI may enter on any thread; this callback only enqueues an owner turn. */
    fun wake() {
        post {
            checkOwner()
            active?.invoke()
            pollClosing()
        }
    }

    /** JNI transfers a close handle on the UI owner, without reentrant polling. */
    fun close(handle: Long) {
        checkOwner()
        stop()
        check(handle != 0L && handles.add(handle)) { "Invalid closing owner" }
        closing.add(this)
        post { pollClosing() }
    }

    internal fun stop() {
        checkOwner()
        active = null
        stopped = true
    }

    internal fun finishAdmission() {
        checkOwner()
        finishIfJoined()
    }

    internal suspend fun awaitClosed() = completion.await()

    private fun checkOwner() {
        check(Thread.currentThread() === ownerThread) { "Service lifecycle requires its owner thread" }
    }

    /** Pending handles stay retained; each joined handle is released once before completion. */
    private fun pollClosing() {
        for (handle in handles.toList()) {
            val status = poll(handle)

            if (status == 0) continue
            handles.remove(handle)

            if (status < 0 && failure == null) failure = IllegalStateException("Service close failed: $status")
            release(handle)
        }

        finishIfJoined()
    }

    private fun finishIfJoined() {
        if (!stopped || handles.isNotEmpty()) return
        closing.remove(this)
        val error = failure

        if (error == null) completion.complete(Unit) else completion.completeExceptionally(error)
    }
}
