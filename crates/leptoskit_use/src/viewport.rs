use std::collections::BTreeMap;

use reactive_graph::prelude::*;
use reactive_graph::wrappers::read::Signal;

/// Logical viewport dimensions, independent of physical pixel density.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Viewport {
	/// Logical width.
	pub width: u32,
	/// Logical height.
	pub height: u32,
}

/// Reactive viewport dimensions supplied by the host.
#[derive(Debug, Clone, Copy)]
pub struct UseWindowSizeReturn {
	/// Logical viewport width.
	pub width: Signal<u32>,
	/// Logical viewport height.
	pub height: Signal<u32>,
}

/// Project a host's viewport signal into width and height signals.
///
/// Native hosts feed layout size; browser hosts feed their resize observation;
/// SSR can provide the intended viewport. No global window is accessed.
pub fn use_window_size(viewport: impl Into<Signal<Viewport>>) -> UseWindowSizeReturn {
	let viewport = viewport.into();
	UseWindowSizeReturn {
		width: Signal::derive(move || viewport.with(|v| v.width)),
		height: Signal::derive(move || viewport.with(|v| v.height)),
	}
}

/// Named minimum widths against a host-provided logical viewport width.
#[derive(Debug, Clone)]
pub struct UseBreakpointsReturn<K> {
	width: Signal<u32>,
	points: BTreeMap<K, u32>,
}

/// Create reactive breakpoint queries. Unknown keys return `None`.
pub fn use_breakpoints<K>(
	points: BTreeMap<K, u32>,
	width: impl Into<Signal<u32>>,
) -> UseBreakpointsReturn<K> {
	UseBreakpointsReturn {
		width: width.into(),
		points,
	}
}

impl<K: Ord> UseBreakpointsReturn<K> {
	/// Whether width is greater than or equal to the named minimum.
	pub fn greater_or_equal(&self, key: &K) -> Option<Signal<bool>> {
		let at = *self.points.get(key)?;
		let width = self.width;
		Some(Signal::derive(move || width.get() >= at))
	}

	/// Whether width is strictly below the named minimum.
	pub fn smaller(&self, key: &K) -> Option<Signal<bool>> {
		let at = *self.points.get(key)?;
		let width = self.width;
		Some(Signal::derive(move || width.get() < at))
	}

	/// Whether width lies in `[lower, upper)`. Reversed bounds match nothing.
	pub fn between(&self, lower: &K, upper: &K) -> Option<Signal<bool>> {
		let lower = *self.points.get(lower)?;
		let upper = *self.points.get(upper)?;
		let width = self.width;
		Some(Signal::derive(move || {
			(lower..upper).contains(&width.get())
		}))
	}
}
