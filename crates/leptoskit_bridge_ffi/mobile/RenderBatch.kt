package dev.leptoskit.mobile

import org.json.JSONObject
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * One widget's properties, as both render transaction forms carry them. The
 * renderer applies the typed fields to every view; [rest] holds any other
 * property that is set, such as a drawing or ARIA state, in the JSON form.
 */
internal class WidgetProps(
    val text: String? = null,
    val source: String? = null,
    val placeholder: String? = null,
    val value: String? = null,
    val accessibilityLabel: String? = null,
    /** RGBA, eight bits a channel. */
    val color: Long? = null,
    val background: Long? = null,
    val borderColor: Long? = null,
    val enabled: Boolean? = null,
    val hidden: Boolean? = null,
    val clip: Boolean? = null,
    val cornerRadius: Double? = null,
    val borderWidth: Double? = null,
    val opacity: Double? = null,
    val fontSize: Double? = null,
    val fontWeight: String? = null,
    val textAlign: String? = null,
    val rest: JSONObject? = null,
) {
    /** Where a scroll view's content is scrolled to; it travels in [rest]. */
    val scrollOffset: ScrollOffset? = rest?.optJSONObject("scroll_offset")?.let(ScrollOffset::fromJson)

    companion object {
        /** Missing and JSON-null properties are both unset. */
        fun fromJson(props: JSONObject): WidgetProps {
            fun string(key: String) = if (props.isNull(key)) null else props.getString(key)

            fun long(key: String) = if (props.isNull(key)) null else props.getLong(key)

            fun double(key: String) = if (props.isNull(key)) null else props.getDouble(key)

            fun boolean(key: String) = if (props.isNull(key)) null else props.getBoolean(key)

            return WidgetProps(
                text = string("text"),
                source = string("source"),
                placeholder = string("placeholder"),
                value = string("value"),
                accessibilityLabel = string("accessibility_label"),
                color = long("color"),
                background = long("background"),
                borderColor = long("border_color"),
                enabled = boolean("enabled"),
                hidden = boolean("hidden"),
                clip = boolean("clip"),
                cornerRadius = double("corner_radius"),
                borderWidth = double("border_width"),
                opacity = double("opacity"),
                fontSize = double("font_size"),
                fontWeight = string("font_weight"),
                textAlign = string("text_align"),
                rest = props,
            )
        }
    }
}

/** Where a scroll view's content is scrolled to, in layout units, as `Element::scroll_offset` sets it. */
internal data class ScrollOffset(
    val x: Double,
    val y: Double,
    val animated: Boolean,
) {
    companion object {
        fun fromJson(offset: JSONObject) = ScrollOffset(offset.getDouble("x"), offset.getDouble("y"), offset.optBoolean("animated"))
    }
}

/** One change to the native tree. */
internal sealed interface RenderMutation {
    class Create(
        val id: Long,
        val kind: String,
        val props: WidgetProps,
    ) : RenderMutation

    class UpdateProps(
        val id: Long,
        val props: WidgetProps,
    ) : RenderMutation

    class UpdateLayout(
        val id: Long,
        val x: Double,
        val y: Double,
        val width: Double,
        val height: Double,
    ) : RenderMutation

    class InsertChild(
        val parent: Long,
        val child: Long,
        val index: Int,
    ) : RenderMutation

    class RemoveChild(
        val parent: Long,
        val child: Long,
    ) : RenderMutation

    class Delete(
        val id: Long,
    ) : RenderMutation
}

