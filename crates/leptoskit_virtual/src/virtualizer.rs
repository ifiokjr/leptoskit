//! The windowing math behind [`leptoskit_virtual`](crate): one axis-agnostic
//! core, following `TanStack Virtual`'s model.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use parking_lot::Mutex;

/// A size estimate that is not a finite, positive number of layout units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InvalidEstimate {
	/// The index whose estimate failed.
	pub index: usize,
	/// The invalid value.
	pub value: f64,
}

impl fmt::Display for InvalidEstimate {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(
			formatter,
			"the size estimate for item {} is {}, not a finite positive number of layout units",
			self.index, self.value
		)
	}
}

impl std::error::Error for InvalidEstimate {}

/// An index this virtualizer has no item for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutOfRange {
	/// The requested index.
	pub index: usize,
	/// The number of items.
	pub count: usize,
}

impl fmt::Display for OutOfRange {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(
			formatter,
			"index {} is outside 0..{}",
			self.index, self.count
		)
	}
}

impl std::error::Error for OutOfRange {}

/// A measurement this virtualizer rejects.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MeasureError {
	/// The index is outside `0..count`.
	OutOfRange(OutOfRange),
	/// The size is not a finite, positive number of layout units.
	InvalidSize {
		/// The item being measured.
		index: usize,
		/// The rejected size.
		value: f64,
	},
}

impl fmt::Display for MeasureError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::OutOfRange(error) => write!(formatter, "{error}"),
			Self::InvalidSize { index, value } => {
				write!(
					formatter,
					"the measured size for item {index} is {value}, not a finite positive number \
					 of layout units"
				)
			}
		}
	}
}

impl std::error::Error for MeasureError {}

/// Where a scrolled-to item should land in the viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
	/// The least movement that shows the item; the current offset when it is
	/// already visible.
	#[default]
	Auto,
	/// The item's start at the viewport's start.
	Start,
	/// The item centered in the viewport.
	Center,
	/// The item's end at the viewport's end.
	End,
}

/// One item to render: its index, start offset, and size, in layout units
/// along the scrolling axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VirtualItem {
	/// The item's index in `0..count`.
	pub index: usize,
	/// The offset of the item's start edge from the content's start.
	pub start: f64,
	/// The item's size along the scrolling axis.
	pub size: f64,
}

/// A size estimate: fixed, or a function of the item's index.
#[derive(Clone)]
pub enum Estimate {
	/// One size for every item.
	Fixed(f64),
	/// A size per index, such as a lookup by item kind.
	PerItem(Arc<dyn Fn(usize) -> f64 + Send + Sync>),
}

impl fmt::Debug for Estimate {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Fixed(size) => formatter.debug_tuple("Fixed").field(size).finish(),
			Self::PerItem(_) => formatter.write_str("PerItem(..)"),
		}
	}
}

impl Estimate {
	fn size_of(&self, index: usize) -> f64 {
		match self {
			Self::Fixed(size) => *size,
			Self::PerItem(estimate) => estimate(index),
		}
	}
}

/// Configuration for one [`Virtualizer`].
#[derive(Debug, Clone)]
pub struct VirtualOptions {
	/// How many items the list holds.
	pub count: usize,
	/// The size estimate for unmeasured items.
	pub estimate: Estimate,
	/// Extra items rendered on each side of the viewport, absorbing scroll
	/// velocity cheaply.
	pub overscan: usize,
	/// Layout units of content before the first item.
	pub padding_start: f64,
	/// Layout units of content after the last item.
	pub padding_end: f64,
}

impl VirtualOptions {
	/// Options for `count` items of one fixed size, without padding or
	/// overscan.
	pub fn fixed(count: usize, size: f64) -> Self {
		Self {
			count,
			estimate: Estimate::Fixed(size),
			overscan: 0,
			padding_start: 0.0,
			padding_end: 0.0,
		}
	}

	/// Sets the overscan.
	#[must_use]
	pub fn with_overscan(mut self, overscan: usize) -> Self {
		self.overscan = overscan;
		self
	}
}

/// The resolved position of every item, cumulative from the padding start.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Measurement {
	start: f64,
	size: f64,
	end: f64,
}

