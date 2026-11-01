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
    override val methods = intArrayOf(STATUS, REQUEST, LOCATE, LOCATE_AFTER)
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

        return when (method) {
            STATUS -> reportedScope(payload)
            REQUEST -> requestPermission(wantsPrecise(payload))
            LOCATE -> acquire(LocationOptions.decode(payload, hasPrevious = false))
            LOCATE_AFTER -> acquire(LocationOptions.decode(payload, hasPrevious = true))
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }
    }

    /** Reports the existing permission scope, which never prompts. */
    private fun reportedScope(payload: ByteBuffer): ByteArray {
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        return permission()
    }

    /** Reads whether the caller wants precise or approximate access. */
    private fun wantsPrecise(payload: ByteBuffer): Boolean {
        if (payload.remaining() != PRECISION_BYTES) throw PluginException(PluginException.INVALID_ARGUMENT)

        val precise = payload.order(ByteOrder.LITTLE_ENDIAN).int

        if (precise != APPROXIMATE && precise != PRECISE) throw PluginException(PluginException.INVALID_ARGUMENT)

        return precise == PRECISE
    }

    /** Whether the OS already granted one scope. */
    private fun granted(permission: String): Boolean = appContext.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED

    /** The granted scope and how precise it is, without prompting. */
    private fun permission(): ByteArray {
        val fine = granted(Manifest.permission.ACCESS_FINE_LOCATION)
        val coarse = granted(Manifest.permission.ACCESS_COARSE_LOCATION)
        val denied = activity?.shouldShowRequestPermissionRationale(Manifest.permission.ACCESS_COARSE_LOCATION) == true

        return ByteBuffer
            .allocate(SCOPE_BYTES)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(scopeStatus(fine, coarse, denied))
            .putInt(scopePrecision(fine, coarse))
            .array()
    }

    /** Access the OS has granted, or the state a prompt could still change. */
    private fun scopeStatus(
        fine: Boolean,
        coarse: Boolean,
        denied: Boolean,
    ): Int =
        when {
            fine || coarse -> STATUS_GRANTED
            denied -> STATUS_DENIED
            else -> STATUS_UNKNOWN
        }

    /** How precise the granted access is: both scopes, coarse only, or neither. */
    private fun scopePrecision(
        fine: Boolean,
        coarse: Boolean,
    ): Int =
        when {
            fine -> PRECISION_FINE
            coarse -> PRECISION_COARSE
            else -> PRECISION_NONE
        }

    /** Asks the host to prompt for the scopes the caller needs. */
    private suspend fun requestPermission(precise: Boolean): ByteArray {
        val permissions = scopeRequests(precise)

        if (permissions.all(::granted)) return permission()

        if (pending != null || requesting) throw PluginException(PluginException.BUSY)
        val host = permissionHost ?: throw PluginException(PluginException.UNAVAILABLE)
        requireDeclared(permissions)
        requesting = true

        try {
            return awaitPrompt(host, permissions)
        } finally {
            requesting = false
        }
    }

    /** A precise request needs both scopes; an approximate one needs only the coarse scope. */
    private fun scopeRequests(precise: Boolean): Array<String> {
        if (precise) {
            return arrayOf(Manifest.permission.ACCESS_COARSE_LOCATION, Manifest.permission.ACCESS_FINE_LOCATION)
        }

        return arrayOf(Manifest.permission.ACCESS_COARSE_LOCATION)
    }

    /** Whether the manifest declares every permission a prompt would ask for. */
    @Suppress("DEPRECATION")
    private fun requireDeclared(permissions: Array<String>) {
        val declared =
            appContext.packageManager
                .getPackageInfo(appContext.packageName, PackageManager.GET_PERMISSIONS)
                .requestedPermissions
                .orEmpty()

        if (permissions.any { it !in declared }) throw PluginException(PluginException.INVALID_ARGUMENT)
    }

    /** Waits for the host's answer and reports the scope it left behind. */
    private suspend fun awaitPrompt(
        host: NativePermissionHost,
        permissions: Array<String>,
    ): ByteArray {
        val result = host.requestNativePermissions(permissions)

        if (closed) throw PluginException(PluginException.CANCELLED)

        if (result.size != permissions.size) throw PluginException(PluginException.NATIVE)

        return knownDenial(permission())
    }

    /** Marks an answer as denied when the completion proves denial, not mere silence. */
    private fun knownDenial(response: ByteArray): ByteArray {
        val status = ByteBuffer.wrap(response).order(ByteOrder.LITTLE_ENDIAN).int

        if (status == STATUS_UNKNOWN) response[0] = STATUS_DENIED.toByte()

        return response
    }

    /** Waits for one fix from the best provider the device can offer. */
    private suspend fun acquire(options: LocationOptions): ByteArray {
        val manager = manager ?: throw PluginException(PluginException.UNAVAILABLE)
        val selected = providers(manager, options)
        val started = SystemClock.elapsedRealtimeNanos()

        return suspendCancellableCoroutine { continuation ->
            pending = continuation
            val observer = FixListener(continuation, options, selected, started)
            listener = observer

            continuation.invokeOnCancellation { handler.post { if (pending === continuation) cleanup() } }

            if (!continuation.isActive) {
                cleanup()

                return@suspendCancellableCoroutine
            }

            val deadline = Runnable { finish(continuation, byteArrayOf(1, 0, 0, 0)) }
            timeout = deadline
            handler.postDelayed(deadline, options.timeout.toLong())
            listen(continuation, manager, observer)
        }
    }

    /** Starts the updates, ending the acquisition when the OS refuses them. */
    private fun listen(
        continuation: CancellableContinuation<ByteArray>,
        manager: LocationManager,
        observer: FixListener,
    ) {
        try {
            observer.start(manager)
            observeBackground(continuation)
            observer.replayCached(manager)
        } catch (error: SecurityException) {
            fail(continuation, PluginException(PluginException.PERMISSION_DENIED, error))
        } catch (error: RuntimeException) {
            fail(continuation, PluginException(PluginException.NATIVE, error))
        }
    }

    /** The providers to ask, best first, or an error when none can answer. */
    private fun providers(
        manager: LocationManager,
        options: LocationOptions,
    ): List<String> {
        val enabled = enabledProviders(manager)

        if (FUSED in enabled) return listOf(FUSED)

        val selected = platformProviders(enabled, options)

        if (selected.isEmpty()) throw PluginException(PluginException.UNAVAILABLE)

        return selected
    }

    /** The platform providers to ask, best first. */
    private fun platformProviders(
        enabled: List<String>,
        options: LocationOptions,
    ): MutableList<String> {
        val selected = mutableListOf<String>()

        if (LocationManager.NETWORK_PROVIDER in enabled) selected += LocationManager.NETWORK_PROVIDER

        if (options.highAccuracy || selected.isEmpty()) addGps(selected, enabled)

        return selected
    }

    /** GPS is a fallback for coarse-only devices, and needs fine access to answer. */
    private fun addGps(
        selected: MutableList<String>,
        enabled: List<String>,
    ) {
        if (!canUseGps(enabled)) return

        selected += LocationManager.GPS_PROVIDER
    }

    /** Enabled providers; the OS reports a missing permission as a security failure. */
    private fun enabledProviders(manager: LocationManager): List<String> =
        try {
            manager.getProviders(true)
        } catch (error: SecurityException) {
            throw PluginException(PluginException.PERMISSION_DENIED, error)
        }

    /** GPS fixes need fine access and an enabled GPS provider. */
    private fun canUseGps(enabled: List<String>): Boolean =
        granted(Manifest.permission.ACCESS_FINE_LOCATION) && LocationManager.GPS_PROVIDER in enabled

    /** Ends the acquisition when the activity that started it stops. */
    private fun observeBackground(continuation: CancellableContinuation<ByteArray>) {
        val activity = activity ?: return
        val observer = StopObserver(continuation, activity)
        lifecycle = observer
        activity.application.registerActivityLifecycleCallbacks(observer)
    }

    /** Answers with the fix, or reports why it cannot be encoded. */
    private fun deliver(
        continuation: CancellableContinuation<ByteArray>,
        location: Location,
    ) {
        try {
            finish(continuation, encodeLocation(location))
        } catch (error: PluginException) {
            fail(continuation, error)
        }
    }

    /** Delivers the first fix the OS reports for one acquisition. */
    private inner class FixListener(
        private val continuation: CancellableContinuation<ByteArray>,
        private val options: LocationOptions,
        private val providers: List<String>,
        private val started: Long,
    ) : LocationListener {
        private val preciseAtStart = granted(Manifest.permission.ACCESS_FINE_LOCATION)

        /** Asks every selected provider for updates. */
        fun start(manager: LocationManager) {
            for (provider in providers) {
                manager.requestLocationUpdates(provider, 0, 0f, this, Looper.getMainLooper())
            }
        }

        /** Answers with a cached fix straight away when the caller accepts one. */
        fun replayCached(manager: LocationManager) {
            if (options.maximumAge == 0) return

            providers.mapNotNull(manager::getLastKnownLocation).maxByOrNull { it.time }?.let(::onLocationChanged)
        }

        // / Delivers the first fix that passes the permission and freshness checks.
        override fun onLocationChanged(location: Location) {
            if (pending !== continuation || !continuation.isActive) return

            val precise = granted(Manifest.permission.ACCESS_FINE_LOCATION)
            val authorized = precise || granted(Manifest.permission.ACCESS_COARSE_LOCATION)

            if (!permitsLocationDelivery(preciseAtStart, authorized, precise)) {
                fail(continuation, PluginException(PluginException.PERMISSION_DENIED))

                return
            }

            if (!acceptsLocation(location, options, started, SystemClock.elapsedRealtimeNanos())) return

            deliver(continuation, location)
        }

        override fun onProviderEnabled(provider: String) {}

        override fun onProviderDisabled(provider: String) {
            if (pending !== continuation || !continuation.isActive) return
            if (providers.none(::enabled)) fail(continuation, PluginException(PluginException.UNAVAILABLE))
        }

        @Deprecated("Required on older Android versions")
        override fun onStatusChanged(
            provider: String?,
            status: Int,
            extras: Bundle?,
        ) {}

        /** Whether one of the requested providers can still answer. */
        private fun enabled(provider: String): Boolean = manager?.isProviderEnabled(provider) == true
    }

    /** Ends the acquisition when the activity that started it stops. */
    private inner class StopObserver(
        private val continuation: CancellableContinuation<ByteArray>,
        private val owner: Activity,
    ) : Application.ActivityLifecycleCallbacks {
        override fun onActivityStopped(current: Activity) {
            if (current !== owner || pending !== continuation) return
            cleanup()
            continuation.cancel()
        }

        override fun onActivityCreated(
            current: Activity,
            state: Bundle?,
        ) {}

        override fun onActivityStarted(current: Activity) {}

        override fun onActivityResumed(current: Activity) {}

        override fun onActivityPaused(current: Activity) {}

        override fun onActivitySaveInstanceState(
            current: Activity,
            state: Bundle,
        ) {}

        override fun onActivityDestroyed(current: Activity) {
            onActivityStopped(current)
        }
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

    private companion object {
        /** Methods the Rust crate calls. */
        const val STATUS = 0
        const val REQUEST = 1
        const val LOCATE = 2
        const val LOCATE_AFTER = 3

        /** A permission request carries one little-endian precision flag. */
        const val PRECISION_BYTES = 4
        const val APPROXIMATE = 0
        const val PRECISE = 1

        /** A scope reply is two little-endian u32 fields. */
        const val SCOPE_BYTES = 8

        /** Granted access, and whether a prompt could still change it. */
        const val STATUS_UNKNOWN = 0
        const val STATUS_DENIED = 2
        const val STATUS_GRANTED = 3

        /** How much of the location the OS exposes. */
        const val PRECISION_NONE = 0
        const val PRECISION_COARSE = 1
        const val PRECISION_FINE = 2
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
        /** Decodes the bounds a fix request carries, with or without a follow-up cursor. */
        fun decode(
            payload: ByteBuffer,
            hasPrevious: Boolean,
        ): LocationOptions {
            val expected = if (hasPrevious) WITH_PREVIOUS_BYTES else BOUNDS_BYTES

            if (payload.remaining() != expected) throw PluginException(PluginException.INVALID_ARGUMENT)

            val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
            val accuracy = input.int
            val timeout = input.int
            val age = input.int
            val previous = if (hasPrevious) input.double else null
            requireBounds(accuracy, timeout, age, previous)

            return LocationOptions(accuracy == PRECISE, timeout, age, previous)
        }

        /** Rejects options the Rust crate would refuse to decode. */
        private fun requireBounds(
            accuracy: Int,
            timeout: Int,
            age: Int,
            previous: Double?,
        ) {
            if (accuracy != APPROXIMATE && accuracy != PRECISE) throw PluginException(PluginException.INVALID_ARGUMENT)

            if (timeout !in MIN_TIMEOUT_MS..MAX_TIMEOUT_MS) throw PluginException(PluginException.INVALID_ARGUMENT)

            if (age !in 0..MAX_AGE_MS) throw PluginException(PluginException.INVALID_ARGUMENT)

            requirePrevious(previous)
        }

        /** A follow-up cursor must be a real timestamp, not a negative or infinite one. */
        private fun requirePrevious(previous: Double?) {
            if (previous == null) return

            if (!previous.isFinite() || previous < 0) throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        /** A bounds request is three u32 fields; a follow-up adds one timestamp. */
        private const val BOUNDS_BYTES = 12
        private const val WITH_PREVIOUS_BYTES = 20
        private const val APPROXIMATE = 0
        private const val PRECISE = 1
        private const val MIN_TIMEOUT_MS = 1
        private const val MAX_TIMEOUT_MS = 120_000
        private const val MAX_AGE_MS = 3_600_000
    }
}

/** Freshness uses the device monotonic clock, independently of wall-clock corrections. */
internal fun acceptsLocation(
    location: Location,
    options: LocationOptions,
    started: Long,
    now: Long,
): Boolean {
    val oldest = if (options.maximumAge == 0) started else now - options.maximumAge.toLong() * NANOS_PER_MILLI
    val measured = location.elapsedRealtimeNanos

    return measured > 0 && measured >= oldest && measured <= now &&
        (options.after?.let { location.time.toDouble() > it } ?: true)
}

/** Pure coordinate conversion; tests use synthetic Location objects without reading hardware. */
internal fun encodeLocation(location: Location): ByteArray {
    val optional = optionalValues(location)
    val values =
        doubleArrayOf(location.time.toDouble(), location.latitude, location.longitude, location.accuracy.toDouble()) +
            optional.map { it ?: 0.0 }.toDoubleArray()
    requireDeliverable(location, values)
    val bytes =
        ByteBuffer
            .allocate(RECORD_BYTES)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(0)
            .putInt(maskOf(optional))
            .putInt(providerCode(location.provider))
            .putInt(if (isMock(location)) MOCK_FLAG else REAL_FLAG)
            .putInt(0)
    values.forEach(bytes::putDouble)

    return bytes.array()
}

/** The optional fields in wire order: altitude, vertical accuracy, bearing, speed. */
private fun optionalValues(location: Location): Array<Double?> =
    arrayOf(
        if (location.hasAltitude()) location.altitude else null,
        verticalAccuracy(location),
        if (location.hasBearing()) location.bearing.toDouble() else null,
        if (location.hasSpeed()) location.speed.toDouble() else null,
    )

/** Vertical accuracy is only reported on API 26 and later. */
private fun verticalAccuracy(location: Location): Double? {
    if (Build.VERSION.SDK_INT >= VERTICAL_ACCURACY_API && location.hasVerticalAccuracy()) {
        return location.verticalAccuracyMeters.toDouble()
    }

    return null
}

/** Rejects a fix the wire cannot carry, so the Rust side never sees a broken one. */
private fun requireDeliverable(
    location: Location,
    values: DoubleArray,
) {
    if (!location.hasAccuracy()) throw PluginException(PluginException.NATIVE)

    if (values.any { !it.isFinite() }) throw PluginException(PluginException.NATIVE)

    if (values[TIME] < 0) throw PluginException(PluginException.NATIVE)

    requireAngles(values[LATITUDE], values[LONGITUDE])
    requireMagnitudes(values[ACCURACY], values[VERTICAL_ACCURACY], values[SPEED])
    requireBearing(values[BEARING])
}

/** Coordinates must be real degrees on the sphere. */
private fun requireAngles(
    latitude: Double,
    longitude: Double,
) {
    if (latitude !in -90.0..90.0) throw PluginException(PluginException.NATIVE)

    if (longitude !in -180.0..180.0) throw PluginException(PluginException.NATIVE)
}

/** Distances cannot be negative. */
private fun requireMagnitudes(
    accuracy: Double,
    vertical: Double,
    speed: Double,
) {
    if (accuracy < 0) throw PluginException(PluginException.NATIVE)

    if (vertical < 0) throw PluginException(PluginException.NATIVE)

    if (speed < 0) throw PluginException(PluginException.NATIVE)
}

/** A bearing is a clockwise angle from north, so it stays under a full turn. */
private fun requireBearing(bearing: Double) {
    if (bearing < 0) throw PluginException(PluginException.NATIVE)

    if (bearing >= FULL_TURN) throw PluginException(PluginException.NATIVE)
}

/** Bits for the optional fields the fix carries, in wire order. */
private fun maskOf(optional: Array<Double?>): Int {
    var mask = 0

    optional.forEachIndexed { index, value -> if (value != null) mask = mask or (1 shl index) }

    return mask
}

private fun providerCode(provider: String?): Int =
    when (provider) {
        LocationManager.GPS_PROVIDER -> PROVIDER_GPS
        LocationManager.NETWORK_PROVIDER -> PROVIDER_NETWORK
        LocationManager.PASSIVE_PROVIDER -> PROVIDER_PASSIVE
        FUSED -> PROVIDER_FUSED
        else -> PROVIDER_UNKNOWN
    }

@Suppress("DEPRECATION")
private fun isMock(location: Location): Boolean = if (Build.VERSION.SDK_INT >= MOCK_API) location.isMock else location.isFromMockProvider

/** Wire fields, in the order the eight doubles are written. */
private const val TIME = 0
private const val LATITUDE = 1
private const val LONGITUDE = 2
private const val ACCURACY = 3
private const val VERTICAL_ACCURACY = 5
private const val BEARING = 6
private const val SPEED = 7

/** One record is two u32 tags and eight doubles. */
private const val RECORD_BYTES = 84

/** Unix nanos per millisecond. */
private const val NANOS_PER_MILLI = 1_000_000

/** A full turn, in degrees. */
private const val FULL_TURN = 360.0

/** The provider codes the record carries; anything else is unknown. */
private const val PROVIDER_UNKNOWN = 0
private const val PROVIDER_GPS = 1
private const val PROVIDER_NETWORK = 2
private const val PROVIDER_PASSIVE = 3
private const val PROVIDER_FUSED = 4

/** Whether a fix came from the mock-location test surface. */
private const val REAL_FLAG = 1
private const val MOCK_FLAG = 2

/** Vertical accuracy and mock flags arrived in API 26 and 31. */
private const val VERTICAL_ACCURACY_API = 26
private const val MOCK_API = 31

/** The provider Google Play services adds alongside the platform's own. */
private const val FUSED = "fused"
