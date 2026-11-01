package dev.leptoskit.plugins.application

import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder

class ApplicationPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "application"
    override val methods = intArrayOf(0)

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (method != 0) throw PluginException(PluginException.UNAVAILABLE)

        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
        val manager = context.packageManager
        val info =
            try {
                if (Build.VERSION.SDK_INT >= 33) {
                    manager.getPackageInfo(context.packageName, PackageManager.PackageInfoFlags.of(0))
                } else {
                    @Suppress("DEPRECATION")
                    manager.getPackageInfo(context.packageName, 0)
                }
            } catch (error: PackageManager.NameNotFoundException) {
                throw PluginException(PluginException.NATIVE, error)
            }
        val versionCode =
            if (Build.VERSION.SDK_INT >= 28) {
                info.longVersionCode
            } else {
                @Suppress("DEPRECATION")
                info.versionCode.toLong()
            }
        val fields =
            listOf(
                context.packageName,
                manager.getApplicationLabel(context.applicationInfo).toString(),
                info.versionName,
                versionCode.toString(),
            )
        val result = ByteArrayOutputStream(128)
        result.write(byteArrayOf(1, 0, 0, 0))

        for (value in fields) {
            val text = (value ?: "").toByteArray(Charsets.UTF_8)

            if (text.size > 4096) throw PluginException(PluginException.TOO_LARGE)
            result.write(
                ByteBuffer
                    .allocate(4)
                    .order(ByteOrder.LITTLE_ENDIAN)
                    .putInt(text.size)
                    .array(),
            )
            result.write(text)
        }

        return result.toByteArray()
    }
}
