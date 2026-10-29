package dev.leptoskit.arena.lynx

import android.app.Application
import com.lynx.tasm.LynxEnv

/** Initialises the Lynx engine once per process, before any LynxView exists. */
class ArenaApplication : Application() {
	override fun onCreate() {
		super.onCreate()
		LynxEnv.inst().init(this, null, null, null)
	}
}
