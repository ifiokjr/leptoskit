package dev.leptoskit.plugins.location

import android.Manifest
import android.app.Activity
import android.app.Application
import android.content.Context
import android.content.pm.PackageManager
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import dev.leptoskit.bridge.NativePermissionHost
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.suspendCancellableCoroutine
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/** Foreground location only. Reads require existing permission and never prompt. */
class LocationPlugin(
    context: Context,
) : NativePlugin {
    override val name = "location"
    override val methods = intArrayOf(0, 1, 2, 3)
    private val appContext = context.applicationContext
    private val activity = context as? Activity
    private val permissionHost = context as? NativePermissionHost
    private val manager = context.getSystemService(Context.LOCATION_SERVICE) as? LocationManager
    private val handler = Handler(Looper.getMainLooper())
    private var pending: CancellableContinuation<ByteArray>? = null
    private var listener: LocationListener? = null
    private var timeout: Runnable? = null
    private var lifecycle: Application.ActivityLifecycleCallbacks? = null
    private var requesting = false
    private var closed = false

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Location dispatch requires the UI thread" }
        if (closed) throw PluginException(PluginException.CANCELLED)
        when (method) {
            0 -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
                return permission()
            }

            1 -> {
                if (payload.remaining() != 4) throw PluginException(PluginException.INVALID_ARGUMENT)
                val precise = payload.order(ByteOrder.LITTLE_ENDIAN).int
                if (precise !in 0..1) throw PluginException(PluginException.INVALID_ARGUMENT)
                return requestPermission(precise == 1)
            }

            2, 3 -> {
                val options = LocationOptions.decode(payload, method == 3)
                if (!granted(Manifest.permission.ACCESS_COARSE_LOCATION) && !granted(Manifest.permission.ACCESS_FINE_LOCATION)) {
                    throw PluginException(PluginException.PERMISSION_DENIED)
                }
                if (pending != null || requesting) throw PluginException(PluginException.BUSY)
                val activity = activity ?: throw PluginException(PluginException.UNAVAILABLE)
                // Foreground means visible, not focused: a resumed Activity can lack
                // focus in multi-window mode, under the notification shade, or just
                // after the permission dialog closes. Android hides the decor view
                // when the Activity stops, which the acquisition observer also ends on.
                if (activity.isFinishing || activity.isDestroyed || activity.window?.decorView?.isShown != true) {
                    throw PluginException(PluginException.CANCELLED)
                }
                return acquire(options)
            }

            else -> {
                throw PluginException(PluginException.UNAVAILABLE)
            }
        }
    }

    private fun granted(permission: String) = appContext.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED

    private fun permission(): ByteArray {
        val fine = granted(Manifest.permission.ACCESS_FINE_LOCATION)
        val coarse = granted(Manifest.permission.ACCESS_COARSE_LOCATION)
        val denied = activity?.shouldShowRequestPermissionRationale(Manifest.permission.ACCESS_COARSE_LOCATION) == true
        return ByteBuffer
            .allocate(8)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(
                if (fine || coarse) {
                    3
                } else if (denied) {
                    2
                } else {
                    0
                },
            ).putInt(
                if (fine) {
                    2
                } else if (coarse) {
                    1
                } else {
                    0
                },
            ).array()
    }

    private suspend fun requestPermission(precise: Boolean): ByteArray {
        val permissions =
            if (precise) {
                arrayOf(Manifest.permission.ACCESS_COARSE_LOCATION, Manifest.permission.ACCESS_FINE_LOCATION)
            } else {
                arrayOf(Manifest.permission.ACCESS_COARSE_LOCATION)
            }
        if (permissions.all(::granted)) return permission()
        if (pending != null || requesting) throw PluginException(PluginException.BUSY)
        val host = permissionHost ?: throw PluginException(PluginException.UNAVAILABLE)

        @Suppress("DEPRECATION")
        val declared =
            appContext.packageManager
                .getPackageInfo(
                    appContext.packageName,
                    PackageManager.GET_PERMISSIONS,
                ).requestedPermissions
                .orEmpty()
        if (permissions.any { it !in declared }) throw PluginException(PluginException.INVALID_ARGUMENT)
        requesting = true
        try {
            val result = host.requestNativePermissions(permissions)
            if (closed) throw PluginException(PluginException.CANCELLED)
            if (result.size != permissions.size) throw PluginException(PluginException.NATIVE)
            val response = permission()
            // A completed explicit denial is known even if rationale is unavailable.
            if (ByteBuffer.wrap(response).order(ByteOrder.LITTLE_ENDIAN).int == 0) response[0] = 2
            return response
        } finally {
            requesting = false
        }
    }

    private suspend fun acquire(options: LocationOptions): ByteArray {
        val manager = manager ?: throw PluginException(PluginException.UNAVAILABLE)
        val enabled =
            try {
                manager.getProviders(true)
            } catch (error: SecurityException) {
                throw PluginException(PluginException.PERMISSION_DENIED, error)
            }
        val providers =
            when {
                "fused" in enabled -> {
                    listOf("fused")
                }

                else -> {
                    buildList {
                        if (LocationManager.NETWORK_PROVIDER in enabled) add(LocationManager.NETWORK_PROVIDER)
                        val gpsAllowed =
                            granted(Manifest.permission.ACCESS_FINE_LOCATION) &&
                                LocationManager.GPS_PROVIDER in enabled && (options.highAccuracy || isEmpty())
                        if (gpsAllowed) add(LocationManager.GPS_PROVIDER)
                    }
                }
            }
        if (providers.isEmpty()) throw PluginException(PluginException.UNAVAILABLE)
        val started = SystemClock.elapsedRealtimeNanos()
        val preciseAtStart = granted(Manifest.permission.ACCESS_FINE_LOCATION)
        return suspendCancellableCoroutine { continuation ->
            pending = continuation
            val observer =
                object : LocationListener {
                    override fun onLocationChanged(location: Location) {
                        if (pending !== continuation || !continuation.isActive) return
                        val precise = granted(Manifest.permission.ACCESS_FINE_LOCATION)
                        val authorized = precise || granted(Manifest.permission.ACCESS_COARSE_LOCATION)
                        if (!permitsLocationDelivery(preciseAtStart, authorized, precise)) {
                            fail(continuation, PluginException(PluginException.PERMISSION_DENIED))
                            return
                        }
                        if (!acceptsLocation(location, options, started, SystemClock.elapsedRealtimeNanos())) return
                        try {
                            finish(continuation, encodeLocation(location))
                        } catch (error: PluginException) {
                            fail(continuation, error)
                        }
                    }

                    override fun onProviderEnabled(provider: String) = Unit

                    override fun onProviderDisabled(provider: String) {
                        if (pending !== continuation || !continuation.isActive) return
                        if (providers.none {
                                manager.isProviderEnabled(
                                    it,
                                )
                            }
                        ) {
                            fail(continuation, PluginException(PluginException.UNAVAILABLE))
                        }
                    }

                    @Deprecated("Required on older Android versions")
                    override fun onStatusChanged(
                        provider: String?,
                        status: Int,
                        extras: Bundle?,
                    ) = Unit
                }
            listener = observer
            continuation.invokeOnCancellation { handler.post { if (pending === continuation) cleanup() } }
            if (!continuation.isActive) {
                cleanup()
                return@suspendCancellableCoroutine
            }
            val deadline = Runnable { finish(continuation, byteArrayOf(1, 0, 0, 0)) }
            timeout = deadline
            handler.postDelayed(deadline, options.timeout.toLong())
            try {
                for (provider in providers) manager.requestLocationUpdates(provider, 0, 0f, observer, Looper.getMainLooper())
                observeBackground(continuation)
                if (options.maximumAge > 0) {
                    val cached = providers.mapNotNull(manager::getLastKnownLocation).maxByOrNull { it.time }
                    if (cached != null) observer.onLocationChanged(cached)
                }
            } catch (error: SecurityException) {
                fail(continuation, PluginException(PluginException.PERMISSION_DENIED, error))
            } catch (error: RuntimeException) {
                fail(continuation, PluginException(PluginException.NATIVE, error))
            }
        }
    }

    private fun observeBackground(continuation: CancellableContinuation<ByteArray>) {
        val activity = activity ?: return
        val observer =
            object : Application.ActivityLifecycleCallbacks {
                override fun onActivityStopped(current: Activity) {
                    if (current === activity && pending === continuation) {
                        cleanup()
                        continuation.cancel()
                    }
                }

                override fun onActivityCreated(
                    current: Activity,
                    state: Bundle?,
                ) = Unit

                override fun onActivityStarted(current: Activity) = Unit

                override fun onActivityResumed(current: Activity) = Unit

                override fun onActivityPaused(current: Activity) = Unit

                override fun onActivitySaveInstanceState(
                    current: Activity,
                    state: Bundle,
                ) = Unit

                override fun onActivityDestroyed(current: Activity) = onActivityStopped(current)
            }
        lifecycle = observer
        activity.application.registerActivityLifecycleCallbacks(observer)
    }

    override fun close() {
        closed = true
        val continuation = pending
        cleanup()
        continuation?.cancel()
    }

    private fun finish(
        continuation: CancellableContinuation<ByteArray>,
        bytes: ByteArray,
    ) {
        if (pending !== continuation) return
        cleanup()
        if (continuation.isActive) continuation.resume(bytes)
    }

    private fun fail(
        continuation: CancellableContinuation<ByteArray>,
        error: PluginException,
    ) {
        if (pending !== continuation) return
        cleanup()
        if (continuation.isActive) continuation.resumeWithException(error)
    }

    private fun cleanup() {
        pending = null
        listener?.let { manager?.removeUpdates(it) }
        listener = null
        timeout?.let(handler::removeCallbacks)
        timeout = null
        lifecycle?.let { activity?.application?.unregisterActivityLifecycleCallbacks(it) }
        lifecycle = null
    }
}

