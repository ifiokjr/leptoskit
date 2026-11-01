package dev.leptoskit.plugins.application

import android.content.Context
import android.content.pm.PackageInfo
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
    override val methods = intArrayOf(STATUS)

    private companion object {
        /** Reports the installed application's identity and version. */
        const val STATUS = 0

        /** The API level where package info takes flags rather than flags ints. */
        const val FLAGS_API = 33

        /** The API level where the version code grew to 64 bits. */
        const val LONG_VERSION_API = 28

        /** Longest string the wire format carries. */
        const val MAX_TEXT_BYTES = 4096

        /** Room for the header and this answer's four short strings. */
        const val RESULT_CAPACITY = 128

        /** Version-one success, which every reply starts with. */
        val SUCCESS = byteArrayOf(1, 0, 0, 0)
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (method != STATUS) throw PluginException(PluginException.UNAVAILABLE)

        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        return encode()
    }

    /** Packs the identity this build reports: package, label, version, and code. */
    private fun encode(): ByteArray {
        val manager = context.packageManager
        val info = packageInfo(manager)
        val fields =
            listOf(
                context.packageName,
                manager.getApplicationLabel(context.applicationInfo).toString(),
                info.versionName,
                versionCode(info).toString(),
            )
        val result = ByteArrayOutputStream(RESULT_CAPACITY)
        result.write(SUCCESS)

        for (value in fields) {
            writeString(result, value ?: "")
        }

        return result.toByteArray()
    }

    /** The installed package's info, read the way its API level allows. */
    private fun packageInfo(manager: PackageManager): PackageInfo =
        try {
            if (Build.VERSION.SDK_INT >= FLAGS_API) {
                manager.getPackageInfo(context.packageName, PackageManager.PackageInfoFlags.of(0))
            } else {
                @Suppress("DEPRECATION")
                manager.getPackageInfo(context.packageName, 0)
            }
        } catch (error: PackageManager.NameNotFoundException) {
            throw PluginException(PluginException.NATIVE, error)
        }

    /** The version code, which grew to a long in API 28. */
    private fun versionCode(info: PackageInfo): Long =
        if (Build.VERSION.SDK_INT >= LONG_VERSION_API) {
            info.longVersionCode
        } else {
            @Suppress("DEPRECATION")
            info.versionCode.toLong()
        }

    /** Writes one length-prefixed UTF-8 string. */
    private fun writeString(
        result: ByteArrayOutputStream,
        value: String,
    ) {
        val text = value.toByteArray(Charsets.UTF_8)

        if (text.size > MAX_TEXT_BYTES) throw PluginException(PluginException.TOO_LARGE)
        result.write(
            ByteBuffer
                .allocate(4)
                .order(ByteOrder.LITTLE_ENDIAN)
                .putInt(text.size)
                .array(),
        )
        result.write(text)
    }
}
