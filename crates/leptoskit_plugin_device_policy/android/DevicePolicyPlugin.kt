package dev.leptoskit.plugins.devicepolicy

import android.app.Activity
import android.app.admin.DevicePolicyManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Looper
import dev.leptoskit.bridge.NativeActivityResultHost
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction

/**
 * Android Device Policy Manager only. Status never prompts. Privileged calls
 * require an active device admin the user enabled through the system UI. Wipe
 * requires the explicit confirmation token from the Rust crate.
 */
class DevicePolicyPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "device-policy"
    override val methods =
        intArrayOf(
            STATUS,
            REQUEST_ACTIVATION,
            LOCK_NOW,
            SET_PASSWORD_QUALITY,
            SET_PASSWORD_MINIMUM_LENGTH,
            SET_CAMERA_DISABLED,
            WIPE_DATA,
            REMOVE_ACTIVE,
        )
    private val activity = context as? Activity
    private val host = context as? NativeActivityResultHost
    private val policy =
        context.getSystemService(Context.DEVICE_POLICY_SERVICE) as? DevicePolicyManager
    private val component = ComponentName(context, DevicePolicyReceiver::class.java)
    private var closed = false
    private var busy = false

    override fun close() {
        closed = true
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper()) {
            "Device policy dispatch requires the UI thread"
        }

        if (closed) throw PluginException(PluginException.CANCELLED)

        return when (method) {
            STATUS -> status(payload)
            REQUEST_ACTIVATION -> requestActivation(payload)
            LOCK_NOW -> lockNow(payload)
            SET_PASSWORD_QUALITY -> setPasswordQuality(payload)
            SET_PASSWORD_MINIMUM_LENGTH -> setPasswordMinimumLength(payload)
            SET_CAMERA_DISABLED -> setCameraDisabled(payload)
            WIPE_DATA -> wipeData(payload)
            REMOVE_ACTIVE -> removeActive(payload)
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }
    }

    /** Observes admin state without presenting system UI. */
    private fun status(payload: ByteBuffer): ByteArray {
        requireEmpty(payload)
        val active = isActive()
        val available = receiverDeclared()
        val state =
            when {
                !available -> STATE_UNAVAILABLE
                active -> STATE_ACTIVE
                else -> STATE_INACTIVE
            }
        val capabilities = if (available) CAPABILITIES_STANDARD else CAPABILITIES_NONE
        val camera =
            if (active) {
                optionalFlag(policy!!.getCameraDisabled(component))
            } else {
                0
            }
        val quality =
            if (active) {
                qualityWire(policy!!.getPasswordQuality(component)) + 1
            } else {
                0
            }
        val minimum =
            if (active) {
                policy!!.getPasswordMinimumLength(component)
            } else {
                LENGTH_UNKNOWN
            }

        return words(1, state, capabilities, camera, quality, minimum)
    }

    /** Opens Android's device-admin consent UI after an explicit caller request. */
    private suspend fun requestActivation(payload: ByteBuffer): ByteArray {
        if (isActive()) return words(OUTCOME_ALREADY_ACTIVE)

        val explanation = explanationOf(payload)
        if (activity == null) throw PluginException(PluginException.UNAVAILABLE)
        val host = host ?: throw PluginException(PluginException.UNAVAILABLE)
        val policy = policy ?: throw PluginException(PluginException.UNAVAILABLE)

        if (!receiverDeclared()) throw PluginException(PluginException.UNAVAILABLE)

        if (busy) throw PluginException(PluginException.BUSY)
        busy = true

        return try {
            val intent =
                Intent(DevicePolicyManager.ACTION_ADD_DEVICE_ADMIN)
                    .putExtra(DevicePolicyManager.EXTRA_DEVICE_ADMIN, component)
                    .putExtra(DevicePolicyManager.EXTRA_ADD_EXPLANATION, explanation)
            val result = host.launchPluginActivity(intent)

            if (closed) throw PluginException(PluginException.CANCELLED)

            when {
                policy.isAdminActive(component) -> words(OUTCOME_ACTIVATED)
                result.resultCode == Activity.RESULT_CANCELED -> words(OUTCOME_DECLINED)
                else -> words(OUTCOME_DECLINED)
            }
        } finally {
            busy = false
        }
    }

    private fun lockNow(payload: ByteBuffer): ByteArray {
        requireEmpty(payload)
        requireActive().lockNow()
        return ByteArray(0)
    }

    private fun setPasswordQuality(payload: ByteBuffer): ByteArray {
        val quality = qualityOf(payload)
        try {
            requireActive().setPasswordQuality(component, quality)
        } catch (_: SecurityException) {
            throw PluginException(PluginException.PERMISSION_DENIED)
        }
        return ByteArray(0)
    }

    private fun setPasswordMinimumLength(payload: ByteBuffer): ByteArray {
        val length = word(payload)
        if (length == LENGTH_UNKNOWN) throw PluginException(PluginException.INVALID_ARGUMENT)
        try {
            requireActive().setPasswordMinimumLength(component, length)
        } catch (_: SecurityException) {
            throw PluginException(PluginException.PERMISSION_DENIED)
        }
        return ByteArray(0)
    }

    private fun setCameraDisabled(payload: ByteBuffer): ByteArray {
        val disabled = flag(word(payload))
        try {
            requireActive().setCameraDisabled(component, disabled)
        } catch (_: SecurityException) {
            throw PluginException(PluginException.PERMISSION_DENIED)
        }
        return ByteArray(0)
    }

    private fun wipeData(payload: ByteBuffer): ByteArray {
        val flags = wipeFlags(payload)
        try {
            requireActive().wipeData(flags)
        } catch (_: SecurityException) {
            throw PluginException(PluginException.PERMISSION_DENIED)
        }
        return ByteArray(0)
    }

    private fun removeActive(payload: ByteBuffer): ByteArray {
        requireEmpty(payload)
        val policy = requireActive()
        try {
            policy.removeActiveAdmin(component)
        } catch (_: SecurityException) {
            throw PluginException(PluginException.PERMISSION_DENIED)
        }
        return ByteArray(0)
    }

    private fun requireActive(): DevicePolicyManager {
        val policy = policy ?: throw PluginException(PluginException.UNAVAILABLE)
        if (!policy.isAdminActive(component)) throw PluginException(PluginException.PERMISSION_DENIED)
        return policy
    }

    private fun isActive(): Boolean = policy?.isAdminActive(component) == true

    private fun receiverDeclared(): Boolean {
        return try {
            context.packageManager.getReceiverInfo(component, PackageManager.GET_META_DATA)
            true
        } catch (_: PackageManager.NameNotFoundException) {
            false
        }
    }

    private fun explanationOf(payload: ByteBuffer): String {
        val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (input.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        val length = input.int
        if (length !in 1..MAX_EXPLANATION_BYTES || input.remaining() != length) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
        val bytes = ByteArray(length)
        input.get(bytes)
        val text =
            try {
                Charsets.UTF_8
                    .newDecoder()
                    .onMalformedInput(CodingErrorAction.REPORT)
                    .onUnmappableCharacter(CodingErrorAction.REPORT)
                    .decode(ByteBuffer.wrap(bytes))
                    .toString()
            } catch (_: CharacterCodingException) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }
        if (text.trim().isEmpty() || text.contains('\u0000')) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
        return text
    }

    private fun wipeFlags(payload: ByteBuffer): Int {
        val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        if (input.remaining() != 12) throw PluginException(PluginException.INVALID_ARGUMENT)
        if (input.int != 1) throw PluginException(PluginException.INVALID_ARGUMENT)
        if (input.int != WIPE_CONFIRMATION_TOKEN) throw PluginException(PluginException.INVALID_ARGUMENT)
        val flags = input.int
        if (flags and WIPE_FLAG_EXTERNAL_STORAGE.inv() != 0) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
        return if (flags and WIPE_FLAG_EXTERNAL_STORAGE != 0) {
            DevicePolicyManager.WIPE_EXTERNAL_STORAGE
        } else {
            0
        }
    }

    private fun qualityOf(payload: ByteBuffer): Int =
        when (word(payload)) {
            QUALITY_UNSPECIFIED -> DevicePolicyManager.PASSWORD_QUALITY_UNSPECIFIED
            QUALITY_SOMETHING -> DevicePolicyManager.PASSWORD_QUALITY_SOMETHING
            QUALITY_NUMERIC -> DevicePolicyManager.PASSWORD_QUALITY_NUMERIC
            QUALITY_ALPHABETIC -> DevicePolicyManager.PASSWORD_QUALITY_ALPHABETIC
            QUALITY_ALPHANUMERIC -> DevicePolicyManager.PASSWORD_QUALITY_ALPHANUMERIC
            QUALITY_COMPLEX -> DevicePolicyManager.PASSWORD_QUALITY_COMPLEX
            else -> throw PluginException(PluginException.INVALID_ARGUMENT)
        }

    private fun qualityWire(quality: Int): Int =
        when (quality) {
            DevicePolicyManager.PASSWORD_QUALITY_UNSPECIFIED -> QUALITY_UNSPECIFIED
            DevicePolicyManager.PASSWORD_QUALITY_SOMETHING -> QUALITY_SOMETHING
            DevicePolicyManager.PASSWORD_QUALITY_NUMERIC,
            DevicePolicyManager.PASSWORD_QUALITY_NUMERIC_COMPLEX,
            -> QUALITY_NUMERIC
            DevicePolicyManager.PASSWORD_QUALITY_ALPHABETIC -> QUALITY_ALPHABETIC
            DevicePolicyManager.PASSWORD_QUALITY_ALPHANUMERIC -> QUALITY_ALPHANUMERIC
            DevicePolicyManager.PASSWORD_QUALITY_COMPLEX -> QUALITY_COMPLEX
            else -> QUALITY_UNSPECIFIED
        }

    private fun requireEmpty(payload: ByteBuffer) {
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
    }

    private fun word(payload: ByteBuffer): Int {
        if (payload.remaining() != 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        return payload.duplicate().order(ByteOrder.LITTLE_ENDIAN).int
    }

    private fun flag(value: Int): Boolean =
        when (value) {
            0 -> false
            1 -> true
            else -> throw PluginException(PluginException.INVALID_ARGUMENT)
        }

    private fun optionalFlag(value: Boolean): Int = if (value) 2 else 1

    private fun words(vararg values: Int): ByteArray {
        val buffer = ByteBuffer.allocate(values.size * 4).order(ByteOrder.LITTLE_ENDIAN)
        values.forEach(buffer::putInt)
        return buffer.array()
    }

    private companion object {
        const val STATUS = 0
        const val REQUEST_ACTIVATION = 1
        const val LOCK_NOW = 2
        const val SET_PASSWORD_QUALITY = 3
        const val SET_PASSWORD_MINIMUM_LENGTH = 4
        const val SET_CAMERA_DISABLED = 5
        const val WIPE_DATA = 6
        const val REMOVE_ACTIVE = 7
        const val STATE_INACTIVE = 0
        const val STATE_ACTIVE = 1
        const val STATE_UNAVAILABLE = 2
        const val OUTCOME_ALREADY_ACTIVE = 0
        const val OUTCOME_ACTIVATED = 1
        const val OUTCOME_DECLINED = 2
        const val QUALITY_UNSPECIFIED = 0
        const val QUALITY_SOMETHING = 1
        const val QUALITY_NUMERIC = 2
        const val QUALITY_ALPHABETIC = 3
        const val QUALITY_ALPHANUMERIC = 4
        const val QUALITY_COMPLEX = 5
        const val CAPABILITIES_NONE = 0
        const val CAPABILITIES_STANDARD = 0b11111
        const val WIPE_CONFIRMATION_TOKEN = 0x57495045
        const val WIPE_FLAG_EXTERNAL_STORAGE = 1
        const val MAX_EXPLANATION_BYTES = 512
        const val LENGTH_UNKNOWN = -1
    }
}
