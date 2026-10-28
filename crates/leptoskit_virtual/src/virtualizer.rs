//! The windowing math behind [`leptoskit_virtual`](crate): one axis-agnostic
//! core, following `TanStack Virtual`'s model.

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

/// A scroll target this virtualizer cannot compute.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OffsetError {
	/// The index is outside `0..count`.
	OutOfRange(OutOfRange),
	/// An item's size estimate is invalid, so no offset can be resolved.
	InvalidEstimate(InvalidEstimate),
}

impl fmt::Display for OffsetError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::OutOfRange(error) => write!(formatter, "{error}"),
			Self::InvalidEstimate(error) => write!(formatter, "{error}"),
		}
	}
}

impl std::error::Error for OffsetError {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			Self::OutOfRange(error) => Some(error),
			Self::InvalidEstimate(error) => Some(error),
		}
	}
}

impl From<OutOfRange> for OffsetError {
	fn from(error: OutOfRange) -> Self {
		Self::OutOfRange(error)
	}
}

impl From<InvalidEstimate> for OffsetError {
	fn from(error: InvalidEstimate) -> Self {
		Self::InvalidEstimate(error)
	}
}

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
	/// The sizes the renderer reported, by index.
	measured: Vec<Option<f64>>,
	scroll_offset: f64,
	viewport: f64,
	/// Every item's resolved position, built lazily: the entries present are
	/// current, and a read extends the table to `count`. Changing a size or
	/// the count truncates it, so only the items after the change re-resolve.
	measurements: Mutex<Vec<Measurement>>,
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
			measured: vec![None; options.count],
			scroll_offset: 0.0,
			viewport: 0.0,
			measurements: Mutex::new(Vec::with_capacity(options.count)),
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
	///
	/// The offset is recorded as reported, including the overscroll of an
	/// elastic edge; the window resolves against it clamped to the scrollable
	/// range, so a list that shrank under its scroll position still renders
	/// its last items.
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
			self.measured.resize(count, None);
			self.measurements.get_mut().truncate(count);
			self.count = count;
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

		if !is_layout_size(size) {
			return Err(MeasureError::InvalidSize { index, value: size });
		}

		if self.measured[index].replace(size) != Some(size) {
			self.measurements.get_mut().truncate(index);
		}
		Ok(())
	}

	/// The items to render: the visible window plus overscan on each side.
	///
	/// Empty when the viewport has no size yet or the count is zero.
	pub fn items(&self) -> Result<Vec<VirtualItem>, InvalidEstimate> {
		self.with_table(|measurements| {
			let offset = self.clamped_offset(measurements);
			let Some((first, last)) = Self::visible_range(measurements, offset, self.viewport)
			else {
				return Vec::new();
			};

			let first = first.saturating_sub(self.overscan);
			let last = last
				.saturating_add(self.overscan)
				.min(measurements.len() - 1);

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
		self.with_table(|measurements| self.content_size(measurements))
	}

	/// The scroll offset that brings `index` into the viewport at `align`.
	///
	/// The target is clamped to the scrollable range. [`Align::Auto`] returns
	/// the current offset when the item is already visible. An index outside
	/// `0..count` and an invalid size estimate are both errors: neither has an
	/// offset to scroll to.
	pub fn offset_for_index(&self, index: usize, align: Align) -> Result<f64, OffsetError> {
		if index >= self.count {
			return Err(OutOfRange {
				index,
				count: self.count,
			}
			.into());
		}

		let target = self.with_table(|measurements| {
			let measurement = measurements[index];
			let viewport = self.viewport;
			let offset = self.clamped_offset(measurements);

			let target = match align {
				Align::Start => measurement.start,
				Align::Center => measurement.start - viewport / 2.0 + measurement.size / 2.0,
				Align::End => measurement.end - viewport,
				Align::Auto => {
					let visible =
						measurement.start >= offset && measurement.end <= offset + viewport;
					if visible {
						offset
					} else if measurement.end > offset + viewport {
						measurement.end - viewport
					} else {
						measurement.start
					}
				}
			};

			target.clamp(0.0, self.max_offset(measurements))
		})?;
		Ok(target)
	}

	/// The content size over a resolved table: both paddings and every item.
	fn content_size(&self, measurements: &[Measurement]) -> f64 {
		match measurements.last() {
			Some(last) => last.end + self.padding_end,
			None => self.padding_start + self.padding_end,
		}
	}

	/// The largest offset the container can scroll to.
	fn max_offset(&self, measurements: &[Measurement]) -> f64 {
		(self.content_size(measurements) - self.viewport).max(0.0)
	}

	/// The recorded scroll offset, held inside the scrollable range.
	fn clamped_offset(&self, measurements: &[Measurement]) -> f64 {
		self.scroll_offset.clamp(0.0, self.max_offset(measurements))
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

		(first < last_exclusive).then(|| (first, last_exclusive - 1))
	}

	/// Runs `read` over the measurement table, first resolving every item the
	/// table does not hold yet.
	fn with_table<R>(&self, read: impl FnOnce(&[Measurement]) -> R) -> Result<R, InvalidEstimate> {
		let mut measurements = self.measurements.lock();
		let mut start = measurements
			.last()
			.map_or(self.padding_start, |last| last.end);

		for index in measurements.len()..self.count {
			let size = self.measured[index].unwrap_or_else(|| self.estimate.size_of(index));
			if !is_layout_size(size) {
				// The items before this one stay resolved; the next read
				// retries from here, once a measurement or the count changed.
				return Err(InvalidEstimate { index, value: size });
			}
			measurements.push(Measurement {
				start,
				size,
				end: start + size,
			});
			start += size;
		}

		Ok(read(&measurements))
	}
}

/// Whether `size` is a finite, positive number of layout units.
fn is_layout_size(size: f64) -> bool {
	size.is_finite() && size > 0.0
}
