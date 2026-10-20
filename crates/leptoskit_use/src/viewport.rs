use std::collections::BTreeMap;

use reactive_graph::prelude::*;
use reactive_graph::wrappers::read::Signal;

/// Logical viewport dimensions, independent of physical pixel density.
///
/// The default integer type preserves browser-style dimensions. Use `f32` for
/// the fractional logical pixels supplied by native surfaces and Tailwind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Viewport<T = u32> {
	/// Logical width.
	pub width: T,
	/// Logical height.
	pub height: T,
}

/// Reactive viewport dimensions supplied by the host.
#[derive(Debug, Clone, Copy)]
pub struct UseWindowSizeReturn<T: Send + Sync + 'static = u32> {
	/// Logical viewport width.
	pub width: Signal<T>,
	/// Logical viewport height.
	pub height: Signal<T>,
}

/// Project a host's viewport signal into width and height signals.
///
/// Native hosts feed layout size; browser hosts feed their resize observation;
/// SSR can provide the intended viewport. No global window is accessed.
pub fn use_window_size<T>(viewport: impl Into<Signal<Viewport<T>>>) -> UseWindowSizeReturn<T>
where
	T: Clone + Send + Sync + 'static,
{
	let viewport = viewport.into();
	UseWindowSizeReturn {
		width: Signal::derive(move || viewport.with(|v| v.width.clone())),
		height: Signal::derive(move || viewport.with(|v| v.height.clone())),
	}
}

/// Named minimum widths against a host-provided logical viewport width.
#[derive(Debug, Clone)]
pub struct UseBreakpointsReturn<K, T: Send + Sync + 'static = u32> {
	width: Signal<T>,
	points: BTreeMap<K, T>,
}

/// Create reactive breakpoint queries. Unknown keys return `None`.
/// Integer and fractional logical widths use the host's original units.
pub fn use_breakpoints<K, T>(
	points: BTreeMap<K, T>,
	width: impl Into<Signal<T>>,
) -> UseBreakpointsReturn<K, T>
where
	T: Send + Sync + 'static,
{
	UseBreakpointsReturn {
		width: width.into(),
		points,
	}
}

impl<K: Ord, T: Clone + PartialOrd + Send + Sync + 'static> UseBreakpointsReturn<K, T> {
	/// Whether width is greater than or equal to the named minimum.
	pub fn greater_or_equal(&self, key: &K) -> Option<Signal<bool>> {
		let at = self.points.get(key)?.clone();
		let width = self.width;
		Some(Signal::derive(move || width.with(|value| value >= &at)))
	}

	/// Whether width is strictly below the named minimum.
	pub fn smaller(&self, key: &K) -> Option<Signal<bool>> {
		let at = self.points.get(key)?.clone();
		let width = self.width;
		Some(Signal::derive(move || width.with(|value| value < &at)))
	}

	/// Whether width lies in `[lower, upper)`. Reversed bounds match nothing.
	pub fn between(&self, lower: &K, upper: &K) -> Option<Signal<bool>> {
		let lower = self.points.get(lower)?.clone();
		let upper = self.points.get(upper)?.clone();
		let width = self.width;
		Some(Signal::derive(move || {
			width.with(|value| &lower <= value && value < &upper)
		}))
	}
}
