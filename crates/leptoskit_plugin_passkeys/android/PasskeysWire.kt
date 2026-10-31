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
 * Checks WebAuthn options JSON (an object with a `challenge`) and returns it unchanged,
 * for Credential Manager, which takes WebAuthn's standard JSON as it is.
 */
internal fun webAuthnOptions(payload: ByteBuffer): String {
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
    return text
}

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
