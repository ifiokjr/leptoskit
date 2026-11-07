// Experimental Wear OS tile host stub for leptoskit.
//
// A future generated Wear OS app will extend androidx.wear.tiles.TileService
// and ask the Wasm guest for the WatchTile surface declared in Rust. This
// file is a scaffold only: it does not ship in an APK yet.

package dev.leptoskit.wearos

/**
 * Placeholder for the Wear OS TileService entry point.
 *
 * Manifest requirements when this is wired:
 * - `<service android:name=".LeptoskitTileService" …>` with
 *   `androidx.wear.tiles.action.BIND_TILE_PROVIDER`
 * - `com.google.android.wearable.feature.TILES` metadata as required by the
 *   Wear Tiles library version you depend on
 */
class LeptoskitTileServicePlaceholder {
	companion object {
		const val SURFACE = "tile"
		const val STATUS = "experimental-stub"
	}
}