/// The lazily rebuilt measurement table.
#[derive(Debug, Default)]
struct Table {
	measurements: Vec<Measurement>,
	dirty: bool,
}

/// Headless list virtualization: which items intersect a scrolled viewport.
///
/// The virtualizer is pure windowing math, in the style of `TanStack Virtual`.
/// Feed it the scroll offset and viewport size the surface reports; it returns
/// the items to render and the content size that keeps the scrollbar honest.
/// Sizes come from the estimate until [`measure`](Self::measure) supplies the
/// real one, and a changed measurement shifts every item after it.
///
/// The axis is the caller's choice: offsets and sizes apply to whichever axis
/// the scroll container scrolls; [`VirtualList`](crate::VirtualList) wires
/// the vertical and horizontal containers.
///
/// ```
/// use leptoskit_virtual::Align;
/// use leptoskit_virtual::VirtualOptions;
/// use leptoskit_virtual::Virtualizer;
///
/// let mut virtualizer = Virtualizer::new(VirtualOptions::fixed(1000, 50.0));
/// virtualizer.set_viewport(400.0);
/// virtualizer.scroll_to(10_000.0);
///
/// let items = virtualizer.items().expect("a fixed estimate is valid");
/// assert_eq!(items.first().map(|item| item.index), Some(200));
/// assert_eq!(items.last().map(|item| item.index), Some(207));
/// assert_eq!(virtualizer.total_size(), Ok(50_000.0));
///
/// // Programmatic scrolling lands where it should.
/// assert_eq!(
/// 	virtualizer.offset_for_index(500, Align::Start),
/// 	Ok(25_000.0)
/// );
/// ```
#[derive(Debug)]
pub struct Virtualizer {
	count: usize,
	estimate: Estimate,
	overscan: usize,
	padding_start: f64,
	padding_end: f64,
	measured: HashMap<usize, f64>,
	scroll_offset: f64,
	viewport: f64,
	table: Mutex<Table>,
}

impl Virtualizer {
	/// A virtualizer over the options' items.
	///
	/// Sizes resolve lazily, so an invalid estimate surfaces from
	/// [`items`](Self::items), [`total_size`](Self::total_size), and
	/// [`offset_for_index`](Self::offset_for_index) instead — the error names
	/// the offending index.
	pub fn new(options: VirtualOptions) -> Self {
		Self {
			count: options.count,
			estimate: options.estimate,
			overscan: options.overscan,
			padding_start: options.padding_start,
			padding_end: options.padding_end,
			measured: HashMap::new(),
			scroll_offset: 0.0,
			viewport: 0.0,
			table: Mutex::new(Table {
				measurements: Vec::with_capacity(options.count),
				dirty: true,
			}),
		}
	}

	/// The configured item count.
	pub fn count(&self) -> usize {
		self.count
	}

	/// Records the viewport's size along the scrolling axis.
	pub fn set_viewport(&mut self, viewport: f64) {
		self.viewport = viewport;
	}

	/// Records the scroll container's current offset along the axis.
	pub fn scroll_to(&mut self, offset: f64) {
		self.scroll_offset = offset;
	}

	/// The current scroll offset.
	pub fn scroll_offset(&self) -> f64 {
		self.scroll_offset
	}

	/// Replaces the item count, keeping measurements for surviving indexes.
	pub fn set_count(&mut self, count: usize) {
		if count != self.count {
			self.measured.retain(|index, _| *index < count);
			self.count = count;
			self.table.lock().dirty = true;
		}
	}

	/// Supplies a measured size for one item, as the renderer reports it.
	///
	/// A changed size shifts every following item, so windows computed from
	/// estimates re-resolve once real sizes arrive. The size must be finite
	/// and positive; the index must be inside `0..count`.
	pub fn measure(&mut self, index: usize, size: f64) -> Result<(), MeasureError> {
		if index >= self.count {
			return Err(MeasureError::OutOfRange(OutOfRange {
				index,
				count: self.count,
			}));
		}

		if !size.is_finite() || size <= 0.0 {
			return Err(MeasureError::InvalidSize { index, value: size });
		}

		if self.measured.insert(index, size) != Some(size) {
			self.table.lock().dirty = true;
		}
		Ok(())
	}

