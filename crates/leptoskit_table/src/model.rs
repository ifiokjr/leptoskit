//! The table's state and the row pipeline over it: filter, sort, paginate.

use crate::Column;

/// Which way a sorted column runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
	/// Smallest first.
	Ascending,
	/// Largest first.
	Descending,
}

/// One active sort: a column id and its direction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortSpec {
	/// The sorted column's [`id`](Column::id).
	pub column: String,
	/// Which way the column runs.
	pub direction: SortDirection,
}

/// Everything a table does to its rows, owned by the screen like every other
/// leptoskit state: which column is sorted, the global filter text, and the
/// page. Feed it back the results of
/// [`header_press`](Self::for_header_press) and the other reducers after
/// storing them.
///
/// Sorting is single-column: the header cycle expresses one sort at a time.
/// A `page_size` of `None` renders every filtered row on one page.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TableState {
	/// The active sort, if any.
	pub sorting: Option<SortSpec>,
	/// The global filter's text; empty matches everything.
	pub filter: String,
	/// The zero-based page of filtered rows.
	pub page: usize,
	/// Rows per page, or no pagination.
	pub page_size: Option<usize>,
}

impl TableState {
	/// Unsorted, unfiltered, unpaginated.
	pub fn new() -> Self {
		Self::default()
	}

	/// Sets the page size, restarting from the first page.
	#[must_use]
	pub fn with_page_size(mut self, page_size: usize) -> Self {
		self.page_size = Some(page_size);
		self.page = 0;
		self
	}
}

/// The state a header press proposes: pressing an unsorted or descending
/// column sorts it ascending, pressing an ascending one sorts it descending,
/// and pressing it again clears the sort. Unsortable columns change nothing.
/// Every change restarts from the first page.
#[must_use]
pub fn for_header_press(state: &TableState, column_id: &str) -> TableState {
	let Some(proposed) = state
		.sorting
		.as_ref()
		.filter(|sort| sort.column == column_id)
	else {
		return TableState {
			sorting: Some(SortSpec {
				column: column_id.to_owned(),
				direction: SortDirection::Ascending,
			}),
			filter: state.filter.clone(),
			page: 0,
			page_size: state.page_size,
		};
	};

	TableState {
		sorting: match proposed.direction {
			SortDirection::Ascending => {
				Some(SortSpec {
					column: column_id.to_owned(),
					direction: SortDirection::Descending,
				})
			}
			SortDirection::Descending => None,
		},
		filter: state.filter.clone(),
		page: 0,
		page_size: state.page_size,
	}
}

/// The state a new filter text proposes: the first page of the new matches.
#[must_use]
pub fn for_filter(state: &TableState, text: impl Into<String>) -> TableState {
	TableState {
		sorting: state.sorting.clone(),
		filter: text.into(),
		page: 0,
		page_size: state.page_size,
	}
}

/// The state a page turn proposes, clamped to the filtered rows' pages.
#[must_use]
pub fn for_page(state: &TableState, total_filtered: usize, page: usize) -> TableState {
	let pages = page_count(total_filtered, state.page_size);
	TableState {
		sorting: state.sorting.clone(),
		filter: state.filter.clone(),
		page: page.min(pages.saturating_sub(1)),
		page_size: state.page_size,
	}
}

/// How many pages `total` filtered rows occupy.
#[must_use]
pub fn page_count(total: usize, page_size: Option<usize>) -> usize {
	match page_size {
		Some(size) if size > 0 => total.div_ceil(size).max(1),
		_ => 1,
	}
}

/// The rows a table shows: the current page, and the counts the controls
/// need.
///
/// The pipeline is the `TanStack` order: filter, then sort, then paginate. A
/// row matches the filter when any column's cell matches that column's
/// filter rule — cell text containing the query, case-insensitively, unless
/// the column overrides it. The sort is a stable comparison of the filtered
/// rows, so equal keys keep their source order.
pub struct TableRows<'a, T> {
	/// The rows on the current page.
	pub rows: Vec<&'a T>,
	/// The rows the filter kept.
	pub total: usize,
	/// How many pages the filtered rows occupy.
	pub pages: usize,
}

impl<T> TableRows<'_, T> {
	/// The one-based page number, for display.
	#[must_use]
	pub fn page_number(&self, state: &TableState) -> usize {
		self.pages.saturating_sub(1).min(state.page) + 1
	}
}

/// Resolves the rows a table shows for `data` under `state`.
pub fn rows<'a, T>(data: &'a [T], columns: &[Column<T>], state: &TableState) -> TableRows<'a, T> {
	let mut kept: Vec<&T> = if state.filter.is_empty() {
		data.iter().collect()
	} else {
		data.iter()
			.filter(|row| {
				columns
					.iter()
					.any(|column| column.matches(row, &state.filter))
			})
			.collect()
	};

	if let Some(sort) = &state.sorting {
		if let Some(column) = columns.iter().find(|column| column.id == sort.column) {
			kept.sort_by(|left, right| {
				let order = column
					.compare(left, right)
					.unwrap_or(std::cmp::Ordering::Equal);
				match sort.direction {
					SortDirection::Ascending => order,
					SortDirection::Descending => order.reverse(),
				}
			});
		}
	}

	let total = kept.len();
	let pages = page_count(total, state.page_size);
	let rows = match state.page_size {
		Some(size) if size > 0 => {
			let start = state.page.min(pages.saturating_sub(1)) * size;
			let end = (start + size).min(total);
			kept[start..end].to_vec()
		}
		_ => kept,
	};

	TableRows { rows, total, pages }
}
