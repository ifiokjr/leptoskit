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
import java.io.RandomAccessFile
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
    private val maxBytes: Int = 2560,
    private val maxEntries: Int = 64,
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
                ensureActive()
                lock.withLock { run(request) }
            }
        } finally {
            request.secret?.fill(0)
        }
    }

    /** Runs one request under the directory lock, translating platform refusals. */
    private fun run(request: Request): ByteArray {
        if (closed.get()) throw PluginException(PluginException.CANCELLED)

        return try {
            if (!directory.isDirectory && !directory.mkdirs()) throw IOException("Cannot create secure-store directory")

            RandomAccessFile(File(directory, LOCK_FILE), "rw").use { lockFile ->
                lockFile.channel.lock().use { execute(request) }
            }
        } catch (error: SecurityException) {
            throw PluginException(PluginException.PERMISSION_DENIED, error)
        } catch (error: GeneralSecurityException) {
            throw PluginException(PluginException.NATIVE, error)
        } catch (error: IOException) {
            throw PluginException(PluginException.NATIVE, error)
        }
    }

    private fun execute(request: Request): ByteArray {
        if (maxBytes !in MIN_BYTES..MAX_BYTES || maxEntries !in 0..MAX_ENTRIES) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
        val file = AtomicFile(File(directory, STORE_FILE))
        val identity = "leptoskit.secure-store-v2:$application".toByteArray(Charsets.UTF_8)
        val archive =
            if (file.baseFile.exists() || File(directory, "$STORE_FILE.bak").exists()) {
                load(file, identity)
            } else {
                SecureArchive()
            }

        try {
            return apply(request, archive, file, identity)
        } finally {
            archive.clear()
        }
    }

    /** Decrypts and decodes the stored archive. */
    private fun load(
        file: AtomicFile,
        identity: ByteArray,
    ): SecureArchive {
        val stored = readCiphertext(file, MAX_BYTES)
        val cipher = Cipher.getInstance(CIPHER)
        cipher.init(Cipher.DECRYPT_MODE, encryptionKey(create = false), GCMParameterSpec(TAG_BITS, stored, 1, NONCE_BYTES))

        cipher.updateAAD(identity)
        val plaintext = cipher.doFinal(stored, 13, stored.size - 13)

        return try {
            SecureArchive.decode(plaintext)
        } finally {
            plaintext.fill(0)
        }
    }

    /** Applies one request to the archive, and stores it again. */
    private fun apply(
        request: Request,
        archive: SecureArchive,
        file: AtomicFile,
        identity: ByteArray,
    ): ByteArray {
        if (request.method == READ) {
            val value = archive.get(request.service, request.key) ?: return executeLegacy(request)

            return byteArrayOf(1, 0, 0, 0) + value
        }

        val writing = request.method == WRITE

        if (writing) {
            archive.set(request.service, request.key, request.secret ?: throw PluginException(PluginException.INVALID_ARGUMENT))
        } else {
            // Remove the old representation before the new one to prevent resurrection.
            executeLegacy(request)
            archive.remove(request.service, request.key)
        }
        val plaintext = archive.encode(if (writing) maxBytes else MAX_BYTES, if (writing) maxEntries else MAX_ENTRIES)

        try {
            writeCiphertext(file, identity, plaintext)
        } finally {
            plaintext.fill(0)
        }

        if (writing) executeLegacy(Request(REMOVE, request.service, request.key, null))

        return byteArrayOf()
    }

    private fun executeLegacy(request: Request): ByteArray {
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

    private fun readCiphertext(
        file: AtomicFile,
        maximum: Int = MAX_SECRET,
    ): ByteArray =
        file.openRead().use { input ->
            val bytes = ByteArray(maximum + 30)
            var count = 0

            while (count < bytes.size) {
                val read = input.read(bytes, count, bytes.size - count)

                if (read < 0) break
                count += read
            }

            if (count < 29 || count > maximum + 29 || bytes[0] != 1.toByte()) {
                throw IOException("Invalid secure-store ciphertext")
            }

            bytes.copyOf(count)
        }

    private fun encryptionKey(create: Boolean): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        val alias = "$application.leptoskit.secure-store.v1"
        if (store.containsAlias(alias)) {
            return store.getKey(alias, null) as? SecretKey ?: throw GeneralSecurityException("Invalid secure-store key type")
        }

        if (!create) throw GeneralSecurityException("Secure-store key is missing")
        // Without the key, remaining ciphertext can never be decrypted. Discard it
        // before generating a new key so the store recovers instead of failing forever.
        directory.listFiles()?.forEach { entry ->

            if (entry.name == ".lock") return@forEach

            if (!entry.delete() && entry.exists()) throw IOException("Cannot discard undecryptable secure-store entry")
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

    internal class Request(
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
                val service = name(input)
                val key = name(input)
                val secret = if (method == WRITE) field(input, MAX_SECRET) else null

                if (input.hasRemaining()) {
                    secret?.fill(0)
                    throw PluginException(PluginException.INVALID_ARGUMENT)
                }

                return Request(method, service, key, secret)
            }

            /** One length-prefixed field, refusing one past `limit`. */
            private fun field(
                input: ByteBuffer,
                limit: Int,
            ): ByteArray {
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

            /** One service or key name. */
            private fun name(input: ByteBuffer): String {
                val value = field(input, MAX_NAME)

                if (value.isEmpty() || value.any { !isNameByte(it) }) {
                    throw PluginException(PluginException.INVALID_ARGUMENT)
                }

                return value.toString(Charsets.US_ASCII)
            }

            /** Whether one byte may appear in a name: ASCII letters, digits, `-`, `.`, `_`. */
            private fun isNameByte(byte: Byte): Boolean {
                val code = byte.toInt()

                return code in 65..90 || code in 97..122 || code in 48..57 || code == 45 || code == 46 || code == 95
            }
        }
    }

    private companion object {
        /** Reads one entry. */
        const val READ = 0

        /** Writes one entry. */
        const val WRITE = 1

        /** Removes one entry. */
        const val REMOVE = 2

        /** The largest secret, in bytes. */
        const val MAX_SECRET = 2048

        /** The largest service or key name, in bytes. */
        const val MAX_NAME = 128

        /** The largest archive this store will hold, in bytes. */
        const val MAX_BYTES = 2560

        /** The smallest archive worth storing. */
        const val MIN_BYTES = 4

        /** How many entries one archive may hold. */
        const val MAX_ENTRIES = 64

        /** The cipher every archive is written with. */
        const val CIPHER = "AES/GCM/NoPadding"

        /** The GCM authentication tag, in bits, and the nonce, in bytes. */
        const val TAG_BITS = 128
        const val NONCE_BYTES = 12

        /** The directory's lock file and the archive itself. */
        const val LOCK_FILE = ".lock"
        const val STORE_FILE = "entries-v2"

        val lock = Mutex()
    }
}
