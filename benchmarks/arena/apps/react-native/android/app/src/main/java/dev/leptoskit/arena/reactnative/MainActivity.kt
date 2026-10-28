package dev.leptoskit.arena.reactnative

import android.os.Bundle
import com.facebook.react.ReactActivity
import com.facebook.react.ReactActivityDelegate
import com.facebook.react.defaults.DefaultReactActivityDelegate

class MainActivity : ReactActivity() {

  override fun getMainComponentName(): String = "ArenaReactNative"

  /** Passes the `scenario` intent extra to the root component as its `scenario` prop. */
  override fun createReactActivityDelegate(): ReactActivityDelegate =
    object : DefaultReactActivityDelegate(this, mainComponentName) {
      override fun getLaunchOptions(): Bundle? =
        intent.getStringExtra(SCENARIO_EXTRA)?.let { scenario ->
          Bundle().apply { putString(SCENARIO_EXTRA, scenario) }
        }
    }

  private companion object {
    const val SCENARIO_EXTRA = "scenario"
  }
}
