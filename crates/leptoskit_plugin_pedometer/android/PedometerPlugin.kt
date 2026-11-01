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

        if (method == 0) return sensor.invoke(0, payload)

        if (method == 1) return byteArrayOf(status().toByte(), 0, 0, 0)

        if (method == 4) throw PluginException(PluginException.UNAVAILABLE)

        if (busy) throw PluginException(PluginException.BUSY)
        busy = true
        try {
            if (method == 2) {
                if (granted()) return byteArrayOf(3, 0, 0, 0)
                val declared =
                    context.packageManager
                        .getPackageInfo(
                            context.packageName,
                            PackageManager.GET_PERMISSIONS,
                        ).requestedPermissions
                if (declared?.contains(Manifest.permission.ACTIVITY_RECOGNITION) !=
                    true
                ) {
                    throw PluginException(PluginException.PERMISSION_DENIED)
                }
                val host = context as? NativePermissionHost ?: throw PluginException(PluginException.UNAVAILABLE)
                permissionJob = currentCoroutineContext()[Job]
                val result =
                    withTimeoutOrNull(30_000) { host.requestNativePermissions(arrayOf(Manifest.permission.ACTIVITY_RECOGNITION)) }
                        ?: throw PluginException(PluginException.UNAVAILABLE)

                if (result.size != 1) throw PluginException(PluginException.NATIVE)

                if (closed) throw PluginException(PluginException.CANCELLED)

                return byteArrayOf(if (granted()) 3 else 2, 0, 0, 0)
            }

            if (payload.remaining() != 4) throw PluginException(PluginException.INVALID_ARGUMENT)
            val timeout = payload.order(ByteOrder.LITTLE_ENDIAN).int
            val request =
                ByteBuffer
                    .allocate(8)
                    .order(ByteOrder.LITTLE_ENDIAN)
                    .putInt(200)
                    .putInt(timeout)
            request.flip()

            return sensor.invoke(1, request)
        } finally {
            permissionJob = null
            busy = false
        }
    }

    override fun close() {
        closed = true
        permissionJob?.cancel()
        sensor.close()
    }
}
