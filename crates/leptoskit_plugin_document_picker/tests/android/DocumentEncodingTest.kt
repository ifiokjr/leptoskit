package dev.leptoskit.plugins.documentpicker

import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.runBlocking
import java.io.ByteArrayInputStream

private const val NUL = 0.toByte()
private const val DELETE = 127.toByte()
private const val MAX_BYTE = (-1).toByte()

fun main() =
    runBlocking {
        val bytes = byteArrayOf(NUL, DELETE, MAX_BYTE)
        check(DocumentEncoding.readBounded(ByteArrayInputStream(bytes), 3) {}.contentEquals(bytes))
        check(DocumentEncoding.readBounded(ByteArrayInputStream(byteArrayOf()), 0) {}.isEmpty())
        try {
            DocumentEncoding.readBounded(ByteArrayInputStream(bytes), 2) {}
            error("Expected size failure")
        } catch (error: PluginException) {
            check(error.code == PluginException.TOO_LARGE)
        }
        var closed = false
        val stream =
            object : ByteArrayInputStream(bytes) {
                override fun close() {
                    closed = true
                    super.close()
                }
            }
        try {
            stream.use {
                DocumentEncoding.readBounded(it, 3) { throw CancellationException("synthetic cancellation") }
            }
            error("Expected cancellation")
        } catch (_: CancellationException) {
            check(closed)
            check(stream.available() == 3)
        }
        val reply = DocumentEncoding.encode("test".toByteArray(), byteArrayOf(), bytes)
        check(reply.takeLast(3).toByteArray().contentEquals(bytes))
        println("Document encoding bounds and cancellation passed")
    }
