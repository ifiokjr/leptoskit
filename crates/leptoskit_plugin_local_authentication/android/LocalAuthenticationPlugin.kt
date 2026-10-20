package dev.leptoskit.plugins.localauthentication

import android.Manifest
import android.app.Activity
import android.app.Application
import android.app.KeyguardManager
import android.content.Context
import android.content.pm.PackageManager
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricPrompt
import android.os.Build
import android.os.Bundle
import android.os.CancellationSignal
import android.os.Handler
import android.os.Looper
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.suspendCancellableCoroutine
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.UUID
import java.util.concurrent.Executor
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/** Uses platform Class 3 authentication on API 30+, never a custom credential dialog. */
class LocalAuthenticationPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "local-authentication"
    override val methods = intArrayOf(0, 1, 2)
    private val handler = Handler(Looper.getMainLooper())
    private val executor = Executor { command -> handler.post(command) }
    private var active: Session? = null
    private var closed = false
    private val activity = context as? Activity
    private val lifecycle =
        object : Application.ActivityLifecycleCallbacks {
            override fun onActivityDestroyed(target: Activity) {
                if (target === activity) close()
            }

            override fun onActivityCreated(
                target: Activity,
                state: Bundle?,
            ) {}

            override fun onActivityStarted(target: Activity) {}

            override fun onActivityResumed(target: Activity) {}

            // Full-screen OS credential UI may pause or stop this Activity.
            override fun onActivityPaused(target: Activity) {}

            override fun onActivityStopped(target: Activity) {}

            override fun onActivitySaveInstanceState(
                target: Activity,
                state: Bundle,
            ) {}
        }

    init {
        activity?.application?.registerActivityLifecycleCallbacks(lifecycle)
    }

    private data class Session(
        val token: String,
        val signal: CancellationSignal,
        val completion: CancellableContinuation<ByteArray>,
    )

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Authentication dispatch requires the UI thread" }
        if (closed) throw PluginException(PluginException.CANCELLED)
        return when (method) {
            0 -> {
                val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)
                if (input.remaining() != 4) throw PluginException(PluginException.INVALID_ARGUMENT)
                val policy = input.int
                if (policy !in 0..1) throw PluginException(PluginException.INVALID_ARGUMENT)
                capabilities(policy)
            }

            1 -> {
                authenticate(AuthenticationRequest.decode(payload))
            }

            2 -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
                cancel(transport = false)
                ByteArray(0)
            }

            else -> {
                throw PluginException(PluginException.UNAVAILABLE)
            }
        }
    }

    private fun authenticators(policy: Int): Int =
        BiometricManager.Authenticators.BIOMETRIC_STRONG or
            if (policy == 1) BiometricManager.Authenticators.DEVICE_CREDENTIAL else 0

    private fun capabilities(policy: Int): ByteArray {
        val keyguard = context.getSystemService(KeyguardManager::class.java)
        val secure =
            if (keyguard == null) {
                0
            } else if (keyguard.isDeviceSecure) {
                2
            } else {
                1
            }
        if (Build.VERSION.SDK_INT < 30) return authenticationFields(1, policy, 1, 0, secure)
        val packageManager = context.packageManager
        var mask = 0
        if (packageManager.hasSystemFeature(PackageManager.FEATURE_FINGERPRINT)) mask = mask or 1
        if (packageManager.hasSystemFeature(PackageManager.FEATURE_FACE)) mask = mask or 2
        if (packageManager.hasSystemFeature(PackageManager.FEATURE_IRIS)) mask = mask or 4
        if (context.checkSelfPermission(Manifest.permission.USE_BIOMETRIC) != PackageManager.PERMISSION_GRANTED) {
            return authenticationFields(1, policy, 5, mask, secure)
        }
        val manager =
            context.getSystemService(BiometricManager::class.java)
                ?: return authenticationFields(1, policy, 1, mask, secure)
        val status =
            when (manager.canAuthenticate(authenticators(policy))) {
                BiometricManager.BIOMETRIC_SUCCESS -> 0
                BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED -> if (policy == 1 && secure == 1) 4 else 2
                BiometricManager.BIOMETRIC_ERROR_NO_HARDWARE, BiometricManager.BIOMETRIC_ERROR_HW_UNAVAILABLE -> 1
                BiometricManager.BIOMETRIC_ERROR_SECURITY_UPDATE_REQUIRED -> 6
                else -> 7
            }
        return authenticationFields(1, policy, status, mask, secure)
    }

    private suspend fun authenticate(request: AuthenticationRequest): ByteArray {
        currentCoroutineContext().ensureActive()
        if (active != null) throw PluginException(PluginException.BUSY)
        if (Build.VERSION.SDK_INT < 30) throw PluginException(PluginException.UNAVAILABLE)
        if (context.checkSelfPermission(Manifest.permission.USE_BIOMETRIC) != PackageManager.PERMISSION_GRANTED) {
            throw PluginException(PluginException.PERMISSION_DENIED)
        }
        val activity = context as? Activity ?: throw PluginException(PluginException.UNAVAILABLE)
        if (activity.isFinishing || activity.isDestroyed || !activity.hasWindowFocus()) {
            throw PluginException(PluginException.UNAVAILABLE)
        }
        return suspendCancellableCoroutine { continuation ->
            if (!continuation.isActive) return@suspendCancellableCoroutine
            val token = UUID.randomUUID().toString()
            val signal = CancellationSignal()
            active = Session(token, signal, continuation)
            continuation.invokeOnCancellation {
                if (Looper.myLooper() == handler.looper) {
                    if (active?.token == token) cancel(transport = true)
                } else {
                    handler.post { if (active?.token == token) cancel(transport = true) }
                }
            }
            try {
                val builder =
                    BiometricPrompt
                        .Builder(activity)
                        .setTitle(request.reason)
                        .setAllowedAuthenticators(authenticators(request.policy))
                        .setConfirmationRequired(true)
                if (request.policy == 0) builder.setNegativeButton(request.cancelLabel, executor) { _, _ -> finish(token, 2) }
                builder.build().authenticate(signal, executor, callback(token, request.policy))
            } catch (error: SecurityException) {
                fail(token, PluginException(PluginException.PERMISSION_DENIED, error))
            } catch (error: RuntimeException) {
                fail(token, PluginException(PluginException.NATIVE, error))
            }
        }
    }

    private fun callback(
        token: String,
        policy: Int,
    ): BiometricPrompt.AuthenticationCallback =
        object : BiometricPrompt.AuthenticationCallback() {
            override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                when (result.authenticationType) {
                    BiometricPrompt.AUTHENTICATION_RESULT_TYPE_BIOMETRIC -> {
                        finish(token, 0, 1)
                    }

                    BiometricPrompt.AUTHENTICATION_RESULT_TYPE_DEVICE_CREDENTIAL -> {
                        if (policy == 1) finish(token, 0, 2) else fail(token, PluginException(PluginException.NATIVE))
                    }

                    else -> {
                        fail(token, PluginException(PluginException.NATIVE))
                    }
                }
            }

            // Non-matches are retryable; the OS keeps the prompt open.
            override fun onAuthenticationFailed() {}

            override fun onAuthenticationError(
                errorCode: Int,
                errString: CharSequence,
            ) {
                val outcome =
                    when (errorCode) {
                        BiometricPrompt.BIOMETRIC_ERROR_USER_CANCELED -> 2
                        BiometricPrompt.BIOMETRIC_ERROR_CANCELED -> 3
                        BiometricPrompt.BIOMETRIC_ERROR_LOCKOUT, BiometricPrompt.BIOMETRIC_ERROR_LOCKOUT_PERMANENT -> 5
                        BiometricPrompt.BIOMETRIC_ERROR_NO_BIOMETRICS -> 6
                        BiometricPrompt.BIOMETRIC_ERROR_HW_NOT_PRESENT, BiometricPrompt.BIOMETRIC_ERROR_HW_UNAVAILABLE -> 7
                        BiometricPrompt.BIOMETRIC_ERROR_NO_DEVICE_CREDENTIAL -> 8
                        BiometricPrompt.BIOMETRIC_ERROR_TIMEOUT -> 9
                        BiometricPrompt.BIOMETRIC_ERROR_SECURITY_UPDATE_REQUIRED -> 10
                        else -> null
                    }
                if (outcome == null) fail(token, PluginException(PluginException.NATIVE)) else finish(token, outcome)
            }
        }

    private fun finish(
        token: String,
        outcome: Int,
        method: Int = 0,
    ) {
        val session = active ?: return
        if (session.token != token) return
        active = null
        session.signal.cancel()
        if (session.completion.isActive) session.completion.resume(authenticationFields(1, outcome, method))
    }

    private fun fail(
        token: String,
        error: PluginException,
    ) {
        val session = active ?: return
        if (session.token != token) return
        active = null
        session.signal.cancel()
        if (session.completion.isActive) session.completion.resumeWithException(error)
    }

    private fun cancel(transport: Boolean) {
        val session = active ?: return
        active = null
        session.signal.cancel()
        if (!session.completion.isActive) return
        if (transport) {
            session.completion.resumeWithException(PluginException(PluginException.CANCELLED))
        } else {
            session.completion.resume(authenticationFields(1, 4, 0))
        }
    }

    override fun close() {
        if (closed) return
        closed = true
        activity?.application?.unregisterActivityLifecycleCallbacks(lifecycle)
        cancel(transport = true)
    }
}
