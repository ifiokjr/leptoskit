package dev.leptoskit.mobile

import dev.leptoskit.bridge.NativeRequest
import org.json.JSONObject

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

    @JvmStatic external fun event(
        app: Long,
        node: Long,
        kind: Int,
        text: ByteArray,
    ): Int

    @JvmStatic external fun render(app: Long): ByteArray

    /** Decode and validate the native renderer protocol at its JNI boundary. */
    internal fun renderBatch(app: Long): JSONObject {
        val batch = JSONObject(render(app).toString(Charsets.UTF_8))
        check(batch.getInt("version") == 1)

        return batch
    }

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
