//! The rendered table: headers, rows, filter, and pagination as ARIA roles.

use std::fmt;
use std::sync::Arc;

use leptoskit_native::Element;
use leptoskit_native::Role;
use leptoskit_native::components::Status;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;

use crate::Column;
use crate::SortDirection;
use crate::TableState;
use crate::for_filter;
use crate::for_header_press;
use crate::for_page;
use crate::rows;

/// The cell class headers and body cells share, so columns line up.
const CELL_CLASS: &str = "flex-1 min-w-0 px-2 py-1";

/// Formats the page status from the one-based page and the page count.
type PageStatus = Arc<dyn Fn(usize, usize) -> String + Send + Sync>;

/// The words a [`DataTable`] shows and announces, so a translated screen
/// translates its table too. [`Default`] is English.
///
/// ```
/// use std::sync::Arc;
///
/// use leptoskit_table::TableLabels;
///
/// let labels = TableLabels {
/// 	filter: "Filtrar filas".to_owned(),
/// 	previous_page: "Página anterior".to_owned(),
/// 	next_page: "Página siguiente".to_owned(),
/// 	page_status: Arc::new(|page, pages| format!("Página {page} de {pages}")),
/// };
/// assert_eq!((labels.page_status)(2, 5), "Página 2 de 5");
/// ```
#[derive(Clone)]
pub struct TableLabels {
	/// The filter field's placeholder and accessible name.
	pub filter: String,
	/// The previous-page button.
	pub previous_page: String,
	/// The next-page button.
	pub next_page: String,
	/// The live page status, from the one-based page and the page count.
	pub page_status: PageStatus,
}

impl Default for TableLabels {
	fn default() -> Self {
		Self {
			filter: "Filter rows".to_owned(),
			previous_page: "Previous page".to_owned(),
			next_page: "Next page".to_owned(),
			page_status: Arc::new(|page, pages| format!("Page {page} of {pages}")),
		}
	}
}

impl fmt::Debug for TableLabels {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter
			.debug_struct("TableLabels")
			.field("filter", &self.filter)
			.field("previous_page", &self.previous_page)
			.field("next_page", &self.next_page)
			.finish_non_exhaustive()
	}
}

