package dev.leptoskit.plugins.devicepolicy

import android.app.admin.DeviceAdminReceiver
import android.content.Context
import android.content.Intent

/**
 * App-owned manifests must declare this receiver with `BIND_DEVICE_ADMIN` and
 * point its meta-data at an app-owned `@xml/device_admin` policy file. The
 * plugin never silently registers itself as a device owner.
 */
class DevicePolicyReceiver : DeviceAdminReceiver() {
    override fun onEnabled(
        context: Context,
        intent: Intent,
    ) {
        // Activation is observed through DevicePolicyManager.isAdminActive.
    }

    override fun onDisabled(
        context: Context,
        intent: Intent,
    ) {
        // Deactivation is observed through DevicePolicyManager.isAdminActive.
    }
}
