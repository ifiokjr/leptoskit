package dev.leptoskit.plugins.pedometer

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.pm.PackageManager
import android.hardware.Sensor
import android.os.Build
import dev.leptoskit.bridge.NativePermissionHost
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import dev.leptoskit.sensors.MotionSensorPlugin
import kotlinx.coroutines.Job
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.withTimeoutOrNull
import java.nio.ByteBuffer
import java.nio.ByteOrder

class PedometerPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "pedometer"
    override val methods = intArrayOf(0, 1, 2, 3, 4)
    private var closed = false
    private var busy = false
    private var permissionJob: Job? = null
    private val sensor =
        MotionSensorPlugin(context, name, Sensor.TYPE_STEP_COUNTER, bounded = true) {
            if (!granted()) throw PluginException(PluginException.PERMISSION_DENIED)
        }

    private fun granted() =
        Build.VERSION.SDK_INT < 29 ||
            context.checkSelfPermission(Manifest.permission.ACTIVITY_RECOGNITION) == PackageManager.PERMISSION_GRANTED

    private fun status(): Int =
        if (granted()) {
            3
        } else if ((context as? Activity)?.shouldShowRequestPermissionRationale(Manifest.permission.ACTIVITY_RECOGNITION) ==
            true
        ) {
            1
        } else {
            0
        }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)

        if (method <= 2 && payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        return when (method) {
            0 -> sensor.invoke(0, payload)
            1 -> byteArrayOf(status().toByte(), 0, 0, 0)
            2 -> exclusive { requestPermission() }
            3 -> exclusive { start(payload) }
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }
    }

    /** Runs one request at a time, so a second request reports busy. */
    private suspend fun exclusive(operation: suspend () -> ByteArray): ByteArray {
        if (busy) throw PluginException(PluginException.BUSY)

        busy = true
        try {
            return operation()
        } finally {
            permissionJob = null
            busy = false
        }
    }

    /** Asks the system for the activity permission and maps the answer. */
    private suspend fun requestPermission(): ByteArray {
        if (granted()) return byteArrayOf(3, 0, 0, 0)

        if (!declaresPermission()) throw PluginException(PluginException.PERMISSION_DENIED)

        if (askHost() != 1) throw PluginException(PluginException.NATIVE)

        if (closed) throw PluginException(PluginException.CANCELLED)

        return byteArrayOf(if (granted()) 3 else 2, 0, 0, 0)
    }

    /** Shows the system prompt through the host. Says how many answers came back. */
    private suspend fun askHost(): Int {
        val host = context as? NativePermissionHost ?: throw PluginException(PluginException.UNAVAILABLE)
        permissionJob = currentCoroutineContext()[Job]

        val result =
            withTimeoutOrNull(PERMISSION_TIMEOUT_MS) {
                host.requestNativePermissions(arrayOf(Manifest.permission.ACTIVITY_RECOGNITION))
            } ?: throw PluginException(PluginException.UNAVAILABLE)

        return result.size
    }

    /** Whether the manifest asks for the activity permission at all. */
    private fun declaresPermission(): Boolean =
        context.packageManager
            .getPackageInfo(context.packageName, PackageManager.GET_PERMISSIONS)
            .requestedPermissions
            ?.contains(Manifest.permission.ACTIVITY_RECOGNITION) == true

    /** Starts the step sensor with the timeout the caller gave, in milliseconds. */
    private suspend fun start(payload: ByteBuffer): ByteArray {
        if (payload.remaining() != 4) throw PluginException(PluginException.INVALID_ARGUMENT)

        val timeout = payload.order(ByteOrder.LITTLE_ENDIAN).int
        val request =
            ByteBuffer
                .allocate(8)
                .order(ByteOrder.LITTLE_ENDIAN)
                .putInt(STEP_COUNTER_START)
                .putInt(timeout)
        request.flip()

        return sensor.invoke(1, request)
    }

    override fun close() {
        closed = true
        permissionJob?.cancel()
        sensor.close()
    }

    private companion object {
        /** How long the system may take to answer a permission request. */
        const val PERMISSION_TIMEOUT_MS = 30_000L

        /** The sensor method that starts counting steps. */
        const val STEP_COUNTER_START = 200
    }
}
