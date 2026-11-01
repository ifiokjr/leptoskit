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
    override val methods = intArrayOf(CAPABILITIES, AUTHENTICATE, CANCEL)
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
            CAPABILITIES -> capabilities(policyOf(payload))
            AUTHENTICATE -> authenticate(AuthenticationRequest.decode(payload))
            CANCEL -> cancelRequest(payload)
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }
    }

    /** Reads the policy a capabilities query asked about. */
    private fun policyOf(payload: ByteBuffer): Int {
        val input = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)

        if (input.remaining() != POLICY_BYTES) throw PluginException(PluginException.INVALID_ARGUMENT)

        val policy = input.int

        if (policy != BIOMETRIC_POLICY && policy != CREDENTIAL_POLICY) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        return policy
    }

    /** Dismisses this plugin's prompt and answers with empty bytes. */
    private fun cancelRequest(payload: ByteBuffer): ByteArray {
        if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

        cancel(transport = false)

        return ByteArray(0)
    }

    /** Reports what the OS can do for a policy, without showing a prompt. */
    private fun capabilities(policy: Int): ByteArray {
        val secure = deviceSecure()

        if (Build.VERSION.SDK_INT < BIOMETRIC_API) {
            return authenticationFields(WIRE_VERSION, policy, READINESS_UNAVAILABLE, 0, secure)
        }

        val mask = modalities()

        if (context.checkSelfPermission(Manifest.permission.USE_BIOMETRIC) != PackageManager.PERMISSION_GRANTED) {
            return authenticationFields(WIRE_VERSION, policy, READINESS_PERMISSION_DENIED, mask, secure)
        }

        val manager =
            context.getSystemService(BiometricManager::class.java)
                ?: return authenticationFields(WIRE_VERSION, policy, READINESS_UNAVAILABLE, mask, secure)

        return authenticationFields(WIRE_VERSION, policy, readiness(policy, manager, secure), mask, secure)
    }

    /** Whether a device credential is set: unknown, no, or yes. */
    private fun deviceSecure(): Int {
        val keyguard = context.getSystemService(KeyguardManager::class.java) ?: return SECURE_UNKNOWN

        return if (keyguard.isDeviceSecure) SECURE_YES else SECURE_NO
    }

    /** The sensor bits the OS reports, which do not prove enrollment or strength. */
    private fun modalities(): Int {
        val manager = context.packageManager
        var mask = 0

        if (manager.hasSystemFeature(PackageManager.FEATURE_FINGERPRINT)) mask = mask or FINGERPRINT

        if (manager.hasSystemFeature(PackageManager.FEATURE_FACE)) mask = mask or FACE

        if (manager.hasSystemFeature(PackageManager.FEATURE_IRIS)) mask = mask or IRIS

        return mask
    }

    /** The OS readiness for one policy, as the wire's readiness code. */
    private fun readiness(
        policy: Int,
        manager: BiometricManager,
        secure: Int,
    ): Int =
        when (manager.canAuthenticate(authenticators(policy))) {
            BiometricManager.BIOMETRIC_SUCCESS -> {
                READINESS_READY
            }

            BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED -> {
                if (policy == CREDENTIAL_POLICY && secure == SECURE_NO) {
                    READINESS_CREDENTIAL_NOT_SET
                } else {
                    READINESS_NOT_ENROLLED
                }
            }

            BiometricManager.BIOMETRIC_ERROR_NO_HARDWARE,
            BiometricManager.BIOMETRIC_ERROR_HW_UNAVAILABLE,
            -> {
                READINESS_UNAVAILABLE
            }

            BiometricManager.BIOMETRIC_ERROR_SECURITY_UPDATE_REQUIRED -> {
                READINESS_SECURITY_UPDATE
            }

            else -> {
                READINESS_UNKNOWN
            }
        }

    /** Class 3 biometrics, plus the device credential when the policy allows it. */
    private fun authenticators(policy: Int): Int {
        if (policy == CREDENTIAL_POLICY) {
            return BiometricManager.Authenticators.BIOMETRIC_STRONG or
                BiometricManager.Authenticators.DEVICE_CREDENTIAL
        }

        return BiometricManager.Authenticators.BIOMETRIC_STRONG
    }

    /** Shows the system prompt and answers with its terminal outcome. */
    private suspend fun authenticate(request: AuthenticationRequest): ByteArray {
        currentCoroutineContext().ensureActive()

        if (active != null) throw PluginException(PluginException.BUSY)

        val activity = promptableActivity()

        return suspendCancellableCoroutine { continuation ->

            if (continuation.isActive) prompt(activity, request, continuation)
        }
    }

    /** The activity a system prompt can attach to, or an error saying why not. */
    private fun promptableActivity(): Activity {
        if (Build.VERSION.SDK_INT < BIOMETRIC_API) throw PluginException(PluginException.UNAVAILABLE)

        if (context.checkSelfPermission(Manifest.permission.USE_BIOMETRIC) != PackageManager.PERMISSION_GRANTED) {
            throw PluginException(PluginException.PERMISSION_DENIED)
        }

        val activity = context as? Activity ?: throw PluginException(PluginException.UNAVAILABLE)
        // Visible, not focused: apps commonly authenticate at launch, before the
        // window gains focus.
        if (activity.isFinishing || activity.isDestroyed || activity.window?.decorView?.isShown != true) {
            throw PluginException(PluginException.UNAVAILABLE)
        }

        return activity
    }

    /** Registers the session, then shows the prompt it belongs to. */
    private fun prompt(
        activity: Activity,
        request: AuthenticationRequest,
        continuation: CancellableContinuation<ByteArray>,
    ) {
        val token = UUID.randomUUID().toString()
        val session = Session(token, CancellationSignal(), continuation)
        active = session
        continuation.invokeOnCancellation { cancelPending(token) }

        try {
            show(activity, request, session)
        } catch (error: SecurityException) {
            fail(token, PluginException(PluginException.PERMISSION_DENIED, error))
        } catch (error: RuntimeException) {
            fail(token, PluginException(PluginException.NATIVE, error))
        }
    }

    /** Builds and shows the OS prompt for one session. */
    private fun show(
        activity: Activity,
        request: AuthenticationRequest,
        session: Session,
    ) {
        val builder =
            BiometricPrompt
                .Builder(activity)
                .setTitle(request.reason)
                .setAllowedAuthenticators(authenticators(request.policy))
                .setConfirmationRequired(true)

        if (request.policy == BIOMETRIC_POLICY) {
            builder.setNegativeButton(request.cancelLabel, executor) { _, _ ->
                finish(session.token, OUTCOME_USER_CANCELLED)
            }
        }

        builder.build().authenticate(session.signal, executor, callback(session.token, request.policy))
    }

    /** Cancels the session from whichever thread the cancellation arrives on. */
    private fun cancelPending(token: String) {
        if (Looper.myLooper() == handler.looper) {
            cancelCurrent(token)

            return
        }

        handler.post { cancelCurrent(token) }
    }

    private fun cancelCurrent(token: String) {
        if (active?.token == token) cancel(transport = true)
    }

    private fun callback(
        token: String,
        policy: Int,
    ): BiometricPrompt.AuthenticationCallback =
        object : BiometricPrompt.AuthenticationCallback() {
            override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                succeeded(token, policy, result.authenticationType)
            }

            // Non-matches are retryable; the OS keeps the prompt open.
            override fun onAuthenticationFailed() {}

            override fun onAuthenticationError(
                errorCode: Int,
                errString: CharSequence,
            ) {
                failed(token, errorCode)
            }
        }

    /** Records which credential the OS accepted. */
    private fun succeeded(
        token: String,
        policy: Int,
        method: Int,
    ) {
        val verified = verifiedMethod(policy, method) ?: return fail(token, PluginException(PluginException.NATIVE))

        finish(token, OUTCOME_SUCCESS, verified)
    }

    /** The wire method for the accepted credential, or null when it cannot satisfy the policy. */
    private fun verifiedMethod(
        policy: Int,
        method: Int,
    ): Int? =
        when (method) {
            BiometricPrompt.AUTHENTICATION_RESULT_TYPE_BIOMETRIC -> {
                METHOD_BIOMETRIC
            }

            BiometricPrompt.AUTHENTICATION_RESULT_TYPE_DEVICE_CREDENTIAL -> {
                if (policy == CREDENTIAL_POLICY) METHOD_CREDENTIAL else null
            }

            else -> {
                null
            }
        }

    /** Maps an OS prompt error onto the wire outcome. */
    private fun failed(
        token: String,
        errorCode: Int,
    ) {
        val outcome = outcomeOf(errorCode)

        if (outcome == null) fail(token, PluginException(PluginException.NATIVE)) else finish(token, outcome)
    }

    /** The wire outcome for one of the prompt's error codes. */
    private fun outcomeOf(errorCode: Int): Int? =
        when (errorCode) {
            BiometricPrompt.BIOMETRIC_ERROR_USER_CANCELED -> OUTCOME_USER_CANCELLED

            BiometricPrompt.BIOMETRIC_ERROR_CANCELED -> OUTCOME_SYSTEM_CANCELLED

            BiometricPrompt.BIOMETRIC_ERROR_LOCKOUT,
            BiometricPrompt.BIOMETRIC_ERROR_LOCKOUT_PERMANENT,
            -> OUTCOME_LOCKED_OUT

            BiometricPrompt.BIOMETRIC_ERROR_NO_BIOMETRICS -> OUTCOME_NOT_ENROLLED

            BiometricPrompt.BIOMETRIC_ERROR_HW_NOT_PRESENT,
            BiometricPrompt.BIOMETRIC_ERROR_HW_UNAVAILABLE,
            -> OUTCOME_UNAVAILABLE

            BiometricPrompt.BIOMETRIC_ERROR_NO_DEVICE_CREDENTIAL -> OUTCOME_CREDENTIAL_NOT_SET

            BiometricPrompt.BIOMETRIC_ERROR_TIMEOUT -> OUTCOME_TIMED_OUT

            BiometricPrompt.BIOMETRIC_ERROR_SECURITY_UPDATE_REQUIRED -> OUTCOME_SECURITY_UPDATE

            else -> null
        }

    /** Takes the caller's session back, so only its first answer is delivered. */
    private fun take(token: String): Session? {
        if (active?.token != token) return null

        return take()
    }

    /** Takes whatever session is active, cancelling its signal. */
    private fun take(): Session? {
        val session = active ?: return null
        active = null
        session.signal.cancel()

        return session
    }

    private fun finish(
        token: String,
        outcome: Int,
        method: Int = METHOD_UNKNOWN,
    ) {
        val session = take(token) ?: return

        if (session.completion.isActive) session.completion.resume(authenticationFields(WIRE_VERSION, outcome, method))
    }

    private fun fail(
        token: String,
        error: PluginException,
    ) {
        val session = take(token) ?: return

        if (session.completion.isActive) session.completion.resumeWithException(error)
    }

    private fun cancel(transport: Boolean) {
        val session = take() ?: return

        if (!session.completion.isActive) return

        if (transport) {
            session.completion.resumeWithException(PluginException(PluginException.CANCELLED))
        } else {
            session.completion.resume(authenticationFields(WIRE_VERSION, OUTCOME_APP_CANCELLED, METHOD_UNKNOWN))
        }
    }

    override fun close() {
        if (closed) return
        closed = true
        activity?.application?.unregisterActivityLifecycleCallbacks(lifecycle)
        cancel(transport = true)
    }

    private companion object {
        /** Methods the Rust crate calls. */
        const val CAPABILITIES = 0
        const val AUTHENTICATE = 1
        const val CANCEL = 2

        /** The first API level that answers readiness for a policy. */
        const val BIOMETRIC_API = 30

        /** Readiness codes, in the order the Rust enum declares them. */
        const val READINESS_READY = 0
        const val READINESS_UNAVAILABLE = 1
        const val READINESS_NOT_ENROLLED = 2
        const val READINESS_CREDENTIAL_NOT_SET = 4
        const val READINESS_PERMISSION_DENIED = 5
        const val READINESS_SECURITY_UPDATE = 6
        const val READINESS_UNKNOWN = 7

        /** Whether a device credential is set: unknown, no, yes. */
        const val SECURE_UNKNOWN = 0
        const val SECURE_NO = 1
        const val SECURE_YES = 2

        /** Sensor bits in the modality mask. */
        const val FINGERPRINT = 1
        const val FACE = 2
        const val IRIS = 4

        /** Terminal outcome codes the Rust enum decodes. */
        const val OUTCOME_SUCCESS = 0
        const val OUTCOME_USER_CANCELLED = 2
        const val OUTCOME_SYSTEM_CANCELLED = 3
        const val OUTCOME_APP_CANCELLED = 4
        const val OUTCOME_LOCKED_OUT = 5
        const val OUTCOME_NOT_ENROLLED = 6
        const val OUTCOME_UNAVAILABLE = 7
        const val OUTCOME_CREDENTIAL_NOT_SET = 8
        const val OUTCOME_TIMED_OUT = 9
        const val OUTCOME_SECURITY_UPDATE = 10

        /** Which credential satisfied the policy. */
        const val METHOD_UNKNOWN = 0
        const val METHOD_BIOMETRIC = 1
        const val METHOD_CREDENTIAL = 2
    }
}
