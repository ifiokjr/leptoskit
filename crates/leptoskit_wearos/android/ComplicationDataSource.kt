// Experimental Wear OS complication data-source stub for leptoskit.
package dev.leptoskit.wearos

/**
 * Placeholder for a Wear complication data source.
 *
 * Manifest requirements when this is wired:
 * - service with `android.support.wearable.complications.ACTION_COMPLICATION_UPDATE_REQUEST`
 *   (or the androidx.wear.watchface equivalent for your dependency set)
 * - complication types matching [leptoskit_watch::ComplicationFamily]
 */
class LeptoskitComplicationDataSourcePlaceholder {
	companion object {
		const val SURFACE = "complication"
		const val STATUS = "experimental-stub"
	}
}
