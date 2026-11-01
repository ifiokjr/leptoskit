//! The rendered grid over the real headless renderer: sorting presses,
//! filtering, pagination presses, and the committed semantics.

use std::sync::Arc;
use std::sync::RwLock;

use leptoskit_native::Element;
use leptoskit_native::Role;
use leptoskit_table::Column;
use leptoskit_table::DataTable;
use leptoskit_table::TableLabels;
use leptoskit_table::TableState;
use leptoskit_testing::Finder;
use leptoskit_testing::TestError;
use leptoskit_testing::WidgetTester;

struct Planet {
	name: &'static str,
	moons: u8,
	kind: &'static str,
}

const DATA: [Planet; 4] = [
	Planet {
		name: "Jupiter",
		moons: 95,
		kind: "Gas giant",
	},
	Planet {
		name: "Mars",
		moons: 2,
		kind: "Rocky",
	},
	Planet {
		name: "Saturn",
		moons: 146,
		kind: "Gas giant",
	},
	Planet {
		name: "Venus",
		moons: 0,
		kind: "Rocky",
	},
];

fn make_columns() -> [Column<Planet>; 3] {
	[
		Column::new("name", "Name", |planet: &Planet| planet.name.to_owned()).sortable(),
		Column::new("moons", "Moons", |planet: &Planet| planet.moons.to_string())
			.sort_by(|a, b| a.moons.cmp(&b.moons)),
		// Not sortable: its header is a plain column header.
		Column::new("kind", "Kind", |planet: &Planet| planet.kind.to_owned()),
	]
}

/// A controlled table over shared state, the way a screen wires one: the
/// table proposes, the screen stores.
fn mount(
	state: TableState,
	data: &'static [Planet],
) -> Result<(Arc<RwLock<TableState>>, WidgetTester), TestError> {
	let state = Arc::new(RwLock::new(state));
	let render = {
		let state = Arc::clone(&state);
		move || -> Element {
			let current = state.read().expect("uncontended").clone();
			let state = Arc::clone(&state);
			DataTable(
				&make_columns(),
				data,
				&current,
				&TableLabels::default(),
				move |next| *state.write().expect("uncontended") = next,
			)
		}
	};
	let tester = WidgetTester::new(render, 800.0, 600.0)?;
	Ok((state, tester))
}

fn header(tester: &WidgetTester, name: &str) -> Result<leptoskit_testing::Widget, TestError> {
	tester.find(&Finder::aria_role(Role::ColumnHeader, name))
}

#[test]
fn header_presses_sort_the_rendered_rows() -> Result<(), TestError> {
	let (_, mut tester) = mount(TableState::new().with_page_size(10), &DATA)?;

	// Source order on mount, and no header claims a sort yet.
	tester.find(&Finder::text("Jupiter"))?;
	let moons = header(&tester, "Moons")?;
	assert_eq!(moons.props.aria().extra.get("aria-sort"), None);

	// The header holds a real button: pressing it sorts ascending, and the
	// header announces the sort without the mark in its name.
	tester.tap(&Finder::aria_role(Role::Button, "Moons"))?;
	let moons = header(&tester, "Moons")?;
	assert_eq!(
		moons.props.aria().extra.get("aria-sort"),
		Some(&"ascending".to_owned())
	);
	let venus = tester.find(&Finder::text("Venus"))?;
	let mars = tester.find(&Finder::text("Mars"))?;
	assert!(venus.frame.y < mars.frame.y, "no moons sorts first");

	// Pressing again sorts descending: Saturn leads the body, Venus trails.
	tester.tap(&Finder::aria_role(Role::Button, "Moons"))?;
	let moons = header(&tester, "Moons")?;
	assert_eq!(
		moons.props.aria().extra.get("aria-sort"),
		Some(&"descending".to_owned())
	);
	let saturn = tester.find(&Finder::text("Saturn"))?;
	let venus = tester.find(&Finder::text("Venus"))?;
	assert!(saturn.frame.y < venus.frame.y, "Saturn leads Venus");
	assert!(
		saturn.frame.y > moons.frame.y,
		"the body follows the header"
	);

	// A third press clears the sort.
	tester.tap(&Finder::aria_role(Role::Button, "Moons"))?;
	let moons = header(&tester, "Moons")?;
	assert_eq!(moons.props.aria().extra.get("aria-sort"), None);
	Ok(())
}

