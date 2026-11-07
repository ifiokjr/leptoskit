// Experimental watchOS app host stub for leptoskit.
//
// A future generated Watch App will load the Wasm guest and present WatchPage
// surfaces through WatchKit / SwiftUI. This file is a scaffold only.

import Foundation

/// Placeholder for the interactive watchOS app entry.
///
/// Entitlements when this is wired:
/// - Watch App target (optionally paired with an iOS companion)
/// - App Groups for phone ↔ watch data, when shared
/// - HealthKit or other capabilities only when a surface needs them
enum LeptoskitWatchAppPlaceholder {
	static let surface = "page"
	static let status = "experimental-stub"
}