/// A table rendered from columns, data, and state, in the style of
/// `TanStack Table`'s materialization: headers that sort on press, cells from
/// the columns' readers, a global filter field, and pagination when the state
/// has a page size.
///
/// The grid is ARIA roles over the shared tree — `table`, `row`,
/// `columnheader`, `cell` — and the sorted header carries `aria-sort`. A
/// sortable header holds a button, so it is announced as both a column
/// header and a control. Style it with Tailwind classes like any component;
/// build a custom grid from [`rows`](crate::rows) and the same roles when
/// this composition is not the one.
///
/// Every interaction proposes a whole next state — a header press runs
/// [`for_header_press`], typing [`for_filter`], a page turn [`for_page`] —
/// and `on_change` receives it. The screen stores it, exactly like every
/// controlled control:
///
/// ```
/// use leptoskit_table::Column;
/// use leptoskit_table::DataTable;
/// use leptoskit_table::TableLabels;
/// use leptoskit_table::TableState;
///
/// struct Planet {
/// 	name: &'static str,
/// 	moons: u8,
/// }
/// let data = [
/// 	Planet {
/// 		name: "Jupiter",
/// 		moons: 95,
/// 	},
/// 	Planet {
/// 		name: "Mars",
/// 		moons: 2,
/// 	},
/// ];
/// let columns =
/// 	[Column::new("name", "Name", |planet: &Planet| planet.name.to_owned()).sortable()];
///
/// let table = DataTable(
/// 	&columns,
/// 	&data,
/// 	&TableState::new(),
/// 	&TableLabels::default(),
/// 	|_next| (),
/// );
/// // The filter field, then the grid of a header row and two body rows.
/// assert_eq!(table.children.len(), 2);
/// assert_eq!(table.children[1].children.len(), 3);
/// ```
#[track_caller]
pub fn DataTable<T>(
	columns: &[Column<T>],
	data: &[T],
	state: &TableState,
	labels: &TableLabels,
	on_change: impl Fn(TableState) + Send + Sync + 'static,
) -> Element {
	let table = rows(data, columns, state);
	let on_change: Arc<dyn Fn(TableState) + Send + Sync> = Arc::new(on_change);

	let mut grid = View()
		.role(Role::Table)
		.class("flex-col w-full")
		.child(header_row(columns, state, &on_change));
	for row in &table.rows {
		let mut cells = View().role(Role::Row).class("flex-row");
		for column in columns {
			cells = cells.child(
				View()
					.role(Role::Cell)
					.class(CELL_CLASS)
					.child(Text(column.cell_text(row))),
			);
		}
		grid = grid.child(cells);
	}

	let filter_field = {
		let state = state.clone();
		let on_change = Arc::clone(&on_change);
		Element::text_input()
			.aria_label(labels.filter.clone())
			.placeholder(labels.filter.clone())
			.value(state.filter.clone())
			.on_change(move |text| on_change(for_filter(&state, text)))
			.class("h-8 rounded-md px-2")
	};

	let screen = View()
		.class("flex-col gap-2 w-full")
		.child(filter_field)
		.child(grid);
	if state.page_size.is_none() {
		return screen;
	}

	let page_button = |label: &str, enabled: bool, page: usize| {
		let state = state.clone();
		let on_change = Arc::clone(&on_change);
		let total = table.total;
		Element::button(label)
			.enabled(enabled)
			.on_press(move || on_change(for_page(&state, total, page)))
			.class("px-2 py-1 rounded-md")
	};
	let page = table.page;

	screen.child(
		View()
			.class("flex-row gap-2 items-center")
			.child(page_button(
				&labels.previous_page,
				page > 0,
				page.saturating_sub(1),
			))
			.child(Status((labels.page_status)(
				table.page_number(),
				table.pages,
			)))
			.child(page_button(
				&labels.next_page,
				page + 1 < table.pages,
				page + 1,
			)),
	)
}

/// The header row: one `columnheader` per column. A sortable column's header
/// holds a button that proposes the next sort, and the sorted column's header
/// carries `aria-sort` with a visual mark that screen readers skip, since
/// `aria-sort` already says it.
fn header_row<T>(
	columns: &[Column<T>],
	state: &TableState,
	on_change: &Arc<dyn Fn(TableState) + Send + Sync>,
) -> Element {
	// Centered, so plain headers line up with the sort buttons beside them.
	let mut row = View()
		.role(Role::Row)
		.class("flex-row items-center font-semibold");

	for column in columns {
		if !column.is_sortable() {
			row = row.child(
				Text(column.header.clone())
					.role(Role::ColumnHeader)
					.class(CELL_CLASS),
			);
			continue;
		}

		let press = {
			let state = state.clone();
			let on_change = Arc::clone(on_change);
			let id = column.id.clone();
			move || on_change(for_header_press(&state, &id))
		};
		// The header names itself: native backends do not name a container
		// from its children's text the way browsers do.
		let mut header = View()
			.role(Role::ColumnHeader)
			.aria_label(column.header.clone())
			.class(CELL_CLASS)
			.class("flex-row items-center gap-1")
			.child(Element::button(column.header.clone()).on_press(press));

		let direction = state
			.sorting
			.as_ref()
			.filter(|sort| sort.column == column.id)
			.map(|sort| sort.direction);
		if let Some(direction) = direction {
			let (mark, value) = match direction {
				SortDirection::Ascending => ("▲", "ascending"),
				SortDirection::Descending => ("▼", "descending"),
			};
			header = header
				.aria_attr("aria-sort", value)
				.child(Text(mark).aria_hidden(true));
		}

		row = row.child(header);
	}

	row
}
