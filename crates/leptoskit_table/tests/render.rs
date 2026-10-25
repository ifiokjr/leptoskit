//! The rendered grid over the real headless renderer: sorting presses,
//! pagination presses, and the committed semantics.

use std::sync::Arc;
use std::sync::RwLock;

use leptoskit_native::Element;
use leptoskit_native::Role;
use leptoskit_table::Column;
use leptoskit_table::DataTable;
use leptoskit_table::TableState;
use leptoskit_table::for_filter;
use leptoskit_table::for_header_press;
use leptoskit_table::for_page;
use leptoskit_table::rows;
use leptoskit_testing::Finder;
use leptoskit_testing::WidgetTester;

struct Planet {
	name: &'static str,
	moons: u8,
}

const DATA: [Planet; 4] = [
	Planet {
		name: "Jupiter",
		moons: 95,
	},
	Planet {
		name: "Mars",
		moons: 2,
	},
	Planet {
		name: "Saturn",
		moons: 146,
	},
	Planet {
		name: "Venus",
		moons: 0,
	},
];

fn make_columns() -> [Column<Planet>; 2] {
	[
		Column::new("name", "Name", |planet: &Planet| planet.name.to_owned()).sortable(),
		Column::new("moons", "Moons", |planet: &Planet| planet.moons.to_string())
			.sort_by(|a, b| a.moons.cmp(&b.moons)),
	]
}

/// A controlled table over shared state, the way a screen wires one.
fn table_screen(state: &Arc<RwLock<TableState>>) -> Element {
	let current = state.read().expect("uncontended").clone();
	let columns = make_columns();
	DataTable(
		&columns,
		&DATA,
		&current,
		{
			let state = Arc::clone(state);
			move |id| {
				if let Ok(mut next) = state.write() {
					*next = for_header_press(&next, id);
				}
			}
		},
		{
			let state = Arc::clone(state);
			move |text| {
				if let Ok(mut next) = state.write() {
					*next = for_filter(&next, text);
				}
			}
		},
		{
			let state = Arc::clone(state);
			move |page| {
				let columns = make_columns();
				let current = state.read().expect("uncontended").clone();
				let total = rows(&DATA, &columns, &current).total;
				if let Ok(mut next) = state.write() {
					*next = for_page(&next, total, page);
				}
			}
		},
	)
}

#[test]
fn header_presses_sort_the_rendered_rows() -> Result<(), leptoskit_testing::TestError> {
	let state = Arc::new(RwLock::new(TableState::new().with_page_size(10)));
	let mut tester = WidgetTester::new(
		{
			let state = Arc::clone(&state);
			move || table_screen(&state)
		},
		800.0,
		600.0,
	)?;

	// Source order on mount.
	tester.find(&Finder::text("Jupiter"))?;

	// Pressing the moons header sorts ascending: Venus, with no moons, leads.
	tester.tap(&Finder::aria_role(Role::ColumnHeader, "Moons"))?;
	let header = tester.find(&Finder::aria_role(Role::ColumnHeader, "Moons ▲"))?;
	assert_eq!(
		header.props.aria().extra.get("aria-sort"),
		Some(&"ascending".to_owned())
	);

	// Pressing again sorts descending: Saturn leads the body, Venus trails.
	tester.tap(&Finder::aria_role(Role::ColumnHeader, "Moons ▲"))?;
	let header = tester.find(&Finder::aria_role(Role::ColumnHeader, "Moons ▼"))?;
	assert_eq!(
		header.props.aria().extra.get("aria-sort"),
		Some(&"descending".to_owned())
	);
	let saturn = tester.find(&Finder::text("Saturn"))?;
	let venus = tester.find(&Finder::text("Venus"))?;
	assert!(saturn.frame.y < venus.frame.y, "Saturn leads Venus");
	assert!(
		saturn.frame.y > header.frame.y,
		"the body follows the header"
	);
	Ok(())
}

#[test]
fn pagination_presses_move_between_pages() -> Result<(), leptoskit_testing::TestError> {
	let state = Arc::new(RwLock::new(TableState::new().with_page_size(2)));
	let mut tester = WidgetTester::new(
		{
			let state = Arc::clone(&state);
			move || table_screen(&state)
		},
		800.0,
		600.0,
	)?;

	// Two rows to a page: the second page holds Saturn and Venus.
	tester.find(&Finder::text("Jupiter"))?;
	tester.find(&Finder::text("Mars"))?;
	assert!(tester.find(&Finder::text("Venus")).is_err());

	tester.tap(&Finder::aria_role(Role::Button, "Next page"))?;
	assert!(tester.find(&Finder::text("Jupiter")).is_err());
	tester.find(&Finder::text("Saturn"))?;
	tester.find(&Finder::text("Venus"))?;
	tester.find(&Finder::text("Page 2 of 2"))?;

	// The previous button returns to the first page.
	tester.tap(&Finder::aria_role(Role::Button, "Previous page"))?;
	tester.find(&Finder::text("Jupiter"))?;
	Ok(())
}
