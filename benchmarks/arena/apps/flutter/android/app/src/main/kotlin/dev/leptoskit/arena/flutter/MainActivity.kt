package dev.leptoskit.arena.flutter

import android.os.Process
import android.os.SystemClock
import android.util.Log
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {
    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        val messenger = flutterEngine.dartExecutor.binaryMessenger
        MethodChannel(messenger, "arena/host").setMethodCallHandler { call, result ->
            when (call.method) {
                "arguments" -> {
                    // `am start … --es scenario <id>` arrives as an intent extra.
                    val scenario = intent.getStringExtra("scenario")
                    result.success(if (scenario == null) emptyList() else listOf("--scenario", scenario))
                }
                "uptimeMs" -> {
                    val startNanos = Process.getStartElapsedRealtime() * 1_000_000L
                    result.success((SystemClock.elapsedRealtimeNanos() - startNanos) / 1e6)
                }
                "log" -> {
                    Log.i("ARENA", call.arguments as String)
                    result.success(null)
                }
                else -> result.notImplemented()
            }
        }
        // ArenaEcho, the async tier of the `echo` scenario.
        MethodChannel(messenger, "arena/echo").setMethodCallHandler { call, result ->
            when (call.method) {
                "echo" -> result.success(call.arguments)
                "addOne" -> {
                    val value = call.arguments as? Number
                    if (value == null) {
                        result.error("bad_argument", "addOne takes an integer", null)
                    } else {
                        result.success(value.toLong() + 1)
                    }
                }
                else -> result.notImplemented()
            }
        }
    }
}
