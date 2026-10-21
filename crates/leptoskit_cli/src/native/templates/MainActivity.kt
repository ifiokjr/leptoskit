package dev.leptoskit.generated

import android.app.Activity
import android.os.Bundle
import dev.leptoskit.bridge.NativePermissionBroker
import dev.leptoskit.bridge.NativePermissionHost
import dev.leptoskit.mobile.GeneratedPlugins
import dev.leptoskit.mobile.LeptosKitApplicationView

class MainActivity :
    Activity(),
    NativePermissionHost {
    private var content: LeptosKitApplicationView? = null
    private val permissions by lazy { NativePermissionBroker(this) }

    override suspend fun requestNativePermissions(permissions: Array<String>): IntArray = this.permissions.request(permissions)

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray,
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        this.permissions.onResult(requestCode, permissions, grantResults)
    }

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        permissions.restoreState(state)
        val embedded = assets.open("app.lkb").use { it.readBytes() }
        val configuration = assets.open("codepush.json").use { it.readBytes() }
        content =
            LeptosKitApplicationView(this, embedded, configuration) { bridge, dispatcher ->
                GeneratedPlugins.register(this, bridge, dispatcher)
            }
        setContentView(content)
    }

    override fun onSaveInstanceState(state: Bundle) {
        permissions.saveState(state)
        super.onSaveInstanceState(state)
    }

    override fun onResume() {
        super.onResume()
        permissions.setActive(true)
    }

    override fun onPause() {
        permissions.setActive(false)
        super.onPause()
    }

    override fun onDestroy() {
        permissions.close()
        content?.close()
        content = null
        super.onDestroy()
    }
}
