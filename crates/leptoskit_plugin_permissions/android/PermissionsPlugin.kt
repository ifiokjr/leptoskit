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
    override val methods = intArrayOf(STATUS, REQUEST)
    private val activity = context as? Activity
    private val host = context as? NativePermissionHost
    private var closed = false

    override fun close() {
        closed = true
    }

    /**
     * Answers one call on the UI thread, because the OS permission APIs require
     * it. A granted permission answers both methods directly; otherwise a
     * request goes through the host's broker.
     */
    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Permission dispatch requires the UI thread" }

        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)

        val permission = permissionOf(kindOf(payload))
        val activity = activity ?: throw PluginException(PluginException.UNAVAILABLE)

        if (activity.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED) {
            return response(STATUS_GRANTED, PROMPT_BLOCKED)
        }

        return when (method) {
            STATUS -> ungrantedStatus(activity, permission)
            REQUEST -> request(activity, permission)
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }
    }

    /** Reads the permission kind the caller asked about. */
    private fun kindOf(payload: ByteBuffer): Int {
        if (payload.remaining() != KIND_BYTES) throw PluginException(PluginException.INVALID_ARGUMENT)

        return payload.duplicate().order(ByteOrder.LITTLE_ENDIAN).int
    }

    /** Maps a wire kind onto the manifest permission it stands for. */
    private fun permissionOf(kind: Int): String =
        when (kind) {
            CAMERA_KIND -> Manifest.permission.CAMERA
            MICROPHONE_KIND -> Manifest.permission.RECORD_AUDIO
            else -> throw PluginException(PluginException.INVALID_ARGUMENT)
        }

    /**
     * Reports a permission the OS has not granted. Android cannot tell a
     * never-requested permission from a permanent denial, so an unaskable
     * permission is unknown rather than denied.
     */
    private fun ungrantedStatus(
        activity: Activity,
        permission: String,
    ): ByteArray =
        if (activity.shouldShowRequestPermissionRationale(permission)) {
            response(STATUS_DENIED, PROMPT_AVAILABLE)
        } else {
            response(STATUS_UNKNOWN, PROMPT_UNKNOWN)
        }

    /** Asks the OS through the host's broker, then re-reads the resulting state. */
    private suspend fun request(
        activity: Activity,
        permission: String,
    ): ByteArray {
        // A missing manifest declaration cannot be repaired by a runtime prompt.
        if (!declares(activity, permission)) throw PluginException(PluginException.INVALID_ARGUMENT)

        val host = host ?: throw PluginException(PluginException.UNAVAILABLE)
        val result = host.requestNativePermissions(arrayOf(permission))

        if (closed) throw PluginException(PluginException.CANCELLED)

        if (result.size != 1) throw PluginException(PluginException.NATIVE)

        return requestOutcome(activity, permission, result[0])
    }

    /** Maps the broker's answer onto a state and whether another prompt is possible. */
    private fun requestOutcome(
        activity: Activity,
        permission: String,
        grant: Int,
    ): ByteArray =
        when {
            grant == PackageManager.PERMISSION_GRANTED -> response(STATUS_GRANTED, PROMPT_BLOCKED)
            activity.shouldShowRequestPermissionRationale(permission) -> response(STATUS_DENIED, PROMPT_AVAILABLE)
            else -> response(STATUS_DENIED, PROMPT_BLOCKED)
        }

    /** Whether the manifest declares the permission at all. */
    @Suppress("DEPRECATION")
    private fun declares(
        activity: Activity,
        permission: String,
    ): Boolean =
        activity.packageManager
            .getPackageInfo(activity.packageName, PackageManager.GET_PERMISSIONS)
            .requestedPermissions
            .orEmpty()
            .contains(permission)

    private fun response(
        status: Int,
        prompt: Int,
    ): ByteArray =
        ByteBuffer
            .allocate(RESPONSE_BYTES)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(status)
            .putInt(prompt)
            .array()

    private companion object {
        /** Method that reports the current state without prompting. */
        const val STATUS = 0

        /** Method that may show the system prompt. */
        const val REQUEST = 1

        /** Wire value for the camera permission. */
        const val CAMERA_KIND = 0

        /** Wire value for the microphone permission. */
        const val MICROPHONE_KIND = 1

        /** States the wire format reserves for the status field. */
        const val STATUS_UNKNOWN = 0
        const val STATUS_DENIED = 2
        const val STATUS_GRANTED = 3

        /** Whether a prompt can still appear for the permission. */
        const val PROMPT_UNKNOWN = 0
        const val PROMPT_BLOCKED = 1
        const val PROMPT_AVAILABLE = 2

        /** A kind is one little-endian u32. */
        const val KIND_BYTES = 4

        /** Two u32 fields: state and can-prompt. */
        const val RESPONSE_BYTES = 8
    }
}
