package dev.leptoskit.arena.lynx

import android.app.Activity
import android.os.Build
import android.os.Bundle
import android.os.Process
import android.os.SystemClock
import android.view.ViewGroup
import android.view.WindowInsets
import android.widget.FrameLayout
import com.lynx.tasm.LynxView
import com.lynx.tasm.LynxViewBuilder
import com.lynx.tasm.TemplateData

/**
 * Hosts one LynxView filling the safe area and renders the bundled
 * `main.lynx.bundle`. Launch data (`--es scenario <id>`, optionally
 * `--es lane tuned`) reaches the bundle as global props.
 */
class MainActivity : Activity() {
	private var lynxView: LynxView? = null

	override fun onCreate(savedInstanceState: Bundle?) {
		super.onCreate(savedInstanceState)
		val container = FrameLayout(this)
		container.setBackgroundColor(0xFFFFFFFF.toInt())
		container.setOnApplyWindowInsetsListener { view, insets ->
			if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
				val safe = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout())
				view.setPadding(safe.left, safe.top, safe.right, safe.bottom)
			} else {
				@Suppress("DEPRECATION")
				view.setPadding(
					insets.systemWindowInsetLeft,
					insets.systemWindowInsetTop,
					insets.systemWindowInsetRight,
					insets.systemWindowInsetBottom,
				)
			}
			insets
		}

		val builder = LynxViewBuilder()
		builder.setTemplateProvider(AssetTemplateProvider(this))
		builder.addBehavior(InputBehavior())
		builder.registerModule("ArenaEcho", ArenaEchoModule::class.java)
		builder.registerModule("ArenaLog", ArenaLogModule::class.java)
		val view = builder.build(this)
		view.setGlobalProps(TemplateData.fromMap(globalProps()))
		container.addView(
			view,
			FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT),
		)
		setContentView(container)
		lynxView = view
		view.renderTemplateUrl(BUNDLE, "")
	}

	override fun onDestroy() {
		lynxView?.destroy()
		lynxView = null
		super.onDestroy()
	}

	private fun globalProps(): Map<String, Any> {
		val props = mutableMapOf<String, Any>(
			"platform" to "android",
			"frameworkVersion" to BuildConfig.LYNX_VERSION,
			"refreshRate" to refreshRate().toDouble(),
			"processStartUnixMs" to processStartUnixMs(),
		)
		intent.getStringExtra("scenario")?.let { props["scenario"] = it }
		intent.getStringExtra("lane")?.let { props["lane"] = it }
		return props
	}

	private fun refreshRate(): Float =
		if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
			display?.refreshRate ?: 60f
		} else {
			@Suppress("DEPRECATION")
			windowManager.defaultDisplay.refreshRate
		}

	/** The process start as a Unix timestamp in milliseconds, comparable with Lynx's timing entries. */
	private fun processStartUnixMs(): Double {
		val sinceStart = SystemClock.elapsedRealtime() - Process.getStartElapsedRealtime()
		return (System.currentTimeMillis() - sinceStart).toDouble()
	}

	private companion object {
		const val BUNDLE = "main.lynx.bundle"
	}
}
