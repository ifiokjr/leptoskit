package dev.leptoskit.mobile

import dev.leptoskit.bridge.NativeRequest

/**
 * The app is created and registered on the UI thread, then lent to its guest
 * thread with [guestSpawn]; the guest functions post to it and take what it
 * posted without waiting. Retained jobs are independent of both.
 */
object NativeApp {
    @JvmStatic external fun create(
        wasm: ByteArray,
        dataPath: ByteArray,
        cachePath: ByteArray,
    ): Long

    @JvmStatic external fun destroy(app: Long)

    @JvmStatic external fun activateService(
        app: Long,
        callbacks: NativeServiceOwner,
    ): Int

    /** [measurer] is a [NativeTextMeasurer], typed loosely so this file stays free of Android classes. */
    @JvmStatic external fun setTextMeasurer(
        app: Long,
        measurer: Any,
    ): Int

    @JvmStatic external fun beginClose(app: Long): Long

    @JvmStatic external fun pollClose(owner: Long): Int

    @JvmStatic external fun releaseClose(owner: Long)

    @JvmStatic external fun plugins(app: Long): Long

    /** Offers the guest the display frame clock; call before [guestSpawn]. */
    @JvmStatic external fun enableFrames(app: Long): Int

    @JvmStatic external fun error(app: Long): ByteArray

    /**
     * Lends [app] to a new guest thread, which calls [notifier]'s `notified()`
     * after it posts something. Returns 0 when it cannot start.
     */
    @JvmStatic external fun guestSpawn(
        app: Long,
        notifier: Any,
    ): Long

    /** Waits for the guest's step in progress, then gives the app back to close. */
    @JvmStatic external fun guestStop(guest: Long): Int

    /** Call on each notification, before taking what the guest posted. */
    @JvmStatic external fun guestAck(guest: Long)

    @JvmStatic external fun guestStart(
        guest: Long,
        width: Float,
        height: Float,
    ): Int

    @JvmStatic external fun guestResize(
        guest: Long,
        width: Float,
        height: Float,
        dark: Boolean,
    ): Int

    @JvmStatic external fun guestEvent(
        guest: Long,
        node: Long,
        kind: Int,
        text: ByteArray,
    ): Int

    @JvmStatic external fun guestFrame(
        guest: Long,
        timestampNs: Long,
        periodNs: Long,
    ): Int

    /** 1 while the guest waits for a display frame, 0 otherwise. */
    @JvmStatic external fun guestFrameRequested(guest: Long): Int

    @JvmStatic external fun guestWake(guest: Long): Int

    @JvmStatic external fun guestComplete(
        guest: Long,
        id: Long,
        error: Int,
        payload: ByteArray,
    ): Int

    @JvmStatic external fun guestCompleteWorker(
        guest: Long,
        id: Long,
        reply: Long,
    ): Int

    /** The oldest binary transaction the guest posted, or an empty array. */
    @JvmStatic external fun guestTakeRender(guest: Long): ByteArray

    /** The next platform call, with its worker and async job handles written to [jobs], or null. */
    @JvmStatic external fun guestNextRequest(
        guest: Long,
        jobs: LongArray,
    ): NativeRequest?

    @JvmStatic external fun guestNextCancelled(guest: Long): Long

    @JvmStatic external fun guestHasRive(guest: Long): Boolean

    /** Queues a Rive tick for the [visible] views; returns whether the last tick wanted another. */
    @JvmStatic external fun guestRiveTick(
        guest: Long,
        elapsed: Double,
        scale: Float,
        flags: Int,
        visible: LongArray,
    ): Int

    /** The next Rive frame's pixels, with its view, width and height written to [frame], or an empty array. */
    @JvmStatic external fun guestTakeRiveFrame(
        guest: Long,
        frame: LongArray,
    ): ByteArray

    @JvmStatic external fun guestFailed(guest: Long): Boolean

    @JvmStatic external fun guestError(guest: Long): ByteArray

    @JvmStatic external fun executeWorker(worker: Long): Long

    @JvmStatic external fun releaseWorker(worker: Long)

    @JvmStatic external fun releaseWorkerReply(reply: Long)

    @JvmStatic external fun workerCancellation(worker: Long): Long

    @JvmStatic external fun cancelWorker(token: Long)

    @JvmStatic external fun releaseWorkerCancellation(token: Long)

    @JvmStatic external fun asyncCancellation(job: Long): Long

    @JvmStatic external fun cancelAsync(token: Long)

    @JvmStatic external fun releaseAsyncCancellation(token: Long)

    @JvmStatic external fun releaseAsync(job: Long)

    @JvmStatic external fun startAsync(
        job: Long,
        completion: Any,
    )
}
