package dev.leptoskit.mobile

import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext

/** A join diagnostic never turns a completed native owner back into a live guest. */
internal object NativeApplicationTeardown {
    /** Retain the guest until native cleanup joins even if its transition waiter is cancelled. */
    suspend fun join(view: LeptosKitView?): Exception? = join { view?.closeAndJoin() }

    suspend fun join(operation: suspend () -> Unit): Exception? =
        withContext(NonCancellable) {
            try {
                operation()
                null
            } catch (error: Exception) {
                error
            }
        }

    fun retain(
        primary: Throwable,
        cleanup: Throwable?,
    ): Throwable {
        if (cleanup != null && cleanup !== primary && cleanup !in primary.suppressed) {
            primary.addSuppressed(cleanup)
        }

        return primary
    }

    fun message(error: Throwable): String {
        val diagnostics = listOf(error) + error.suppressed

        return diagnostics.joinToString("; ") { it.message ?: it.javaClass.simpleName }
    }
}
