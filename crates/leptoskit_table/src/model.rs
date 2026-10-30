//! The table's state and the row pipeline over it: filter, sort, paginate.

use std::num::NonZeroUsize;

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
/// [`for_header_press`](crate::for_header_press) and the other reducers after
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
	pub page_size: Option<NonZeroUsize>,
}

impl TableState {
	/// Unsorted, unfiltered, unpaginated.
	pub fn new() -> Self {
		Self::default()
	}

	/// Paginates by `page_size` rows, restarting from the first page. A page
	/// size of zero turns pagination off.
	#[must_use]
	pub fn with_page_size(mut self, page_size: usize) -> Self {
		self.page_size = NonZeroUsize::new(page_size);
		self.page = 0;
		self
	}
}

/// The state a header press proposes. The press cycles one column through
/// none → ascending → descending → none: pressing an unsorted column sorts it
/// ascending, pressing an ascending one sorts it descending, and pressing a
/// descending one clears the sort. Pressing another column starts it at
/// ascending. Every change restarts from the first page.
///
/// The reducer does not know the columns. A sort on a column that is not
/// [sortable](Column::is_sortable), or on an id no column carries, is inert:
/// [`rows`] keeps the source order and [`DataTable`](crate::DataTable) marks
/// no header. Offer presses only on sortable headers.
#[must_use]
pub fn for_header_press(state: &TableState, column_id: &str) -> TableState {
	let current = state
		.sorting
		.as_ref()
		.filter(|sort| sort.column == column_id)
		.map(|sort| sort.direction);

	let direction = match current {
		None => Some(SortDirection::Ascending),
		Some(SortDirection::Ascending) => Some(SortDirection::Descending),
		Some(SortDirection::Descending) => None,
	};

	TableState {
		sorting: direction.map(|direction| {
			SortSpec {
				column: column_id.to_owned(),
				direction,
			}
		}),
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
		page: page.min(pages - 1),
		page_size: state.page_size,
	}
}

/// How many pages `total` filtered rows occupy: always at least one, so an
/// empty table still has a page to show.
#[must_use]
pub fn page_count(total: usize, page_size: Option<NonZeroUsize>) -> usize {
	match page_size {
		Some(size) => total.div_ceil(size.get()).max(1),
		None => 1,
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
#[derive(Debug)]
pub struct TableRows<'a, T> {
	/// The rows on the current page.
	pub rows: Vec<&'a T>,
	/// The rows the filter kept.
	pub total: usize,
	/// How many pages the filtered rows occupy.
	pub pages: usize,
	/// The zero-based page shown: the state's page, clamped to `pages`, so a
	/// page past the end after the data shrank shows the last page.
	pub page: usize,
}

impl<T> TableRows<'_, T> {
	/// The one-based page number, for display.
	#[must_use]
	pub fn page_number(&self) -> usize {
		self.page + 1
	}
}

/// Resolves the rows a table shows for `data` under `state`.
pub fn rows<'a, T>(data: &'a [T], columns: &[Column<T>], state: &TableState) -> TableRows<'a, T> {
	let mut kept: Vec<&T> = if state.filter.is_empty() {
		data.iter().collect()
	} else {
		let query_lowercase = state.filter.to_lowercase();
		data.iter()
			.filter(|row| {
				columns
					.iter()
					.any(|column| column.matches(row, &state.filter, &query_lowercase))
			})
			.collect()
	};

	if let Some(sort) = &state.sorting
		&& let Some(column) = columns.iter().find(|column| column.id == sort.column)
	{
		column.sort(&mut kept, sort.direction);
	}

	let total = kept.len();
	let pages = page_count(total, state.page_size);
	let page = state.page.min(pages - 1);
	let rows = match state.page_size {
		Some(size) => {
			let start = page * size.get();
			let end = start.saturating_add(size.get()).min(total);
			kept[start..end].to_vec()
		}
		None => kept,
	};

	TableRows {
		rows,
		total,
		pages,
		page,
	}
}
