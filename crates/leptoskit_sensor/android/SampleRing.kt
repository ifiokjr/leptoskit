package dev.leptoskit.sensors

import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * The newest samples of one stream, each numbered so a reader can ask for
 * everything after the last one it saw. Numbers keep counting across
 * restarts, so a reader's old cursor never matches a sample it has not seen.
 */
class SampleRing {
    private val samples = ArrayDeque<ByteArray>()

    var lastSequence = 0L
        private set

    val isEmpty: Boolean get() = samples.isEmpty()

    /** Keeps a sample, forgetting the oldest once full. */
    fun append(sample: ByteArray) {
        if (samples.size == CAPACITY) samples.removeFirst()

        samples.addLast(sample)
        lastSequence++
    }

    /** Forgets every sample but keeps counting from the last number. */
    fun clear() = samples.clear()

    /**
     * The reply to a watch from [cursor]: the newest number, how many samples
     * were lost before the reader asked, the count, then each sample. Null when
     * nothing newer than the cursor is held.
     */
    fun batch(cursor: Long): ByteArray? {
        if (cursor >= lastSequence || samples.isEmpty()) return null

        val oldest = lastSequence - samples.size + 1
        val skip = if (cursor >= oldest) (cursor - oldest + 1).toInt() else 0
        val dropped = if (cursor + 1 < oldest) minOf(oldest - cursor - 1, MAX_U32) else 0L
        val kept = samples.drop(skip)
        val bytes = ByteBuffer.allocate(HEADER_BYTES + kept.sumOf { it.size }).order(ByteOrder.LITTLE_ENDIAN)

        // The u32 count's bits survive the conversion to a signed int.
        bytes.putLong(lastSequence).putInt(dropped.toInt()).putInt(kept.size)
        kept.forEach { bytes.put(it) }

        return bytes.array()
    }

    companion object {
        /** The most samples held, and so the most one reply carries. */
        const val CAPACITY = 64

        /** Cursor, dropped count and sample count. */
        private const val HEADER_BYTES = 16

        /** The largest dropped count the wire's u32 carries. */
        private const val MAX_U32 = 0xFFFF_FFFFL
    }
}
