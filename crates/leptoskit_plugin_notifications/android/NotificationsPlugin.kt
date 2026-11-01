package dev.leptoskit.plugins.notifications

import android.Manifest
import android.app.Activity
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.content.pm.PackageManager
import android.media.AudioAttributes
import android.media.RingtoneManager
import android.os.Build
import android.os.Looper
import dev.leptoskit.bridge.NativePermissionHost
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction

/** Immediate notifications only. Posting never implicitly requests permission. */
class NotificationsPlugin(
    private val context: Context,
    private val smallIcon: Int = android.R.drawable.ic_dialog_info,
) : NativePlugin {
    override val name = "notifications"
    override val methods = intArrayOf(STATE, REQUEST, SHOW, CANCEL, CHANNEL, DELETE, ACTIVE)

    private companion object {
        /** Reports the current permission, without asking for it. */
        const val STATE = 0

        /** Reports the permission, asking for it when it can. */
        const val REQUEST = 1

        /** Posts a notification to a configured channel. */
        const val SHOW = 2

        /** Cancels one posted notification. */
        const val CANCEL = 3

        /** Creates or replaces a channel. */
        const val CHANNEL = 4

        /** Deletes one channel. */
        const val DELETE = 5

        /** Lists the plugin's posted notifications. */
        const val ACTIVE = 6

        /** The largest show request: an id, title, body, and channel. */
        const val SHOW_BYTES = 4496

        /** The largest cancel or delete request: one identifier. */
        const val ID_BYTES = 68

        /** The largest channel request. */
        const val CHANNEL_BYTES = 1236

        /** Longest title and body the wire format carries. */
        const val MAX_TITLE = 256
        const val MAX_BODY = 4096
    }

    private val manager = context.getSystemService(NotificationManager::class.java)
    private var closed = false

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper())

        if (closed) throw PluginException(PluginException.CANCELLED)
        currentCoroutineContext().ensureActive()
        val manager = manager ?: throw PluginException(PluginException.UNAVAILABLE)

        return dispatch(manager, method, payload)
    }

    /** Runs one method, and answers with the bytes it returns. */
    private suspend fun dispatch(
        manager: NotificationManager,
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        when (method) {
            STATE, REQUEST -> return permissionReply(manager, method, payload)
            SHOW -> show(manager, payload)
            CANCEL, DELETE -> cancelOrDelete(manager, method, payload)
            CHANNEL -> configure(Reader(payload, CHANNEL_BYTES), manager)
            ACTIVE -> active(manager, payload)
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }

        return byteArrayOf()
    }

    /** Reports the permission, asking for it when the caller asked to. */
    private suspend fun permissionReply(
        manager: NotificationManager,
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        empty(payload)

        if (method == REQUEST) requestPermission()

        return ByteBuffer
            .allocate(4)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(permission())
            .array()
    }

    /** Posts one notification to a channel the app configured. */
    private fun show(
        manager: NotificationManager,
        payload: ByteBuffer,
    ) {
        val input = Reader(payload, SHOW_BYTES)
        val id = input.identifier()
        val title = input.text(MAX_TITLE, false, true)
        val body = input.text(MAX_BODY, true, false)
        val channel = input.identifier()
        input.finish()

        if (permission() != GRANTED) throw PluginException(PluginException.PERMISSION_DENIED)
        val configured =
            manager.getNotificationChannel(PREFIX + channel)
                ?: throw PluginException(PluginException.INVALID_ARGUMENT)

        if (configured.importance == NotificationManager.IMPORTANCE_NONE) {
            throw PluginException(PluginException.PERMISSION_DENIED)
        }

        val notification =
            Notification
                .Builder(context, configured.id)
                .setSmallIcon(smallIcon)
                .setContentTitle(title)
                .setContentText(body)
                .setStyle(Notification.BigTextStyle().bigText(body))
                .setAutoCancel(true)
                .build()

        try {
            manager.notify(PREFIX + id, 0, notification)
        } catch (_: SecurityException) {
            throw PluginException(PluginException.PERMISSION_DENIED)
        }
    }

    /** Cancels one posted notification, or deletes one channel. */
    private fun cancelOrDelete(
        manager: NotificationManager,
        method: Int,
        payload: ByteBuffer,
    ) {
        val input = Reader(payload, ID_BYTES)
        val id = input.identifier()
        input.finish()

        if (method == CANCEL) manager.cancel(PREFIX + id, 0) else manager.deleteNotificationChannel(PREFIX + id)
    }

    /** Cancels every notification this plugin posted. */
    private fun active(
        manager: NotificationManager,
        payload: ByteBuffer,
    ) {
        empty(payload)

        for (notification in manager.activeNotifications) {
            if (notification.tag?.startsWith(PREFIX) == true) manager.cancel(notification.tag, notification.id)
        }
    }

    override fun close() {
        closed = true
    }

    private fun permission(): Int {
        val manager = manager ?: return 0
        if (Build.VERSION.SDK_INT >= 33 &&
            context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            val activity = context as? Activity

            return if (activity?.shouldShowRequestPermissionRationale(Manifest.permission.POST_NOTIFICATIONS) == true) 2 else 0
        }

        return if (manager.areNotificationsEnabled()) 3 else 2
    }

    private suspend fun requestPermission() {
        if (Build.VERSION.SDK_INT < 33 || permission() == 3) return
        @Suppress("DEPRECATION")
        val declarations =
            context.packageManager
                .getPackageInfo(
                    context.packageName,
                    PackageManager.GET_PERMISSIONS,
                ).requestedPermissions
                .orEmpty()

        if (Manifest.permission.POST_NOTIFICATIONS !in declarations) throw PluginException(PluginException.INVALID_ARGUMENT)
        val host = context as? NativePermissionHost ?: throw PluginException(PluginException.UNAVAILABLE)
        val results = host.requestNativePermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS))

        if (closed) throw PluginException(PluginException.CANCELLED)

        currentCoroutineContext().ensureActive()

        if (results.size != 1) throw PluginException(PluginException.NATIVE)
    }

    private fun configure(
        input: Reader,
        manager: NotificationManager,
    ) {
        val id = input.identifier()
        val title = input.text(128, false, true)
        val description = input.text(1024, true, false)
        val importance =
            when (input.number()) {
                1 -> NotificationManager.IMPORTANCE_LOW
                2 -> NotificationManager.IMPORTANCE_DEFAULT
                3 -> NotificationManager.IMPORTANCE_HIGH
                else -> throw PluginException(PluginException.INVALID_ARGUMENT)
            }
        val sound =
            when (input.number()) {
                0 -> false
                1 -> true
                else -> throw PluginException(PluginException.INVALID_ARGUMENT)
            }
        input.finish()
        val channel = NotificationChannel(PREFIX + id, title, importance)
        channel.description = description
        channel.setSound(
            if (sound) RingtoneManager.getDefaultUri(RingtoneManager.TYPE_NOTIFICATION) else null,
            AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_NOTIFICATION).build(),
        )
        manager.createNotificationChannel(channel)
    }

    private fun empty(payload: ByteBuffer) {
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
    }

    companion object {
        internal const val PREFIX = "leptoskit.notifications."

        /** The permission value that means notifications may be posted. */
        internal const val GRANTED = 3
    }
}

private class Reader(
    payload: ByteBuffer,
    maximum: Int,
) {
    private val data = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)

    init {
        if (data.remaining() > maximum) throw PluginException(PluginException.INVALID_ARGUMENT)
    }

    fun number(): Int {
        if (data.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)

        return data.int
    }

    fun identifier(): String {
        val value = text(64, false, true)

        if (!value.all { it in 'a'..'z' || it in 'A'..'Z' || it in '0'..'9' || it in "._-" }) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        return value
    }

    fun text(
        maximum: Int,
        multiline: Boolean,
        required: Boolean,
    ): String {
        val length = number()

        if (length !in 0..maximum || length > data.remaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
        val field = data.slice().apply { limit(length) }
        data.position(data.position() + length)
        val value =
            try {
                Charsets.UTF_8
                    .newDecoder()
                    .onMalformedInput(
                        CodingErrorAction.REPORT,
                    ).onUnmappableCharacter(CodingErrorAction.REPORT)
                    .decode(field)
                    .toString()
            } catch (_: CharacterCodingException) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }

        if ((required && value.isBlank()) || value.any { it.isISOControl() && !(multiline && (it == '\n' || it == '\t')) }) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        return value
    }

    fun finish() {
        if (data.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
    }
}
