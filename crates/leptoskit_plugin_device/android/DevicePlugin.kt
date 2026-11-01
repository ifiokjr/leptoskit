package dev.leptoskit.plugins.device

import android.app.ActivityManager
import android.app.UiModeManager
import android.content.Context
import android.content.res.Configuration
import android.os.Build
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder

class DevicePlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "device"
    override val methods = intArrayOf(STATUS)

    private companion object {
        /** The only method: this device's identity. */
        const val STATUS = 0

        /** The wire format version every reply carries. */
        const val VERSION_ONE = 1

        // / Header words: version, kind, simulator, and a 64-bit memory size.
        const val HEADER_BYTES = 20

        /** Room for the header and this answer's six short strings. */
        const val RESULT_CAPACITY = 128

        /** Longest string the wire format carries. */
        const val MAX_TEXT_BYTES = 4096

        /** The emulator flag's value when the hardware is an emulator. */
        const val SIMULATOR = 2
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (method != STATUS) throw PluginException(PluginException.UNAVAILABLE)

        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        return encode()
    }

    /** Packs the device's kind, memory, and identifying strings. */
    private fun encode(): ByteArray {
        val result = ByteArrayOutputStream(RESULT_CAPACITY)
        result.write(
            ByteBuffer
                .allocate(HEADER_BYTES)
                .order(ByteOrder.LITTLE_ENDIAN)
                .putInt(VERSION_ONE)
                .putInt(deviceKind())
                .putInt(simulatorFlag())
                .putLong(totalMemory())
                .array(),
        )

        for (value in listOf(
            Build.MANUFACTURER,
            Build.BRAND,
            Build.MODEL,
            "Android",
            Build.VERSION.RELEASE,
            Build.SUPPORTED_ABIS.firstOrNull(),
        )) {
            writeText(result, value)
        }

        return result.toByteArray()
    }

    /** The device's total memory, or zero when the activity service is missing. */
    private fun totalMemory(): Long {
        val manager = context.getSystemService(Context.ACTIVITY_SERVICE) as? ActivityManager ?: return 0L
        val info = ActivityManager.MemoryInfo()
        manager.getMemoryInfo(info)

        return info.totalMem.coerceAtLeast(0)
    }

    /** 2 when this build runs on an emulator, and 0 otherwise. */
    private fun simulatorFlag(): Int = if (Build.HARDWARE == "goldfish" || Build.HARDWARE == "ranchu") SIMULATOR else 0

    /** Writes one length-prefixed string, treating blank and unknown values as empty. */
    private fun writeText(
        result: ByteArrayOutputStream,
        value: String?,
    ) {
        val text = value?.takeUnless { it.isBlank() || it == Build.UNKNOWN } ?: ""
        val bytes = text.toByteArray(Charsets.UTF_8)

        if (bytes.size > MAX_TEXT_BYTES) throw PluginException(PluginException.TOO_LARGE)
        result.write(
            ByteBuffer
                .allocate(4)
                .order(ByteOrder.LITTLE_ENDIAN)
                .putInt(bytes.size)
                .array(),
        )
        result.write(bytes)
    }

    private fun deviceKind(): Int {
        val mode = context.getSystemService(Context.UI_MODE_SERVICE) as? UiModeManager

        return when (mode?.currentModeType) {
            Configuration.UI_MODE_TYPE_TELEVISION -> {
                3
            }

            Configuration.UI_MODE_TYPE_DESK -> {
                4
            }

            Configuration.UI_MODE_TYPE_WATCH -> {
                5
            }

            Configuration.UI_MODE_TYPE_CAR -> {
                6
            }

            Configuration.UI_MODE_TYPE_NORMAL -> {
                val width = context.resources.configuration.smallestScreenWidthDp
                when {
                    width >= 600 -> 2
                    width > 0 -> 1
                    else -> 0
                }
            }

            else -> {
                0
            }
        }
    }
}