#[test]
fn an_unsortable_header_is_not_a_control() -> Result<(), TestError> {
	let (_, tester) = mount(TableState::new(), &DATA)?;
	header(&tester, "Kind")?;
	assert!(
		tester
			.find(&Finder::aria_role(Role::Button, "Kind"))
			.is_err()
	);
	Ok(())
}

#[test]
fn typing_filters_the_rows_and_restarts_the_pages() -> Result<(), TestError> {
	let (state, mut tester) = mount(TableState::new().with_page_size(1), &DATA)?;
	tester.tap(&Finder::aria_role(Role::Button, "Next page"))?;
	assert_eq!(state.read().expect("uncontended").page, 1);

	tester.enter_text(&Finder::label("Filter rows"), "GAS")?;
	let current = state.read().expect("uncontended").clone();
	assert_eq!(current.filter, "GAS");
	assert_eq!(current.page, 0, "a new filter starts at the first page");
	tester.find(&Finder::text("Jupiter"))?;
	tester.find(&Finder::text("Page 1 of 2"))?;
	assert!(tester.find(&Finder::text("Mars")).is_err());
	Ok(())
}

#[test]
fn pagination_presses_move_between_pages() -> Result<(), TestError> {
	let (_, mut tester) = mount(TableState::new().with_page_size(2), &DATA)?;

	// Two rows to a page: the first page cannot go back.
	tester.find(&Finder::text("Jupiter"))?;
	tester.find(&Finder::text("Mars"))?;
	assert!(tester.find(&Finder::text("Venus")).is_err());
	assert_eq!(
		tester
			.find(&Finder::aria_role(Role::Button, "Previous page"))?
			.props
			.enabled,
		Some(false)
	);

	// The second page holds Saturn and Venus, and is the last.
	tester.tap(&Finder::aria_role(Role::Button, "Next page"))?;
	assert!(tester.find(&Finder::text("Jupiter")).is_err());
	tester.find(&Finder::text("Saturn"))?;
	tester.find(&Finder::text("Venus"))?;
	tester.find(&Finder::aria_role(Role::Status, "Page 2 of 2"))?;
	assert_eq!(
		tester
			.find(&Finder::aria_role(Role::Button, "Next page"))?
			.props
			.enabled,
		Some(false)
	);

	// The previous button returns to the first page.
	tester.tap(&Finder::aria_role(Role::Button, "Previous page"))?;
	tester.find(&Finder::text("Jupiter"))?;
	Ok(())
}

#[test]
fn an_empty_table_and_an_unmatched_filter_render_one_empty_page() -> Result<(), TestError> {
	let (_, tester) = mount(TableState::new().with_page_size(2), &[])?;
	header(&tester, "Name")?;
	tester.find(&Finder::text("Page 1 of 1"))?;

	let filtered = TableState {
		filter: "pluto".to_owned(),
		..TableState::new().with_page_size(2)
	};
	let (_, tester) = mount(filtered, &DATA)?;
	assert!(tester.find(&Finder::text("Jupiter")).is_err());
	tester.find(&Finder::text("Page 1 of 1"))?;
	Ok(())
}

#[test]
fn a_zero_page_size_shows_every_row_without_a_pager() -> Result<(), TestError> {
	let (_, tester) = mount(TableState::new().with_page_size(0), &DATA)?;

	for planet in &DATA {
		tester.find(&Finder::text(planet.name))?;
	}

	assert!(
		tester
			.find(&Finder::aria_role(Role::Button, "Next page"))
			.is_err()
	);
	Ok(())
}

#[test]
fn labels_translate_the_controls() -> Result<(), TestError> {
	let labels = TableLabels {
		filter: "Filtrar".to_owned(),
		previous_page: "Anterior".to_owned(),
		next_page: "Siguiente".to_owned(),
		page_status: Arc::new(|page, pages| format!("Página {page} de {pages}")),
	};
	let tester = WidgetTester::new(
		move || {
			DataTable(
				&make_columns(),
				&DATA,
				&TableState::new().with_page_size(2),
				&labels,
				|_| (),
			)
		},
		800.0,
		600.0,
	)?;
	tester.find(&Finder::label("Filtrar"))?;
	tester.find(&Finder::aria_role(Role::Button, "Anterior"))?;
	tester.find(&Finder::aria_role(Role::Button, "Siguiente"))?;
	tester.find(&Finder::text("Página 1 de 2"))?;
	Ok(())
}
