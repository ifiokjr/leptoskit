package dev.leptoskit.arena.lynx

import android.content.Context
import android.util.Log
import com.lynx.jsbridge.LynxMethod
import com.lynx.jsbridge.LynxModule

/** Writes report lines to logcat under the `ARENA` tag, where the harness reads them. */
class ArenaLogModule(context: Context) : LynxModule(context) {
	@LynxMethod
	fun log(line: String) {
		Log.i("ARENA", line)
	}
}
