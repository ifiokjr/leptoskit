package dev.leptoskit.mobile

import android.os.Handler
import android.os.Looper
import dev.leptoskit.bridge.NativeBridge
import org.json.JSONObject
import java.io.File
import java.util.concurrent.Executors

/** Worker-only JNI boundary; no renderer or Android view handle crosses it. */
internal object NativeUpdates {
    @JvmStatic external fun open(
        configuration: ByteArray,
        embedded: ByteArray,
        directory: ByteArray,
    ): Long

    @JvmStatic external fun destroy(handle: Long)

    @JvmStatic external fun launch(handle: Long): Array<ByteArray>

    @JvmStatic external fun sync(handle: Long): ByteArray

    @JvmStatic external fun confirm(
        handle: Long,
        receipt: ByteArray,
    )

    @JvmStatic external fun reject(
        handle: Long,
        receipt: ByteArray,
    )
}

/** Owned launch bytes and the opaque receipt identifying exactly this launch. */
internal data class NativeLaunch(
    val wasm: ByteArray,
    val receipt: ByteArray,
    val version: String,
    val build: Long,
    val candidate: Boolean,
)

internal data class NativeSync(
    val staged: Boolean,
    val version: String,
    val build: Long,
)

/**
 * Owns the update manager on one process-wide worker. Close is queued after any
 * bounded download, before a recreated Activity can open the same private store.
 * The directory lease also rejects accidentally overlapping live application views.
 */
internal class NativeReleases(
    private val directory: File,
    private val embedded: ByteArray,
    private val configuration: ByteArray,
) : AutoCloseable {
    private companion object {
        val worker = Executors.newSingleThreadExecutor { task -> Thread(task, "leptoskit-updates") }
        val directories = mutableSetOf<String>()
    }

    private val main = Handler(Looper.getMainLooper())
    private var handle = 0L

    @Volatile private var closed = false

    fun open(completion: (Result<Unit>) -> Unit) =
        submit(completion) {
            NativeBridge.load()
            val path = directory.absolutePath
            check(directories.add(path)) { "An application already owns this update store" }
            try {
                handle = NativeUpdates.open(configuration, embedded, path.toByteArray(Charsets.UTF_8))
                check(handle != 0L) { "Cannot open update store" }
            } catch (error: Exception) {
                directories.remove(path)
                throw error
            }
        }

    fun launch(completion: (Result<NativeLaunch>) -> Unit) =
        submit(completion) {
            val values = NativeUpdates.launch(requireHandle())
            check(values.size == 2) { "Invalid native launch response" }
            val metadata = JSONObject(values[1].toString(Charsets.UTF_8))
            NativeLaunch(
                values[0],
                metadata.getString("receipt").toByteArray(Charsets.UTF_8),
                metadata.getString("version"),
                metadata.getLong("build"),
                metadata.getBoolean("candidate"),
            )
        }

    fun sync(completion: (Result<NativeSync>) -> Unit) =
        submit(completion) {
            val metadata = JSONObject(NativeUpdates.sync(requireHandle()).toString(Charsets.UTF_8))
            val status = metadata.getString("status")
            check(status == "staged" || status == "current") { "Invalid update status: $status" }
            NativeSync(status == "staged", metadata.getString("version"), metadata.getLong("build"))
        }

    fun confirm(
        launch: NativeLaunch,
        completion: (Result<Unit>) -> Unit,
    ) = submit(completion) { NativeUpdates.confirm(requireHandle(), launch.receipt) }

    fun reject(
        launch: NativeLaunch,
        completion: (Result<Unit>) -> Unit,
    ) = submit(completion) { NativeUpdates.reject(requireHandle(), launch.receipt) }

    private fun requireHandle(): Long {
        check(handle != 0L) { "Update store is not open" }
        return handle
    }

    private fun <T> submit(
        completion: (Result<T>) -> Unit,
        operation: () -> T,
    ) {
        check(!closed) { "Update store is closed" }
        worker.execute {
            if (closed) return@execute
            val result =
                try {
                    Result.success(operation())
                } catch (error: Exception) {
                    Result.failure(error)
                }
            main.post { if (!closed) completion(result) }
        }
    }

    override fun close() {
        if (closed) return
        closed = true
        worker.execute {
            if (handle != 0L) {
                NativeUpdates.destroy(handle)
                handle = 0L
                directories.remove(directory.absolutePath)
            }
        }
    }
}