/** A queued precise fix cannot cross an authorization downgrade. */
internal fun permitsLocationDelivery(
    startedPrecise: Boolean,
    authorized: Boolean,
    precise: Boolean,
): Boolean = authorized && (!startedPrecise || precise)

internal data class LocationOptions(
    val highAccuracy: Boolean,
    val timeout: Int,
    val maximumAge: Int,
    val after: Double?,
) {
    companion object {
        fun decode(
            payload: ByteBuffer,
            hasPrevious: Boolean,
        ): LocationOptions {
            if (payload.remaining() != if (hasPrevious) 20 else 12) throw PluginException(PluginException.INVALID_ARGUMENT)
            val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
            val accuracy = input.int
            val timeout = input.int
            val age = input.int
            val previous = if (hasPrevious) input.double else null
            if (accuracy !in 0..1 || timeout !in 1..120_000 || age !in 0..3_600_000 ||
                previous?.let { !it.isFinite() || it < 0 } == true
            ) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }
            return LocationOptions(accuracy == 1, timeout, age, previous)
        }
    }
}

/** Freshness uses the device monotonic clock, independently of wall-clock corrections. */
internal fun acceptsLocation(
    location: Location,
    options: LocationOptions,
    started: Long,
    now: Long,
): Boolean {
    val oldest = if (options.maximumAge == 0) started else now - options.maximumAge.toLong() * 1_000_000
    val measured = location.elapsedRealtimeNanos
    return measured > 0 && measured >= oldest && measured <= now &&
        (options.after?.let { location.time.toDouble() > it } ?: true)
}

