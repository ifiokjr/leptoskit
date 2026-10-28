package dev.leptoskit.arena.lynx

import android.content.Context
import com.lynx.tasm.provider.AbsTemplateProvider
import java.io.IOException
import java.util.concurrent.Executors

/** Loads bundles from the APK's assets: the arena ships its bundle in the app, with no dev server. */
class AssetTemplateProvider(context: Context) : AbsTemplateProvider() {
	private val assets = context.applicationContext.assets
	private val loader = Executors.newSingleThreadExecutor()

	override fun loadTemplate(uri: String, callback: Callback) {
		loader.execute {
			try {
				callback.onSuccess(assets.open(uri).use { it.readBytes() })
			} catch (error: IOException) {
				callback.onFailed("cannot read asset $uri: ${error.message}")
			}
		}
	}
}
