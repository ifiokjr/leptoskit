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
    override val methods = intArrayOf(0)

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (method != 0) throw PluginException(PluginException.UNAVAILABLE)

        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
        val manager = context.getSystemService(Context.ACTIVITY_SERVICE) as? ActivityManager
        val memory =
            manager?.let {
                val info = ActivityManager.MemoryInfo()
                it.getMemoryInfo(info)
                info.totalMem.coerceAtLeast(0)
            } ?: 0L
        val simulator = if (Build.HARDWARE == "goldfish" || Build.HARDWARE == "ranchu") 2 else 0
        val result = ByteArrayOutputStream(128)
        result.write(
            ByteBuffer
                .allocate(20)
                .order(ByteOrder.LITTLE_ENDIAN)
                .putInt(1)
                .putInt(deviceKind())
                .putInt(simulator)
                .putLong(memory)
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
            val text = value?.takeUnless { it.isBlank() || it == Build.UNKNOWN } ?: ""
            val bytes = text.toByteArray(Charsets.UTF_8)

            if (bytes.size > 4096) throw PluginException(PluginException.TOO_LARGE)
            result.write(
                ByteBuffer
                    .allocate(4)
                    .order(ByteOrder.LITTLE_ENDIAN)
                    .putInt(bytes.size)
                    .array(),
            )
            result.write(bytes)
        }

        return result.toByteArray()
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