/** Pure coordinate conversion; tests use synthetic Location objects without reading hardware. */
internal fun encodeLocation(location: Location): ByteArray {
    val optional =
        arrayOf(
            if (location.hasAltitude()) location.altitude else null,
            if (Build.VERSION.SDK_INT >= 26 && location.hasVerticalAccuracy()) location.verticalAccuracyMeters.toDouble() else null,
            if (location.hasBearing()) location.bearing.toDouble() else null,
            if (location.hasSpeed()) location.speed.toDouble() else null,
        )
    val values =
        doubleArrayOf(location.time.toDouble(), location.latitude, location.longitude, location.accuracy.toDouble()) +
            optional.map { it ?: 0.0 }.toDoubleArray()
    if (!location.hasAccuracy() || values.any { !it.isFinite() } || values[0] < 0 || values[1] !in -90.0..90.0 ||
        values[2] !in -180.0..180.0 || values[3] < 0 || values[5] < 0 || values[6] < 0 || values[6] >= 360 || values[7] < 0
    ) {
        throw PluginException(PluginException.NATIVE)
    }
    var mask = 0
    optional.forEachIndexed { index, value -> if (value != null) mask = mask or (1 shl index) }
    val provider =
        when (location.provider) {
            LocationManager.GPS_PROVIDER -> 1
            LocationManager.NETWORK_PROVIDER -> 2
            LocationManager.PASSIVE_PROVIDER -> 3
            "fused" -> 4
            else -> 0
        }

    @Suppress("DEPRECATION")
    val mock = if (Build.VERSION.SDK_INT >= 31) location.isMock else location.isFromMockProvider
    val bytes =
        ByteBuffer
            .allocate(84)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(0)
            .putInt(mask)
            .putInt(provider)
            .putInt(if (mock) 2 else 1)
            .putInt(0)
    values.forEach(bytes::putDouble)
    return bytes.array()
}
