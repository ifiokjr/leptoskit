// Experimental Wear OS activity host stub for leptoskit.
package dev.leptoskit.wearos

/**
 * Placeholder for the interactive watch app activity.
 *
 * Manifest requirements when this is wired:
 * - `android.hardware.type.watch` uses-feature
 * - launcher activity with Wear category
 * - optional phone companion deep links
 */
class LeptoskitWatchActivityPlaceholder {
	companion object {
		const val SURFACE = "page"
		const val STATUS = "experimental-stub"
	}
}
