package dev.leptoskit.bridge

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.os.Bundle
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/** Synthetic external activities: these tests never present a picker or access user files. */
class ActivityResultBrokerTest {
    private class RecordingActivity : Activity() {
        val codes = mutableListOf<Int>()
        var unavailable = false

        @Deprecated("Synthetic platform callback")
        override fun startActivityForResult(
            intent: Intent?,
            requestCode: Int,
        ) {
            if (unavailable) throw ActivityNotFoundException("Synthetic unavailable activity")
            codes.add(requestCode)
        }
    }

    @Test
    fun cancelledPresentationRetainsSlotUntilItsResultAndLateResultCannotMatchNextCaller() =
        runBlocking(Dispatchers.Main) {
            withTimeout(5_000) {
                val activity = RecordingActivity()
                val broker = NativeActivityResultBroker(activity)
                broker.setActive(true)
                val first = async(start = CoroutineStart.UNDISPATCHED) { broker.launch(Intent("test.first")) }
                val firstCode = activity.codes.single()
                first.cancelAndJoin()
                rejects(PluginException.BUSY) { broker.launch(Intent("test.overlap")) }
                assertTrue(broker.onResult(firstCode, Activity.RESULT_CANCELED, null))
                val next = async(start = CoroutineStart.UNDISPATCHED) { broker.launch(Intent("test.next")) }
                val nextCode = activity.codes.last()
                assertTrue(nextCode > firstCode)
                assertFalse(broker.onResult(firstCode, Activity.RESULT_OK, Intent("test.stale")))
                assertFalse(next.isCompleted)
                val data = Intent("test.result")
                assertTrue(broker.onResult(nextCode, Activity.RESULT_OK, data))
                assertEquals(NativeActivityResult(Activity.RESULT_OK, data), next.await())
                assertFalse(broker.onResult(nextCode, Activity.RESULT_OK, data))
                broker.close()
            }
        }

    @Test
    fun pauseAllowsPendingResultButPreventsNewLaunchAndDestroyCancelsTheCaller() =
        runBlocking(Dispatchers.Main) {
            withTimeout(5_000) {
                val activity = RecordingActivity()
                val broker = NativeActivityResultBroker(activity)
                rejects(PluginException.BUSY) { broker.launch(Intent("test.inactive")) }
                broker.setActive(true)
                val first = async(start = CoroutineStart.UNDISPATCHED) { broker.launch(Intent("test.external")) }
                broker.setActive(false)
                assertTrue(broker.onResult(activity.codes.single(), Activity.RESULT_CANCELED, null))
                assertEquals(Activity.RESULT_CANCELED, first.await().resultCode)
                rejects(PluginException.BUSY) { broker.launch(Intent("test.paused")) }
                broker.setActive(true)
                val pending = async(start = CoroutineStart.UNDISPATCHED) { broker.launch(Intent("test.destroy")) }
                broker.close()
                pending.join()
                assertTrue(pending.isCancelled)
                assertFalse(broker.onResult(activity.codes.last(), Activity.RESULT_OK, null))
                rejects(PluginException.CANCELLED) { broker.launch(Intent("test.closed")) }
                broker.close()
            }
        }

    @Test
    fun failedLaunchReleasesSlotAndSavedWatermarkSeparatesRecreatedHosts() =
        runBlocking(Dispatchers.Main) {
            withTimeout(5_000) {
                val activity = RecordingActivity()
                val broker = NativeActivityResultBroker(activity)
                broker.setActive(true)
                activity.unavailable = true
                rejects(PluginException.UNAVAILABLE) { broker.launch(Intent("test.missing")) }
                activity.unavailable = false
                val first = async(start = CoroutineStart.UNDISPATCHED) { broker.launch(Intent("test.first")) }
                val oldCode = activity.codes.single()
                val state = Bundle()
                broker.saveState(state)
                broker.close()
                first.join()
                val recreated = NativeActivityResultBroker(activity)
                recreated.restoreState(state)
                recreated.setActive(true)
                val next = async(start = CoroutineStart.UNDISPATCHED) { recreated.launch(Intent("test.next")) }
                val code = activity.codes.last()
                assertTrue(code > oldCode)
                assertTrue(code in 0x1000..0x4BFF)
                assertFalse(recreated.onResult(oldCode, Activity.RESULT_OK, null))
                assertTrue(recreated.onResult(code, Activity.RESULT_CANCELED, null))
                next.await()
                recreated.close()
            }
        }

    private suspend fun rejects(
        code: Int,
        action: suspend () -> Unit,
    ) {
        try {
            action()
            fail("Expected plugin error $code")
        } catch (error: PluginException) {
            assertEquals(code, error.code)
        }
    }
}
