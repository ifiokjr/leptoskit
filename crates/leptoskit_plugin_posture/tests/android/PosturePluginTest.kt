package dev.leptoskit.plugins.posture

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Synthetic wire-format fixtures only. These tests never open a foldable
 * emulator or read the hinge angle sensor.
 */
class PosturePluginTest {
    @Test
    fun wireFormatMatchesRustTenWordHinges() {
        val hinge =
            PosturePlugin.EncodedHinge(
                fold = PosturePlugin.HALF_OPEN,
                axis = PosturePlugin.VERTICAL,
                occluding = PosturePlugin.FALSE,
                separating = PosturePlugin.TRUE,
                x = 0.48f,
                y = 0f,
                width = 0.04f,
                height = 1f,
                angle = 97,
            )
        val bytes =
            PosturePlugin.encodePosture(
                PosturePlugin.TRUE,
                PosturePlugin.HALF_OPEN,
                listOf(hinge),
            )
        assertEquals(PosturePlugin.HEADER_BYTES + PosturePlugin.HINGE_BYTES, bytes.size)

        val view = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        assertEquals(PosturePlugin.TRUE, view.int)
        assertEquals(PosturePlugin.HALF_OPEN, view.int)
        assertEquals(1, view.int)
        assertEquals(PosturePlugin.HALF_OPEN, view.int)
        assertEquals(PosturePlugin.VERTICAL, view.int)
        assertEquals(PosturePlugin.FALSE, view.int)
        assertEquals(PosturePlugin.TRUE, view.int)
        assertEquals(1, view.int)
        assertEquals(0.48f, view.float, 0f)
        assertEquals(0f, view.float, 0f)
        assertEquals(0.04f, view.float, 0f)
        assertEquals(1f, view.float, 0f)
        assertEquals(97, view.int)
        assertEquals(0, view.remaining())
    }

    @Test
    fun continuousOpenFoldReportsNotSeparating() {
        // A flat continuous foldable keeps its crease geometry but marks the
        // hinge as not separating so Rust's panes() stays one pane.
        val continuous =
            PosturePlugin.EncodedHinge(
                fold = PosturePlugin.OPEN,
                axis = PosturePlugin.VERTICAL,
                occluding = PosturePlugin.FALSE,
                separating = PosturePlugin.FALSE,
                x = 0.5f,
                y = 0f,
                width = 0f,
                height = 1f,
                angle = 180,
            )
        val tabletop =
            PosturePlugin.EncodedHinge(
                fold = PosturePlugin.HALF_OPEN,
                axis = PosturePlugin.HORIZONTAL,
                occluding = PosturePlugin.FALSE,
                separating = PosturePlugin.TRUE,
                x = 0f,
                y = 0.5f,
                width = 1f,
                height = 0f,
                angle = 90,
            )

        val open = PosturePlugin.encodePosture(PosturePlugin.TRUE, PosturePlugin.OPEN, listOf(continuous))
        val half =
            PosturePlugin.encodePosture(
                PosturePlugin.TRUE,
                PosturePlugin.HALF_OPEN,
                listOf(tabletop),
            )

        assertEquals(PosturePlugin.FALSE, ByteBuffer.wrap(open).order(ByteOrder.LITTLE_ENDIAN).getInt(24))
        assertEquals(PosturePlugin.TRUE, ByteBuffer.wrap(half).order(ByteOrder.LITTLE_ENDIAN).getInt(24))
        assertTrue(open.size == half.size)
    }

    @Test
    fun dualScreenAndTrifoldCapAtMaxHinges() {
        val hinge =
            PosturePlugin.EncodedHinge(
                fold = PosturePlugin.OPEN,
                axis = PosturePlugin.VERTICAL,
                occluding = PosturePlugin.TRUE,
                separating = PosturePlugin.TRUE,
                x = 0.25f,
                y = 0f,
                width = 0f,
                height = 1f,
                angle = PosturePlugin.NO_ANGLE,
            )
        val hinges = List(PosturePlugin.MAX_HINGES) { index ->
            hinge.copy(x = 0.2f * (index + 1))
        }
        val bytes = PosturePlugin.encodePosture(PosturePlugin.TRUE, PosturePlugin.OPEN, hinges)
        assertEquals(
            PosturePlugin.HEADER_BYTES + PosturePlugin.HINGE_BYTES * PosturePlugin.MAX_HINGES,
            bytes.size,
        )
        assertEquals(PosturePlugin.MAX_HINGES, ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN).getInt(8))
    }

    @Test
    fun rigidDeviceIsATwelveByteHeader() {
        val bytes = PosturePlugin.encodePosture(PosturePlugin.FALSE, PosturePlugin.UNKNOWN, emptyList())
        assertArrayEquals(
            byteArrayOf(
                1,
                0,
                0,
                0, // false foldable
                0,
                0,
                0,
                0, // unknown fold
                0,
                0,
                0,
                0, // no hinges
            ),
            bytes,
        )
    }
}
