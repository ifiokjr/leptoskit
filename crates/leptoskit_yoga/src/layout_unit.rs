//! Chrome/Blink-compatible fixed-point layout units.
//!
//! Blink stores most geometry as `LayoutUnit` with a 1/64 CSS-pixel
//! denominator. Flexible track sizing carries each track's truncated
//! fractional remainder into the next track using **binary32** arithmetic
//! (matching C++ `float`), which is what makes `getBoundingClientRect`
//! report patterns like `74.65625 / 74.671875 / 74.65625` instead of
//! evenly-floored thirds. Percentage lengths use the same denominator.

/// Sub-pixels per CSS pixel, matching Blink's `kFixedPointDenominator`.
pub(crate) const LAYOUT_UNIT_SCALE: i64 = 64;

/// Rounds a CSS-pixel length to Blink's `LayoutUnit::FromFloatRounded`.
pub(crate) fn to_layout_unit(pixels: f32) -> i64 {
	if !pixels.is_finite() {
		return 0;
	}

	(f64::from(pixels) * LAYOUT_UNIT_SCALE as f64).round() as i64
}

/// Floors a CSS-pixel length into a `LayoutUnit`, matching
/// `LayoutUnit(double)` truncation used while distributing `fr` shares.
pub(crate) fn floor_to_layout_unit(pixels: f32) -> i64 {
	if !pixels.is_finite() {
		return 0;
	}

	(f64::from(pixels) * LAYOUT_UNIT_SCALE as f64).floor() as i64
}

/// Converts a layout unit back to CSS pixels.
pub(crate) fn from_layout_unit(units: i64) -> f32 {
	units as f32 / LAYOUT_UNIT_SCALE as f32
}

/// Resolves a percentage of a definite basis.
///
/// Whole-number percentages (`50%`, `90%`, `5%`) use integer `LayoutUnit`
/// math so `90%` of `160` stays exact `144`. Non-integral percentages such as
/// Tailwind's `w-1/3` (`33.333334%`) floor the product into a `LayoutUnit`,
/// matching Chrome.
pub(crate) fn percent_of(basis: f32, fraction: f32) -> f32 {
	if !basis.is_finite() || !fraction.is_finite() {
		return 0.0;
	}

	let fraction = crate::finite(fraction);
	let percent = f64::from(fraction) * 100.0;
	let nearest = percent.round();

	if (percent - nearest).abs() < 1e-4 {
		let basis_lu = to_layout_unit(basis);
		return from_layout_unit(basis_lu * nearest as i64 / 100);
	}

	from_layout_unit(floor_to_layout_unit(basis * fraction).max(0))
}

/// Distributes `space` CSS pixels across proportional `weights` the way Blink
/// sizes flexible grid tracks: each share is floored to a `LayoutUnit` and the
/// truncated fractional remainder is carried into the next track in `f32`
/// arithmetic.
pub(crate) fn distribute_weighted(space: f32, weights: &[f32]) -> Vec<f32> {
	let count = weights.len();

	if count == 0 {
		return Vec::new();
	}

	if !space.is_finite() || space <= 0.0 {
		return vec![0.0; count];
	}

	let mut weight_sum = 0.0_f32;

	for weight in weights {
		weight_sum += crate::finite(*weight).max(0.0);
	}

	if weight_sum <= 0.0 {
		return vec![0.0; count];
	}

	// Match C++ `float` flexFraction = space / sum(weights).
	let flex_fraction = space / weight_sum;
	let mut leftover = 0.0_f32;
	let mut sizes = Vec::with_capacity(count);

	for weight in weights {
		let weight = crate::finite(*weight).max(0.0);
		let share = flex_fraction * weight + leftover;
		let units = floor_to_layout_unit(share).max(0);
		let used = from_layout_unit(units);
		leftover = (share - used).max(0.0);
		sizes.push(used);
	}

	sizes
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
	use super::*;

	#[test]
	fn equal_fr_tracks_match_chrome_including_lost_layout_units() {
		assert_eq!(
			distribute_weighted(224.0, &[1.0, 1.0, 1.0]),
			vec![
				from_layout_unit(4_778),
				from_layout_unit(4_779),
				from_layout_unit(4_778),
			]
		);
		assert_eq!(
			distribute_weighted(328.0, &[1.0, 1.0, 1.0]),
			vec![
				from_layout_unit(6_997),
				from_layout_unit(6_997),
				from_layout_unit(6_998),
			]
		);
		assert_eq!(
			distribute_weighted(100.0, &[1.0, 1.0, 1.0]),
			vec![
				from_layout_unit(2_133),
				from_layout_unit(2_133),
				from_layout_unit(2_133),
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
	fn percent_nice_values_stay_exact() {
		assert_eq!(percent_of(160.0, 0.9), 144.0);
		assert_eq!(percent_of(320.0, 0.5), 160.0);
		assert_eq!(percent_of(378.0, 0.05), from_layout_unit(1_209));
	}

	#[test]
	fn percent_thirds_floor_like_chrome() {
		assert_eq!(percent_of(320.0, 1.0 / 3.0), from_layout_unit(6_826));
	}
}
