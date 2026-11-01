package dev.leptoskit.plugins.permissions

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.pm.PackageManager
import android.os.Looper
import dev.leptoskit.bridge.NativePermissionHost
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Queries never prompt. Requests require the generated host's permission-result broker. */
class PermissionsPlugin(
    context: Context,
) : NativePlugin {
    override val name = "permissions"
    override val methods = intArrayOf(0, 1)
    private val activity = context as? Activity
    private val host = context as? NativePermissionHost
    private var closed = false

    override fun close() {
        closed = true
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Permission dispatch requires the UI thread" }

        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)

        if (payload.remaining() != 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        val permission =
            when (payload.duplicate().order(ByteOrder.LITTLE_ENDIAN).int) {
                0 -> Manifest.permission.CAMERA
                1 -> Manifest.permission.RECORD_AUDIO
                else -> throw PluginException(PluginException.INVALID_ARGUMENT)
            }
        val activity = activity ?: throw PluginException(PluginException.UNAVAILABLE)

        if (activity.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED) return response(3, 1)

        if (method == 0) {
            return if (activity.shouldShowRequestPermissionRationale(permission)) response(2, 2) else response(0, 0)
        }

        // A missing manifest declaration cannot be repaired by a runtime prompt.
        @Suppress("DEPRECATION")
        val declared =
            activity.packageManager
                .getPackageInfo(activity.packageName, PackageManager.GET_PERMISSIONS)
                .requestedPermissions
                .orEmpty()

        if (permission !in declared) throw PluginException(PluginException.INVALID_ARGUMENT)
        val host = host ?: throw PluginException(PluginException.UNAVAILABLE)
        val result = host.requestNativePermissions(arrayOf(permission))

        if (closed) throw PluginException(PluginException.CANCELLED)

        if (result.size != 1) throw PluginException(PluginException.NATIVE)

        return if (result[0] == PackageManager.PERMISSION_GRANTED) {
            response(3, 1)
        } else {
            response(2, if (activity.shouldShowRequestPermissionRationale(permission)) 2 else 1)
        }
    }

    private fun response(
        status: Int,
        prompt: Int,
    ): ByteArray =
        ByteBuffer
            .allocate(8)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(status)
            .putInt(prompt)
            .array()
}
