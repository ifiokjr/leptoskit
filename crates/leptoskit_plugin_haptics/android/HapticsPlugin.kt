package dev.leptoskit.plugins.haptics

import android.app.Activity
import android.content.Context
import android.os.Build
import android.os.Looper
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import android.view.HapticFeedbackConstants as Feedback

/** Uses the view's haptic API without overriding the user's feedback settings. */
class HapticsPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "haptics"
    override val methods = intArrayOf(0, 1, 2, 3)

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Haptics requires the UI thread" }
        val activity = context as? Activity ?: throw PluginException(PluginException.UNAVAILABLE)
        val effect =
            when (method) {
                0 -> {
                    if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
                    Feedback.CLOCK_TICK
                }

                1 -> {
                    impact(value(payload))
                }

                2 -> {
                    notification(value(payload))
                }

                3 -> {
                    androidEffect(value(payload))
                }

                else -> {
                    throw PluginException(PluginException.UNAVAILABLE)
                }
            }
        activity.window.decorView.performHapticFeedback(effect)

        return byteArrayOf()
    }

    private fun value(payload: ByteBuffer): Int {
        if (payload.remaining() != 4) throw PluginException(PluginException.INVALID_ARGUMENT)

        return payload.order(ByteOrder.LITTLE_ENDIAN).int
    }

    private fun impact(style: Int): Int =
        when (style) {
            0 -> Feedback.KEYBOARD_TAP
            1 -> Feedback.VIRTUAL_KEY
            2 -> Feedback.LONG_PRESS
            3 -> Feedback.CONTEXT_CLICK
            4 -> Feedback.CLOCK_TICK
            else -> throw PluginException(PluginException.INVALID_ARGUMENT)
        }

    private fun notification(kind: Int): Int =
        when (kind) {
            0 -> if (Build.VERSION.SDK_INT >= 30) Feedback.CONFIRM else Feedback.VIRTUAL_KEY
            1 -> Feedback.LONG_PRESS

            2 -> if (Build.VERSION.SDK_INT >= 30) Feedback.REJECT else Feedback.LONG_PRESS
            else -> throw PluginException(PluginException.INVALID_ARGUMENT)
        }

    private fun androidEffect(effect: Int): Int {
        val minimum =
            when (effect) {
                Feedback.LONG_PRESS, Feedback.VIRTUAL_KEY, Feedback.KEYBOARD_TAP,
                Feedback.CLOCK_TICK, Feedback.CONTEXT_CLICK,
                -> 26

                Feedback.KEYBOARD_RELEASE, Feedback.VIRTUAL_KEY_RELEASE, Feedback.TEXT_HANDLE_MOVE -> 27

                Feedback.GESTURE_START, Feedback.GESTURE_END, Feedback.CONFIRM, Feedback.REJECT -> 30

                Feedback.TOGGLE_ON, Feedback.TOGGLE_OFF, Feedback.GESTURE_THRESHOLD_ACTIVATE,
                Feedback.GESTURE_THRESHOLD_DEACTIVATE, Feedback.DRAG_START, Feedback.SEGMENT_TICK,
                Feedback.SEGMENT_FREQUENT_TICK, Feedback.NO_HAPTICS,
                -> 34

                else -> throw PluginException(PluginException.INVALID_ARGUMENT)
            }

        if (Build.VERSION.SDK_INT < minimum) throw PluginException(PluginException.UNAVAILABLE)

        return effect
    }
}
