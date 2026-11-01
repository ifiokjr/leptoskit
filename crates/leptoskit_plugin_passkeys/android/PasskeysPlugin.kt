package dev.leptoskit.plugins.passkeys

import android.app.Activity
import android.content.Context
import android.os.Looper
import androidx.credentials.CreatePublicKeyCredentialRequest
import androidx.credentials.CreatePublicKeyCredentialResponse
import androidx.credentials.CredentialManager
import androidx.credentials.GetCredentialRequest
import androidx.credentials.GetPublicKeyCredentialOption
import androidx.credentials.PublicKeyCredential
import androidx.credentials.exceptions.CreateCredentialCancellationException
import androidx.credentials.exceptions.CreateCredentialException
import androidx.credentials.exceptions.CreateCredentialProviderConfigurationException
import androidx.credentials.exceptions.CreateCredentialUnsupportedException
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.GetCredentialException
import androidx.credentials.exceptions.GetCredentialProviderConfigurationException
import androidx.credentials.exceptions.GetCredentialUnsupportedException
import androidx.credentials.exceptions.NoCredentialException
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Job
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import java.nio.ByteBuffer

/**
 * Passkeys through Credential Manager. The relying party's `assetlinks.json` must let
 * this app's signing certificate get its credentials. Sign in with Apple isn't offered
 * on Android: the sign-in client uses the system browser for it.
 */
class PasskeysPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "passkeys"
    override val methods = intArrayOf(0, 1, 2)
    private var job: Job? = null
    private var closed = false

    override fun close() {
        closed = true
        job?.cancel()
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Passkeys require the owning UI thread" }

        if (closed) throw PluginException(PluginException.CANCELLED)
        // Method 2, Sign in with Apple, has no Android implementation.
        if (method !in 0..1) throw PluginException(PluginException.UNAVAILABLE)
        val options = webAuthnOptions(payload)
        val activity = context as? Activity ?: throw PluginException(PluginException.UNAVAILABLE)

        if (job != null) throw PluginException(PluginException.BUSY)
        currentCoroutineContext().ensureActive()
        job = currentCoroutineContext()[Job]
        try {
            val manager = CredentialManager.create(activity)

            return if (method == 0) create(manager, activity, options) else get(manager, activity, options)
        } finally {
            job = null
        }
    }

    private suspend fun create(
        manager: CredentialManager,
        activity: Activity,
        options: String,
    ): ByteArray {
        val response =
            try {
                manager.createCredential(activity, CreatePublicKeyCredentialRequest(options))
            } catch (error: CreateCredentialCancellationException) {
                throw PluginException(PluginException.CANCELLED, error)
            } catch (error: CreateCredentialProviderConfigurationException) {
                throw PluginException(PluginException.UNAVAILABLE, error)
            } catch (error: CreateCredentialUnsupportedException) {
                throw PluginException(PluginException.UNAVAILABLE, error)
            } catch (error: CreateCredentialException) {
                throw PluginException(PluginException.NATIVE, error)
            }
        val created = response as? CreatePublicKeyCredentialResponse ?: throw PluginException(PluginException.NATIVE)

        return webAuthnAnswer(created.registrationResponseJson)
    }

    private suspend fun get(
        manager: CredentialManager,
        activity: Activity,
        options: String,
    ): ByteArray {
        val request = GetCredentialRequest(listOf(GetPublicKeyCredentialOption(options)))
        val response =
            try {
                manager.getCredential(activity, request)
            } catch (error: GetCredentialCancellationException) {
                throw PluginException(PluginException.CANCELLED, error)
            } catch (error: NoCredentialException) {
                throw PluginException(PluginException.CANCELLED, error)
            } catch (error: GetCredentialProviderConfigurationException) {
                throw PluginException(PluginException.UNAVAILABLE, error)
            } catch (error: GetCredentialUnsupportedException) {
                throw PluginException(PluginException.UNAVAILABLE, error)
            } catch (error: GetCredentialException) {
                throw PluginException(PluginException.NATIVE, error)
            }
        val credential = response.credential as? PublicKeyCredential ?: throw PluginException(PluginException.NATIVE)

        return webAuthnAnswer(credential.authenticationResponseJson)
    }
}