	/// The items to render: the visible window plus overscan on each side.
	///
	/// Empty when the viewport has no size yet or the count is zero.
	pub fn items(&self) -> Result<Vec<VirtualItem>, InvalidEstimate> {
		self.with_table(|measurements| {
			let Some((first, last)) =
				Self::visible_range(measurements, self.scroll_offset, self.viewport)
			else {
				return Vec::new();
			};

			let first = first.saturating_sub(self.overscan);
			let last = (last + self.overscan).min(measurements.len() - 1);

			measurements[first..=last]
				.iter()
				.enumerate()
				.map(|(offset, measurement)| {
					VirtualItem {
						index: first + offset,
						start: measurement.start,
						size: measurement.size,
					}
				})
				.collect()
		})
	}

	/// The content size along the scrolling axis: padding, every item, and
	/// trailing padding.
	pub fn total_size(&self) -> Result<f64, InvalidEstimate> {
		self.with_table(|measurements| {
			match measurements.last() {
				Some(last) => last.end + self.padding_end,
				None => self.padding_start + self.padding_end,
			}
		})
	}

	/// The scroll offset that brings `index` into the viewport at `align`.
	///
	/// The target is clamped to the scrollable range. [`Align::Auto`] returns
	/// the current offset when the item is already visible. An invalid
	/// estimate leaves the offset at its current value: the error already
	/// surfaced from [`items`](Self::items) or
	/// [`total_size`](Self::total_size), and there is nothing to compute.
	pub fn offset_for_index(&self, index: usize, align: Align) -> Result<f64, OutOfRange> {
		if index >= self.count {
			return Err(OutOfRange {
				index,
				count: self.count,
			});
		}

		self.with_table(|measurements| {
			let measurement = measurements[index];
			// An in-range index implies at least one measurement.
			let total = measurements
				.last()
				.map(|last| last.end + self.padding_end)
				.expect("an in-range index implies a measurement");
			let viewport = self.viewport;

			let target = match align {
				Align::Start => measurement.start,
				Align::Center => measurement.start - viewport / 2.0 + measurement.size / 2.0,
				Align::End => measurement.end - viewport,
				Align::Auto => {
					let visible = measurement.start >= self.scroll_offset
						&& measurement.end <= self.scroll_offset + viewport;
					if visible {
						self.scroll_offset
					} else if measurement.end > self.scroll_offset + viewport {
						measurement.end - viewport
					} else {
						measurement.start
					}
				}
			};

			target.clamp(0.0, (total - viewport).max(0.0))
		})
		.or(Ok(self.scroll_offset))
	}

	/// The inclusive window of indexes intersecting the viewport.
	fn visible_range(
		measurements: &[Measurement],
		offset: f64,
		viewport: f64,
	) -> Option<(usize, usize)> {
		if measurements.is_empty() || viewport <= 0.0 {
			return None;
		}

		// The first item whose end passes the viewport's start, then every
		// item whose start precedes the viewport's end.
		let first = measurements.partition_point(|measurement| measurement.end <= offset);
		let last_exclusive = measurements[first..]
			.partition_point(|measurement| measurement.start < offset + viewport)
			+ first;

		(first < last_exclusive).then_some((first, last_exclusive - 1))
	}

	/// Runs `read` over the measurement table, rebuilding it first when sizes
	/// or the count changed.
	fn with_table<R>(&self, read: impl FnOnce(&[Measurement]) -> R) -> Result<R, InvalidEstimate> {
		let mut table = self.table.lock();
		if table.dirty {
			table.measurements.clear();
			table.measurements.reserve(self.count);

			let mut start = self.padding_start;
			for index in 0..self.count {
				let size = match self.measured.get(&index) {
					Some(size) => *size,
					None => self.estimate.size_of(index),
				};
				if !size.is_finite() || size <= 0.0 {
					// The table stays dirty: it re-resolves the moment the
					// estimate, a measurement, or the count changes.
					return Err(InvalidEstimate { index, value: size });
				}
				table.measurements.push(Measurement {
					start,
					size,
					end: start + size,
				});
				start += size;
			}
			table.dirty = false;
		}
		Ok(read(&table.measurements))
	}
}
