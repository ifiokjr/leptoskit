package dev.leptoskit.mobile

import dev.leptoskit.bridge.NativeRequest

/** App/bridge access belongs to the UI thread; retained jobs are independent. */
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

    @JvmStatic external fun pollService(app: Long): Int

    /** [measurer] is a [NativeTextMeasurer], typed loosely so this file stays free of Android classes. */
    @JvmStatic external fun setTextMeasurer(
        app: Long,
        measurer: Any,
    ): Int

    @JvmStatic external fun nextCancelled(app: Long): Long

    @JvmStatic external fun beginClose(app: Long): Long

    @JvmStatic external fun pollClose(owner: Long): Int

    @JvmStatic external fun releaseClose(owner: Long)

    @JvmStatic external fun plugins(app: Long): Long

    @JvmStatic external fun start(
        app: Long,
        width: Float,
        height: Float,
    ): Int

    @JvmStatic external fun resize(
        app: Long,
        width: Float,
        height: Float,
        dark: Boolean,
    ): Int

    @JvmStatic external fun wake(app: Long): Int

    /** Offers the guest the display frame clock; call before [start]. */
    @JvmStatic external fun enableFrames(app: Long): Int

    /** 1 while the guest waits for a display frame, 0 otherwise. */
    @JvmStatic external fun frameRequested(app: Long): Int

    /** Delivers a display frame; pump before the frame callback returns. */
    @JvmStatic external fun frame(
        app: Long,
        timestampNs: Long,
        periodNs: Long,
    ): Int

    @JvmStatic external fun event(
        app: Long,
        node: Long,
        kind: Int,
        text: ByteArray,
    ): Int

    @JvmStatic external fun render(app: Long): ByteArray

    /** The same transaction as [render], in the binary form [RenderBatch] reads. */
    @JvmStatic external fun renderBinary(app: Long): ByteArray

    /** 1 when [render] has changes to send; 0 when this pump changed nothing. */
    @JvmStatic external fun renderPending(app: Long): Int

    @JvmStatic external fun hasRive(app: Long): Boolean

    /** [renderer] implements synchronous riveVisible/presentRive image callbacks. */
    @JvmStatic external fun riveTick(
        app: Long,
        elapsed: Double,
        scale: Float,
        flags: Int,
        renderer: Any,
    ): Int

    @JvmStatic external fun error(app: Long): ByteArray

    @JvmStatic external fun nextRequest(
        app: Long,
        status: IntArray,
    ): NativeRequest?

    @JvmStatic external fun worker(
        app: Long,
        plugin: Int,
        method: Int,
        owner: Long,
    ): Long

    @JvmStatic external fun executeWorker(worker: Long): Long

    @JvmStatic external fun releaseWorker(worker: Long)

    @JvmStatic external fun completeWorker(
        bridge: Long,
        id: Long,
        reply: Long,
    ): Int

    @JvmStatic external fun releaseWorkerReply(reply: Long)

    @JvmStatic external fun workerCancellation(worker: Long): Long

    @JvmStatic external fun cancelWorker(token: Long)

    @JvmStatic external fun releaseWorkerCancellation(token: Long)

    @JvmStatic external fun asyncJob(
        app: Long,
        id: Long,
        plugin: Int,
        method: Int,
        owner: Long,
    ): Long

    @JvmStatic external fun asyncCancellation(job: Long): Long

    @JvmStatic external fun cancelAsync(token: Long)

    @JvmStatic external fun releaseAsyncCancellation(token: Long)

    @JvmStatic external fun releaseAsync(job: Long)

    @JvmStatic external fun startAsync(
        job: Long,
        completion: Any,
    )
}
