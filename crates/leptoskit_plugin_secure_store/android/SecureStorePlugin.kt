package dev.leptoskit.plugins.securestore

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.AtomicFile
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.io.File
import java.io.IOException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.security.GeneralSecurityException
import java.security.KeyStore
import java.security.MessageDigest
import java.util.concurrent.atomic.AtomicBoolean
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** AES-GCM ciphertext lives in Android's non-backup directory; keys never leave AndroidKeyStore. */
class SecureStorePlugin(
    context: Context,
) : NativePlugin {
    override val name = "secure-store"
    override val methods = intArrayOf(0, 1, 2)
    private val directory = File(context.applicationContext.noBackupFilesDir, "leptoskit-secure-store")
    private val application = context.applicationContext.packageName
    private val closed = AtomicBoolean(false)

    override fun close() {
        closed.set(true)
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed.get()) throw PluginException(PluginException.CANCELLED)
        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)
        // The bridge-owned buffer must not cross a suspension point.
        val request = Request.decode(method, payload)
        return try {
            withContext(Dispatchers.IO) {
                lock.withLock {
                    ensureActive()
                    if (closed.get()) throw PluginException(PluginException.CANCELLED)
                    try {
                        execute(request)
                    } catch (error: SecurityException) {
                        throw PluginException(PluginException.PERMISSION_DENIED, error)
                    } catch (error: GeneralSecurityException) {
                        throw PluginException(PluginException.NATIVE, error)
                    } catch (error: IOException) {
                        throw PluginException(PluginException.NATIVE, error)
                    }
                }
            }
        } finally {
            request.secret?.fill(0)
        }
    }

    private fun execute(request: Request): ByteArray {
        if (!directory.isDirectory && !directory.mkdirs()) throw IOException("Cannot create secure-store directory")
        val identity = "leptoskit.secure-store:$application:${request.service}/${request.key}".toByteArray(Charsets.UTF_8)
        val digest = MessageDigest.getInstance("SHA-256").digest(identity)
        val filename = digest.joinToString("") { "%02x".format(it.toInt() and 255) }
        val file = AtomicFile(File(directory, filename))
        return when (request.method) {
            0 -> {
                // AtomicFile restores an interrupted write from its backup, if present.
                if (!file.baseFile.exists() && !File(directory, "$filename.bak").exists()) {
                    byteArrayOf(0, 0, 0, 0)
                } else {
                    val stored = readCiphertext(file)
                    val cipher = Cipher.getInstance("AES/GCM/NoPadding")
                    cipher.init(Cipher.DECRYPT_MODE, encryptionKey(create = false), GCMParameterSpec(128, stored, 1, 12))
                    cipher.updateAAD(identity)
                    val plaintext = cipher.doFinal(stored, 13, stored.size - 13)
                    try {
                        byteArrayOf(1, 0, 0, 0) + plaintext
                    } finally {
                        plaintext.fill(0)
                    }
                }
            }

            1 -> {
                val value = request.secret ?: throw PluginException(PluginException.INVALID_ARGUMENT)
                val cipher = Cipher.getInstance("AES/GCM/NoPadding")
                cipher.init(Cipher.ENCRYPT_MODE, encryptionKey(create = true))
                if (cipher.iv.size != 12) throw GeneralSecurityException("Unexpected GCM IV size")
                cipher.updateAAD(identity)
                val ciphertext = byteArrayOf(1) + cipher.iv + cipher.doFinal(value)
                val stream = file.startWrite()
                try {
                    stream.write(ciphertext)
                    stream.fd.sync()
                    file.finishWrite(stream)
                } catch (error: Exception) {
                    // AtomicFile requires failWrite for every unsuccessful write, then propagation.
                    file.failWrite(stream)
                    throw error
                }
                // AtomicFile logs some rename failures instead of throwing them.
                if (!MessageDigest.isEqual(ciphertext, readCiphertext(file))) {
                    throw IOException("Secure-store write did not commit")
                }
                byteArrayOf()
            }

            2 -> {
                file.delete()
                if (file.baseFile.exists() || File(directory, "$filename.bak").exists() ||
                    File(directory, "$filename.new").exists()
                ) {
                    throw IOException("Cannot delete secure-store entry")
                }
                byteArrayOf()
            }

            else -> {
                throw PluginException(PluginException.UNAVAILABLE)
            }
        }
    }

    private fun readCiphertext(file: AtomicFile): ByteArray =
        file.openRead().use { input ->
            val bytes = ByteArray(MAX_SECRET + 30)
            var count = 0
            while (count < bytes.size) {
                val read = input.read(bytes, count, bytes.size - count)
                if (read < 0) break
                count += read
            }
            if (count < 29 || count > MAX_SECRET + 29 || bytes[0] != 1.toByte()) {
                throw IOException("Invalid secure-store ciphertext")
            }
            bytes.copyOf(count)
        }

    private fun encryptionKey(create: Boolean): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        val alias = "$application.leptoskit.secure-store.v1"
        val existing = store.getKey(alias, null)
        if (existing != null) {
            return existing as? SecretKey ?: throw GeneralSecurityException("Invalid secure-store key type")
        }
        if (!create || directory.list()?.isNotEmpty() != false) {
            throw GeneralSecurityException("Secure-store key is missing")
        }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        generator.init(
            KeyGenParameterSpec
                .Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .setRandomizedEncryptionRequired(true)
                .build(),
        )
        return generator.generateKey()
    }

    private class Request(
        val method: Int,
        val service: String,
        val key: String,
        val secret: ByteArray?,
    ) {
        companion object {
            fun decode(
                method: Int,
                source: ByteBuffer,
            ): Request {
                val input = source.duplicate().order(ByteOrder.LITTLE_ENDIAN)

                fun field(limit: Int): ByteArray {
                    if (input.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)
                    val size = input.int
                    if (limit == MAX_SECRET && (size < 0 || size > limit)) {
                        throw PluginException(PluginException.TOO_LARGE)
                    }
                    if (size < 0 || size > limit || size > input.remaining()) {
                        throw PluginException(PluginException.INVALID_ARGUMENT)
                    }
                    return ByteArray(size).also(input::get)
                }

                fun name(): String {
                    val value = field(128)
                    if (value.isEmpty() ||
                        value.any { byte ->
                            val code = byte.toInt()
                            code !in 65..90 && code !in 97..122 && code !in 48..57 && code != 45 && code != 46 && code != 95
                        }
                    ) {
                        throw PluginException(PluginException.INVALID_ARGUMENT)
                    }
                    return value.toString(Charsets.US_ASCII)
                }
                val service = name()
                val key = name()
                val secret = if (method == 1) field(MAX_SECRET) else null
                if (input.hasRemaining()) {
                    secret?.fill(0)
                    throw PluginException(PluginException.INVALID_ARGUMENT)
                }
                return Request(method, service, key, secret)
            }
        }
    }

    private companion object {
        const val MAX_SECRET = 2048
        val lock = Mutex()
    }
}
