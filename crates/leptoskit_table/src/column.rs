//! Typed columns: how one field of a row reads, sorts, and filters.

use std::cmp::Ordering;
use std::cmp::Reverse;
use std::sync::Arc;

use crate::SortDirection;

/// How a row's cell reads.
type CellReader<T> = Arc<dyn Fn(&T) -> String + Send + Sync>;
/// How a column compares two rows.
type RowCompare<T> = Arc<dyn Fn(&T, &T) -> Ordering + Send + Sync>;
/// How a column matches the global filter.
type FilterRule<T> = Arc<dyn Fn(&T, &str) -> bool + Send + Sync>;

/// How a column sorts two rows.
enum SortBy<T> {
	/// Lexically, over the cell text.
	Text,
	/// By a custom comparison of the rows themselves.
	Custom(RowCompare<T>),
}

/// One column of a [`Table`](crate): its identity, its header, and how each
/// row's cell reads, sorts, and filters.
///
/// A column is data, not state: build the set once and pass it to
/// [`rows`](crate::rows) and [`DataTable`](crate::DataTable) on every render.
/// The cell reads a string; sorting and filtering are defined over the row
/// itself, so they never depend on how a cell happens to format.
///
/// ```
/// use leptoskit_table::Column;
///
/// struct Planet {
/// 	name: &'static str,
/// 	moons: u8,
/// }
///
/// let name = Column::new("name", "Name", |planet: &Planet| planet.name.to_owned()).sortable();
/// let moons = Column::new("moons", "Moons", |planet: &Planet| planet.moons.to_string())
/// 	.sort_by(|a, b| a.moons.cmp(&b.moons));
///
/// let jupiter = Planet {
/// 	name: "Jupiter",
/// 	moons: 95,
/// };
/// assert_eq!(name.cell_text(&jupiter), "Jupiter");
/// assert_eq!(moons.cell_text(&jupiter), "95");
/// ```
pub struct Column<T> {
	/// The column's stable identity, used by sorting state and callbacks.
	pub id: String,
	/// The header cell's visible text.
	pub header: String,
	cell: CellReader<T>,
	sort: Option<SortBy<T>>,
	filter: Option<FilterRule<T>>,
}

impl<T> Clone for Column<T> {
	fn clone(&self) -> Self {
		Self {
			id: self.id.clone(),
			header: self.header.clone(),
			cell: Arc::clone(&self.cell),

			sort: match &self.sort {
				Some(SortBy::Text) => Some(SortBy::Text),
				Some(SortBy::Custom(compare)) => Some(SortBy::Custom(Arc::clone(compare))),
				None => None,
			},

			filter: self.filter.as_ref().map(Arc::clone),
		}
	}
}

impl<T> std::fmt::Debug for Column<T> {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("Column")
			.field("id", &self.id)
			.field("header", &self.header)
			.field("sortable", &self.sort.is_some())
			.finish_non_exhaustive()
	}
}

impl<T> Column<T> {
	/// A column over a cell reader, not sortable and not filtered beyond the
	/// default text match.
	pub fn new(
		id: impl Into<String>,
		header: impl Into<String>,
		cell: impl Fn(&T) -> String + Send + Sync + 'static,
	) -> Self {
		Self {
			id: id.into(),
			header: header.into(),
			cell: Arc::new(cell),
			sort: None,
			filter: None,
		}
	}

	/// Makes the column sortable, lexically over its cell text.
	#[must_use]
	pub fn sortable(mut self) -> Self {
		self.sort = Some(SortBy::Text);
		self
	}

	/// Makes the column sortable by a comparison of the rows themselves.
	#[must_use]
	pub fn sort_by(mut self, compare: impl Fn(&T, &T) -> Ordering + Send + Sync + 'static) -> Self {
		self.sort = Some(SortBy::Custom(Arc::new(compare)));
		self
	}

	/// Replaces this column's contribution to the global filter: by default a
	/// row matches when some column's cell text contains the query,
	/// case-insensitively.
	#[must_use]
	pub fn filter_by(
		mut self,
		predicate: impl Fn(&T, &str) -> bool + Send + Sync + 'static,
	) -> Self {
		self.filter = Some(Arc::new(predicate));
		self
	}

	/// Whether the column carries a sort.
	pub fn is_sortable(&self) -> bool {
		self.sort.is_some()
	}

	/// The cell's visible text.
	pub fn cell_text(&self, row: &T) -> String {
		(self.cell)(row)
	}

	/// Whether the row's cell in this column matches the global filter.
	///
	/// `query_lowercase` is `query` lowercased once by the caller, so the
	/// default rule does not lowercase it again for every cell.
	pub(crate) fn matches(&self, row: &T, query: &str, query_lowercase: &str) -> bool {
		match &self.filter {
			Some(predicate) => predicate(row, query),
			None => self.cell_text(row).to_lowercase().contains(query_lowercase),
		}
	}

	/// Stably sorts `rows` by this column, if it has a sort, and reports
	/// whether it did. Equal keys keep their order in either direction.
	pub(crate) fn sort(&self, rows: &mut [&T], direction: SortDirection) -> bool {
		match &self.sort {
			Some(SortBy::Text) => {
				// Read each cell once, not twice per comparison.
				match direction {
					SortDirection::Ascending => rows.sort_by_cached_key(|row| self.cell_text(row)),
					SortDirection::Descending => {
						rows.sort_by_cached_key(|row| Reverse(self.cell_text(row)));
					}
				}

				true
			}
			Some(SortBy::Custom(compare)) => {
				match direction {
					SortDirection::Ascending => rows.sort_by(|left, right| compare(left, right)),
					SortDirection::Descending => rows.sort_by(|left, right| compare(right, left)),
				}

				true
			}
			None => false,
		}
	}
}