/** One committed render: its changes, and the root when it changed. */
internal class RenderBatch(
    val root: Long?,
    val mutations: List<RenderMutation>,
) {
    companion object {
        /** The JSON transaction `lk_app_render` sends. */
        fun fromJson(batch: JSONObject): RenderBatch {
            check(batch.getInt("version") == 1)
            val changes = batch.getJSONArray("mutations")
            val mutations =
                List(changes.length()) { index ->
                    val change = changes.getJSONObject(index)
                    val name = change.keys().next()
                    val item = change.getJSONObject(name)

                    when (name) {
                        "Create" -> {
                            RenderMutation.Create(
                                item.getLong("id"),
                                item.getString("kind"),
                                WidgetProps.fromJson(item.getJSONObject("props")),
                            )
                        }

                        "UpdateProps" -> {
                            RenderMutation.UpdateProps(item.getLong("id"), WidgetProps.fromJson(item.getJSONObject("props")))
                        }

                        "UpdateLayout" -> {
                            RenderMutation.UpdateLayout(
                                item.getLong("id"),
                                item.getDouble("x"),
                                item.getDouble("y"),
                                item.getDouble("width"),
                                item.getDouble("height"),
                            )
                        }

                        "InsertChild" -> {
                            RenderMutation.InsertChild(item.getLong("parent"), item.getLong("child"), item.getInt("index"))
                        }

                        "RemoveChild" -> {
                            RenderMutation.RemoveChild(item.getLong("parent"), item.getLong("child"))
                        }

                        "Delete" -> {
                            RenderMutation.Delete(item.getLong("id"))
                        }

                        else -> {
                            error("Unknown render mutation: $name")
                        }
                    }
                }

            return RenderBatch(if (batch.isNull("root")) null else batch.getLong("root"), mutations)
        }

        /**
         * The binary transaction `lk_app_render_binary` sends, laid out as the
         * Rust bridge's `binary` module documents. Parsing JSON into a tree of
         * objects, then looking each property up by name, was a large part of
         * applying a screen.
         */
        fun fromBinary(bytes: ByteArray): RenderBatch {
            val reader = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
            check(bytes.size >= HEADER && String(bytes, 0, MAGIC.length, Charsets.US_ASCII) == MAGIC) {
                "Malformed render transaction"
            }
            reader.position(MAGIC.length)
            val root = reader.long
            val mutations = List(reader.int) { reader.mutation(bytes) }
            check(!reader.hasRemaining()) { "Malformed render transaction: trailing bytes" }

            return RenderBatch(if (root < 0) null else root, mutations)
        }

        private const val MAGIC = "LKB1"
        private const val HEADER = 16
        private const val U32 = 0xffff_ffffL
        private val FONT_WEIGHTS = arrayOf("regular", "medium", "semibold", "bold")
        private val TEXT_ALIGNS = arrayOf("start", "center", "end")

        private fun ByteBuffer.id(): Long = int.toLong() and U32

        private fun ByteBuffer.string(bytes: ByteArray): String {
            val length = int
            check(length in 0..remaining()) { "Malformed render transaction: string length" }
            val text = String(bytes, position(), length, Charsets.UTF_8)
            position(position() + length)

            return text
        }

        private fun ByteBuffer.mutation(bytes: ByteArray): RenderMutation =
            when (val op = get().toInt()) {
                1 -> {
                    val id = id()
                    val length = get().toInt() and 0xff
                    val kind = String(bytes, position(), length, Charsets.UTF_8)
                    position(position() + length)
                    RenderMutation.Create(id, kind, props(bytes))
                }

                2 -> {
                    RenderMutation.UpdateProps(id(), props(bytes))
                }

                3 -> {
                    RenderMutation.UpdateLayout(
                        id(),
                        float.toDouble(),
                        float.toDouble(),
                        float.toDouble(),
                        float.toDouble(),
                    )
                }

                4 -> {
                    RenderMutation.InsertChild(id(), id(), int)
                }

                5 -> {
                    RenderMutation.RemoveChild(id(), id())
                }

                6 -> {
                    RenderMutation.Delete(id())
                }

                else -> {
                    error("Unknown render mutation: $op")
                }
            }

        private fun ByteBuffer.props(bytes: ByteArray): WidgetProps {
            val present = int

            fun has(bit: Int) = present and (1 shl bit) != 0

            fun color(bit: Int) = if (has(bit)) int.toLong() and U32 else null

            return WidgetProps(
                text = if (has(0)) string(bytes) else null,
                source = if (has(1)) string(bytes) else null,
                placeholder = if (has(2)) string(bytes) else null,
                value = if (has(3)) string(bytes) else null,
                accessibilityLabel = if (has(4)) string(bytes) else null,
                color = color(5),
                background = color(6),
                borderColor = color(7),
                enabled = if (has(8)) get().toInt() != 0 else null,
                hidden = if (has(9)) get().toInt() != 0 else null,
                clip = if (has(10)) get().toInt() != 0 else null,
                cornerRadius = if (has(11)) float.toDouble() else null,
                borderWidth = if (has(12)) float.toDouble() else null,
                opacity = if (has(13)) float.toDouble() else null,
                fontSize = if (has(14)) float.toDouble() else null,
                fontWeight = if (has(15)) FONT_WEIGHTS[get().toInt()] else null,
                textAlign = if (has(16)) TEXT_ALIGNS[get().toInt()] else null,
                rest = if (has(31)) JSONObject(string(bytes)) else null,
            )
        }
    }
}
