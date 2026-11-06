//! Chrome/Blink-compatible fixed-point layout units.
//!
//! Blink stores most geometry as `LayoutUnit` with a 1/64 CSS-pixel
//! denominator. Distributing free space and resolving percentages through
//! that fixed point is what makes `getBoundingClientRect` report values like
//! `109.34375` instead of `109.333…`. Matching it keeps Yoga frames
//! byte-identical to Chrome for the same class trees.

/// Sub-pixels per CSS pixel, matching Blink's `kFixedPointDenominator`.
pub(crate) const LAYOUT_UNIT_SCALE: i64 = 64;

/// Rounds a CSS-pixel length to Blink's `LayoutUnit::FromFloatRounded`.
pub(crate) fn to_layout_unit(pixels: f32) -> i64 {
	if !pixels.is_finite() {
		return 0;
	}

	(f64::from(pixels) * LAYOUT_UNIT_SCALE as f64).round() as i64
}

/// Converts a layout unit back to CSS pixels.
pub(crate) fn from_layout_unit(units: i64) -> f32 {
	units as f32 / LAYOUT_UNIT_SCALE as f32
}

/// Resolves a percentage of a definite basis through Blink's
/// `LayoutUnit::FromFloatRounded`, so values like `90%` of `160` stay exact
/// `144` even when `0.9` is not binary-exact in `f32`.
pub(crate) fn percent_of(basis: f32, fraction: f32) -> f32 {
	if !basis.is_finite() || !fraction.is_finite() {
		return 0.0;
	}

	from_layout_unit(to_layout_unit(
		basis * crate::finite(fraction),
	))
}

/// Distributes `space` CSS pixels across proportional `weights`, truncating
/// each share but the last so the pieces always sum to `space` after
/// `LayoutUnit` rounding. Remainder layout units land on the last weight,
/// matching Chrome's equal-`fr` and weighted-`fr` track sizes.
pub(crate) fn distribute_weighted(space: f32, weights: &[f32]) -> Vec<f32> {
	let count = weights.len();

	if count == 0 {
		return Vec::new();
	}

	let total_lu = to_layout_unit(space).max(0);
	let weight_sum: f64 = weights
		.iter()
		.map(|weight| f64::from(crate::finite(*weight)).max(0.0))
		.sum();

	if weight_sum <= 0.0 || total_lu == 0 {
		return vec![0.0; count];
	}

	let mut sizes = vec![0.0; count];
	let mut used = 0i64;

	for (index, weight) in weights.iter().enumerate() {
		let weight = f64::from(crate::finite(*weight)).max(0.0);

		if index + 1 == count {
			sizes[index] = from_layout_unit(total_lu - used);

			break;
		}

		let share = ((total_lu as f64) * (weight / weight_sum)).floor() as i64;
		let share = share.clamp(0, total_lu - used);
		sizes[index] = from_layout_unit(share);
		used += share;
	}

	sizes
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
	use super::*;

	#[test]
	fn equal_fr_tracks_match_chrome_thirds() {
		assert_eq!(
			distribute_weighted(328.0, &[1.0, 1.0, 1.0]),
			vec![
				from_layout_unit(6_997),
				from_layout_unit(6_997),
				from_layout_unit(6_998),
			]
		);
	}

	#[test]
	fn weighted_fr_tracks_put_remainder_on_the_last() {
		assert_eq!(
			distribute_weighted(316.0, &[1.0, 2.0]),
			vec![from_layout_unit(6_741), from_layout_unit(13_483)]
		);
	}

	#[test]
	fn percent_nice_values_round_to_exact_layout_units() {
		assert_eq!(percent_of(160.0, 0.9), 144.0);
		assert_eq!(percent_of(320.0, 0.5), 160.0);
	}
}
