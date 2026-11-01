package dev.leptoskit.plugins.sharing

import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder

private fun wire(
    value: String,
    kind: Int = 0,
): ByteBuffer {
    val bytes = value.toByteArray(Charsets.UTF_8)
    val result = ByteBuffer.allocate(8 + bytes.size).order(ByteOrder.LITTLE_ENDIAN)
    result.putInt(kind).putInt(bytes.size).put(bytes)
    result.flip()
    return result
}

fun main() {
    check(ShareContent.decode(wire("hello\n世界")).value == "hello\n世界")
    check(ShareContent.decode(wire("https://example.test/a%20b", 1)).kind == 1)
    for ((value, kind) in listOf(
        "" to 0,
        "a\u0000b" to 0,
        "file:///secret" to 1,
        "https://user:pass@example.test" to 1,
        "https:///host" to 1,
    )) {
        try {
            ShareContent.decode(wire(value, kind))
            error("Expected rejection: $value")
        } catch (error: PluginException) {
            check(error.code == PluginException.INVALID_ARGUMENT)
        }
    }

    try {
        ShareContent.decode(wire("a".repeat(65537)))
        error("Expected bounds failure")
    } catch (error: PluginException) {
        check(error.code == PluginException.TOO_LARGE)
    }
    val malformed = ByteBuffer.wrap(byteArrayOf(0, 0, 0, 0, 1, 0, 0, 0, -1))
    try {
        ShareContent.decode(malformed)
        error("Expected malformed UTF-8 rejection")
    } catch (error: PluginException) {
        check(error.code == PluginException.INVALID_ARGUMENT)
    }
    println("Sharing synthetic wire and bounds checks passed")
}
