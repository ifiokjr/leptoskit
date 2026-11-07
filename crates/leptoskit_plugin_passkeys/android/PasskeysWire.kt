package dev.leptoskit.plugins.passkeys

import dev.leptoskit.bridge.PluginException
import org.json.JSONException
import org.json.JSONObject
import java.nio.ByteBuffer
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction

/** The largest options or answer accepted. */
internal const val MAX_JSON_BYTES = 64 * 1024

/**
 * Checks WebAuthn options JSON (an object with a `challenge`) for Credential Manager,
 * which takes WebAuthn's standard JSON, and requires user verification in it, as Apple
 * does: registration options get `authenticatorSelection.userVerification` and sign-in
 * options get `userVerification`, both set to `required` whatever the app asked for.
 */
internal fun webAuthnOptions(
    payload: ByteBuffer,
    creating: Boolean,
): String {
    if (payload.remaining() !in 1..MAX_JSON_BYTES) throw PluginException(PluginException.INVALID_ARGUMENT)
    val text =
        try {
            Charsets.UTF_8
                .newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(payload.duplicate())
                .toString()
        } catch (_: CharacterCodingException) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
    val options =
        try {
            JSONObject(text)
        } catch (_: JSONException) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

    if (options.optString("challenge").isEmpty()) throw PluginException(PluginException.INVALID_ARGUMENT)

    try {
        if (creating) {
            val selection = options.optJSONObject("authenticatorSelection") ?: JSONObject()

            options.put("authenticatorSelection", selection.put("userVerification", REQUIRED))
        } else {
            options.put("userVerification", REQUIRED)
        }
    } catch (_: JSONException) {
        throw PluginException(PluginException.INVALID_ARGUMENT)
    }

    return options.toString()
}

/** The only user-verification requirement this plugin sends. */
private const val REQUIRED = "required"

/** An answer to return: bounded UTF-8 of a JSON object. */
internal fun webAuthnAnswer(json: String): ByteArray {
    val bytes = json.toByteArray(Charsets.UTF_8)

    if (bytes.size !in 1..MAX_JSON_BYTES) throw PluginException(PluginException.NATIVE)
    try {
        JSONObject(json)
    } catch (_: JSONException) {
        throw PluginException(PluginException.NATIVE)
    }

    return bytes
}
