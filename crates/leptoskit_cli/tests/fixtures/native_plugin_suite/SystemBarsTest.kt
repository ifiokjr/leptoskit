package dev.leptoskit.generated

import android.app.Activity
import android.os.Build
import android.view.View
import android.view.WindowInsetsController
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Test

class SystemBarsTest {
    @Test
    @Suppress("DEPRECATION")
    fun lightThemeUsesDarkSystemIcons() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val intent = requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName))
        ActivityScenario.launch<Activity>(intent).use { scenario ->
            scenario.onActivity { activity ->
                if (Build.VERSION.SDK_INT >= 30) {
                    val appearance = requireNotNull(activity.window.insetsController).systemBarsAppearance
                    val expected =
                        WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS or
                            WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS
                    assertEquals(expected, appearance and expected)
                } else {
                    val expected = View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR or View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR
                    assertEquals(expected, activity.window.decorView.systemUiVisibility and expected)
                }
            }
        }
    }
}
