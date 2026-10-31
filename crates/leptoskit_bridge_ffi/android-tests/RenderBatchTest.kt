package dev.leptoskit.mobile

import android.util.Base64
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

/**
 * The binary render transaction decodes to what the JSON one does, for the
 * fixture the Rust encoder's tests write: every typed property, properties
 * sent in the JSON form, and every kind of mutation.
 */
class RenderBatchTest {
    private val binary = Base64.decode(RENDER_TRANSACTION_BINARY, Base64.DEFAULT)

    @Test
    fun decodesWhatTheJsonTransactionDecodes() {
        val expected = RenderBatch.fromJson(JSONObject(RENDER_TRANSACTION_JSON))
        val decoded = RenderBatch.fromBinary(binary)

        assertEquals(expected.root, decoded.root)
        assertEquals(expected.mutations.map(::describe), decoded.mutations.map(::describe))
    }

    @Test
    fun rejectsATruncatedTransaction() {
        for (length in listOf(0, 3, 12, 16, binary.size / 2, binary.size - 1)) {
            assertThrows(Exception::class.java) { RenderBatch.fromBinary(binary.copyOf(length)) }
        }
    }

    private fun describe(mutation: RenderMutation): String =
        when (mutation) {
            is RenderMutation.Create -> {
                "create ${mutation.id} ${mutation.kind} ${describe(mutation.props)}"
            }

            is RenderMutation.UpdateProps -> {
                "props ${mutation.id} ${describe(mutation.props)}"
            }

            is RenderMutation.UpdateLayout -> {
                "layout ${mutation.id} ${mutation.x} ${mutation.y} ${mutation.width} ${mutation.height}"
            }

            is RenderMutation.InsertChild -> {
                "insert ${mutation.parent} ${mutation.child} ${mutation.index}"
            }

            is RenderMutation.RemoveChild -> {
                "remove ${mutation.parent} ${mutation.child}"
            }

            is RenderMutation.Delete -> {
                "delete ${mutation.id}"
            }
        }

    /**
     * Typed fields as they are; of the rest, only properties no typed field
     * carries. The scroll offset is typed, read from the rest, so its JSON,
     * whose key order differs between the two forms, is left out.
     */
    private fun describe(props: WidgetProps): String {
        val typed =
            listOf(
                props.text,
                props.source,
                props.placeholder,
                props.value,
                props.accessibilityLabel,
                props.color,
                props.background,
                props.borderColor,
                props.enabled,
                props.hidden,
                props.clip,
                props.cornerRadius,
                props.borderWidth,
                props.opacity,
                props.fontSize,
                props.fontWeight,
                props.textAlign,
                props.scrollOffset,
            )
        val rest = props.rest
        val others =
            rest
                ?.keys()
                ?.asSequence()
                ?.filter { it !in TYPED_KEYS && !rest.isNull(it) }
                ?.sorted()
                ?.joinToString { "$it=${rest.get(it)}" }

        return "$typed {${others.orEmpty()}}"
    }

    private companion object {
        val TYPED_KEYS =
            setOf(
                "text",
                "source",
                "placeholder",
                "value",
                "accessibility_label",
                "color",
                "background",
                "border_color",
                "enabled",
                "hidden",
                "clip",
                "corner_radius",
                "border_width",
                "opacity",
                "font_size",
                "font_weight",
                "text_align",
                "scroll_offset",
            )
    }
}
