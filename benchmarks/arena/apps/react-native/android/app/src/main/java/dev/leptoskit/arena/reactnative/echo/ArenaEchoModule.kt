package dev.leptoskit.arena.reactnative.echo

import android.util.Base64
import android.util.Log
import com.facebook.react.bridge.Arguments
import com.facebook.react.bridge.ArrayBuffer
import com.facebook.react.bridge.Promise
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.bridge.ReadableMap
import com.facebook.react.bridge.WritableMap

/**
 * The arena's echo module. Promise methods are the async tier; the `Sync`
 * methods are synchronous TurboModule methods, the sync tier.
 */
class ArenaEchoModule(reactContext: ReactApplicationContext) : NativeArenaEchoSpec(reactContext) {

  override fun echo(promise: Promise) {
    promise.resolve(null)
  }

  override fun addOne(value: Double, promise: Promise) {
    promise.resolve(value + 1)
  }

  override fun echoString(value: String, promise: Promise) {
    promise.resolve(value)
  }

  /** Decodes to bytes and encodes again, so the bytes really cross into Kotlin. */
  override fun echoBase64(value: String, promise: Promise) {
    val bytes = Base64.decode(value, Base64.NO_WRAP)
    promise.resolve(Base64.encodeToString(bytes, Base64.NO_WRAP))
  }

  override fun echoObject(value: ReadableMap, promise: Promise) {
    promise.resolve(copy(value))
  }

  override fun echoSync(): Double? = null

  override fun addOneSync(value: Double): Double = value + 1

  override fun echoStringSync(value: String): String = value

  /** The argument borrows the JS buffer; returning it gives JS a copy. */
  override fun echoBytesSync(value: ArrayBuffer): ArrayBuffer = value

  override fun echoObjectSync(value: ReadableMap): WritableMap = copy(value)

  override fun log(line: String) {
    Log.i(TAG, line)
  }

  private fun copy(value: ReadableMap): WritableMap = Arguments.createMap().apply { merge(value) }

  companion object {
    const val NAME = NativeArenaEchoSpec.NAME
    private const val TAG = "ARENA"
  }
}
