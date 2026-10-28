package dev.leptoskit.arena.lynx

import android.content.Context
import com.lynx.jsbridge.LynxMethod
import com.lynx.jsbridge.LynxModule
import com.lynx.react.bridge.Callback
import com.lynx.react.bridge.ReadableMap

/**
 * The arena's echo module. Lynx calls `@LynxMethod`s on the background (JS)
 * thread. The callback methods are the async tier: Lynx queues the callback
 * back to JavaScript as a separate task. The returning methods are the sync
 * tier: their result is the JavaScript call's return value.
 */
class ArenaEchoModule(context: Context) : LynxModule(context) {
	@LynxMethod
	fun echo(callback: Callback) = callback.invoke()

	@LynxMethod
	fun addOne(value: Double, callback: Callback) = callback.invoke(value + 1)

	@LynxMethod
	fun echoString(value: String, callback: Callback) = callback.invoke(value)

	@LynxMethod
	fun echoBytes(value: ByteArray, callback: Callback) = callback.invoke(value)

	@LynxMethod
	fun echoObject(value: ReadableMap, callback: Callback) = callback.invoke(value)

	@LynxMethod
	fun echoSync() = Unit

	@LynxMethod
	fun addOneSync(value: Double): Double = value + 1

	@LynxMethod
	fun echoStringSync(value: String): String = value

	@LynxMethod
	fun echoBytesSync(value: ByteArray): ByteArray = value

	@LynxMethod
	fun echoObjectSync(value: ReadableMap): ReadableMap = value
}
