@file:Suppress("ktlint:standard:max-line-length")

package dev.leptoskit.mobile

// Written by the bridge's `binary` tests; rerun them with
// LEPTOSKIT_UPDATE_FIXTURES=1 after changing the format.
/** The sample render transaction in the binary form, base64 encoded. */
internal const val RENDER_TRANSACTION_BINARY = "TEtCMQAAAAAAAAAACAAAAAEAAAAAC3Njcm9sbF92aWV3AAAAAAEBAAAABHRleHT//wGADgAAAENhcmQgNyDigJQg4pyTEQAAAGZpbGU6Ly8vaW1hZ2UucG5nBgAAAFNlYXJjaAUAAAB0eXBlZAwAAABTZXZlbnRoIGNhcmT/MyIRAP///4AAAAAAAQEAAAhBAACAPwAAgD4AAFBBAgJgAAAAeyJjaGVja2VkIjp0cnVlLCJpZCI6ImNhcmQtNyIsIml0YWxpYyI6dHJ1ZSwic2Nyb2xsX29mZnNldCI6eyJ4IjowLjAsInkiOjEyMC41LCJhbmltYXRlZCI6dHJ1ZX19AQIAAAAIbWFwX3ZpZXcBAAAACQAAAG9ubHkgdGV4dAICAAAAAIABAAMBAwEAAAAAAIBBAAAgwAAgs0MAAIhBBAAAAAABAAAAAAAAAAUAAAAAAwAAAAYDAAAA"

/** The same transaction in the JSON form. */
internal const val RENDER_TRANSACTION_JSON = """{"mutations":[{"Create":{"id":0,"kind":"scroll_view","props":{}}},{"Create":{"id":1,"kind":"text","props":{"accessibility_label":"Seventh card","background":4294967040,"border_color":128,"border_width":1.0,"checked":true,"clip":true,"color":287454207,"corner_radius":8.5,"enabled":false,"font_size":13.0,"font_weight":"semibold","hidden":true,"id":"card-7","italic":true,"opacity":0.25,"placeholder":"Search","scroll_offset":{"animated":true,"x":0.0,"y":120.5},"source":"file:///image.png","text":"Card 7 — ✓","text_align":"end","value":"typed"}}},{"Create":{"id":2,"kind":"map_view","props":{"text":"only text"}}},{"UpdateProps":{"id":2,"props":{"font_weight":"bold","text_align":"center"}}},{"UpdateLayout":{"height":17.0,"id":1,"width":358.25,"x":16.0,"y":-2.5}},{"InsertChild":{"child":1,"index":0,"parent":0}},{"RemoveChild":{"child":3,"parent":0}},{"Delete":{"id":3}}],"root":0,"version":1}"""
