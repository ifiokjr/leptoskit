package dev.leptoskit.plugins.securestore

import androidx.test.platform.app.InstrumentationRegistry
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Test
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.security.MessageDigest

/** Run only in a test application; entries use a reserved test service. */
class SecureStorePluginTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val plugin = SecureStorePlugin(context)
    private val service = "leptoskit-instrumentation"

    @Test
    fun roundtripOverwriteEmptyAndServiceIsolation() =
        runBlocking {
            try {
                plugin.invoke(2, request("binary"))
                assertArrayEquals(byteArrayOf(0, 0, 0, 0), plugin.invoke(0, request("binary")))
                plugin.invoke(1, request("binary", byteArrayOf(0, 127, -1)))
                assertArrayEquals(byteArrayOf(1, 0, 0, 0, 0, 127, -1), plugin.invoke(0, request("binary")))
                assertArrayEquals(byteArrayOf(0, 0, 0, 0), plugin.invoke(0, request("binary", scope = "$service-other")))
                plugin.invoke(1, request("binary", byteArrayOf()))
                assertArrayEquals(byteArrayOf(1, 0, 0, 0), plugin.invoke(0, request("binary")))
                plugin.invoke(2, request("binary"))
                plugin.invoke(2, request("binary"))
                assertArrayEquals(byteArrayOf(0, 0, 0, 0), plugin.invoke(0, request("binary")))
            } finally {
                plugin.invoke(2, request("binary"))
            }
        }

    @Test
    fun tamperedCiphertextIsAnErrorAndNeverPlaintext() =
        runBlocking {
            try {
                plugin.invoke(1, request("tamper", "public test fixture".toByteArray()))
                val file = storedFile("tamper")
                val ciphertext = file.readBytes()
                ciphertext[ciphertext.lastIndex] = (ciphertext.last().toInt() xor 1).toByte()
                file.writeBytes(ciphertext)
                try {
                    plugin.invoke(0, request("tamper"))
                    fail("Authentication failure must not return a missing value or plaintext")
                } catch (error: PluginException) {
                    assertEquals(PluginException.NATIVE, error.code)
                }
            } finally {
                plugin.invoke(2, request("tamper"))
            }
        }

    @Test
    fun swappingEntriesFailsAuthenticatedIdentityCheck() =
        runBlocking {
            try {
                plugin.invoke(1, request("swap-a", byteArrayOf(1, 2, 3)))
                plugin.invoke(1, request("swap-b", byteArrayOf(4, 5, 6)))
                storedFile("swap-b").writeBytes(storedFile("swap-a").readBytes())
                try {
                    plugin.invoke(0, request("swap-b"))
                    fail("Ciphertext from another key must fail authentication")
                } catch (error: PluginException) {
                    assertEquals(PluginException.NATIVE, error.code)
                }
            } finally {
                plugin.invoke(2, request("swap-a"))
                plugin.invoke(2, request("swap-b"))
            }
        }

    @Test
    fun rejectsMalformedRequestsBeforeStorage() =
        runBlocking {
            val invalid = listOf(ByteArray(0), request("trailer").array() + byteArrayOf(0), request("../escape").array())
            for (payload in invalid) {
                try {
                    plugin.invoke(0, ByteBuffer.wrap(payload))
                    fail("Malformed request must fail")
                } catch (error: PluginException) {
                    assertEquals(PluginException.INVALID_ARGUMENT, error.code)
                }
            }
        }

    private fun request(
        key: String,
        value: ByteArray? = null,
        scope: String = service,
    ): ByteBuffer {
        val serviceBytes = scope.toByteArray()
        val keyBytes = key.toByteArray()
        return ByteBuffer
            .allocate(8 + serviceBytes.size + keyBytes.size + (value?.let { 4 + it.size } ?: 0))
            .order(ByteOrder.LITTLE_ENDIAN)
            .apply {
                putInt(serviceBytes.size).put(serviceBytes)
                putInt(keyBytes.size).put(keyBytes)
                value?.let { putInt(it.size).put(it) }
                flip()
            }
    }

    private fun storedFile(key: String): File {
        val identity = "leptoskit.secure-store:${context.packageName}:$service/$key".toByteArray()
        val name = MessageDigest.getInstance("SHA-256").digest(identity).joinToString("") { "%02x".format(it.toInt() and 255) }
        return File(File(context.noBackupFilesDir, "leptoskit-secure-store"), name)
    }
}
