//! The rendered table: headers, rows, filter, and pagination as ARIA roles.

use std::sync::Arc;

use leptoskit_native::Element;
use leptoskit_native::Role;
use leptoskit_native::components::Status;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;

use crate::Column;
use crate::SortDirection;
use crate::TableState;
use crate::rows;

/// The cell class headers and body cells share, so columns line up.
const CELL_CLASS: &str = "flex-1 min-w-0 px-2 py-1";
/// The header label's sort indicator, ascending and descending.
const SORT_MARKS: [&str; 2] = [" ▲", " ▼"];

/// A table rendered from columns, data, and state, in the style of
/// `TanStack Table`'s materialization: headers that sort on press, cells from
/// the columns' readers, a global filter field, and pagination when the state
/// has a page size.
///
/// The grid is ARIA roles over the shared tree — `table`, `row`,
/// `columnheader` carrying `aria-sort`, `cell` — so screen readers announce
/// it on the web and on every native backend. Style it with Tailwind classes
/// like any component; build a custom grid from [`rows`](crate::rows) and
/// the same roles when this composition is not the one.
///
/// The callbacks propose changes; the screen stores the resulting
/// [`TableState`](crate::TableState), exactly like every controlled control:
///
/// ```
/// use leptoskit_table::Column;
/// use leptoskit_table::DataTable;
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
/// let table = DataTable(&columns, &data, &TableState::new(), |_| (), |_| (), |_| ());
/// // The filter field, then the grid of a header row and two body rows.
/// assert_eq!(table.children.len(), 2);
/// assert_eq!(table.children[1].children.len(), 3);
/// ```
pub fn DataTable<T>(
	columns: &[Column<T>],
	data: &[T],
	state: &TableState,
	on_header_press: impl Fn(&str) + Send + Sync + 'static,
	on_filter_change: impl Fn(String) + Send + Sync + 'static,
	on_page: impl Fn(usize) + Send + Sync + 'static,
) -> Element {
	let table = rows(data, columns, state);

	let mut grid = View().role(Role::Table).class("flex-col w-full");
	let on_header_press = Arc::new(on_header_press);
	grid = grid.child(header_row(columns, state, {
		let on_header_press = Arc::clone(&on_header_press);
		move |id| on_header_press(id)
	}));

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

	let mut screen = View().class("flex-col gap-2 w-full");
	screen = screen.child(
		Element::text_input()
			.aria_label("Filter rows")
			.placeholder("Filter rows")
			.value(state.filter.clone())
			.on_change(move |text| on_filter_change(text.to_owned()))
			.class("h-8 rounded-md px-2"),
	);
	screen = screen.child(grid);

	if state.page_size.is_some() {
		let page = table.page_number(state).saturating_sub(1);
		let on_page = Arc::new(on_page);
		let previous = {
			let on_page = Arc::clone(&on_page);
			move || on_page(page.saturating_sub(1))
		};
		let next = {
			let on_page = Arc::clone(&on_page);
			move || on_page(page + 1)
		};

		screen = screen.child(
			View()
				.class("flex-row gap-2 items-center")
				.child(
					Element::button("Previous page")
						.enabled(page > 0)
						.on_press(previous)
						.class("px-2 py-1 rounded-md"),
				)
				.child(Status(format!(
					"Page {} of {}",
					table.page_number(state),
					table.pages
				)))
				.child(
					Element::button("Next page")
						.enabled(page + 1 < table.pages)
						.on_press(next)
						.class("px-2 py-1 rounded-md"),
				),
		);
	}

	screen
}

/// The header row: one pressable `columnheader` per sortable column, carrying
/// `aria-sort` while its column runs the sort.
fn header_row<T>(
	columns: &[Column<T>],
	state: &TableState,
	on_header_press: impl Fn(&str) + Send + Sync + 'static,
) -> Element {
	let on_header_press = Arc::new(on_header_press);
	let mut row = View().role(Role::Row).class("flex-row font-semibold");

	for column in columns {
		let sort = state
			.sorting
			.as_ref()
			.filter(|sort| sort.column == column.id);

		let mut label = column.header.clone();
		if let Some(sort) = sort {
			label.push_str(match sort.direction {
				SortDirection::Ascending => SORT_MARKS[0],
				SortDirection::Descending => SORT_MARKS[1],
			});
		}

		let header = if column.is_sortable() {
			let id = column.id.clone();
			let on_header_press = Arc::clone(&on_header_press);
			Element::button(label)
				.role(Role::ColumnHeader)
				.class(CELL_CLASS)
				.on_press(move || on_header_press(&id))
		} else {
			// An unsortable header is a plain cell, not a control.
			Text(label).role(Role::ColumnHeader).class(CELL_CLASS)
		};

		let header = match sort {
			Some(sort) => {
				header.aria_attr(
					"aria-sort",
					match sort.direction {
						SortDirection::Ascending => "ascending",
						SortDirection::Descending => "descending",
					},
				)
			}
			None => header,
		};

		row = row.child(header);
	}

	row
}
