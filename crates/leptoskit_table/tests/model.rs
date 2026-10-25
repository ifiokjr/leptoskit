//! The row pipeline and its reducers: sort cycles, filtering, pagination.

use leptoskit_table::Column;
use leptoskit_table::SortDirection;
use leptoskit_table::SortSpec;
use leptoskit_table::TableState;
use leptoskit_table::for_filter;
use leptoskit_table::for_header_press;
use leptoskit_table::for_page;
use leptoskit_table::page_count;
use leptoskit_table::rows;

struct Planet {
	name: &'static str,
	moons: u8,
}

fn columns() -> Vec<Column<Planet>> {
	vec![
		Column::new("name", "Name", |planet: &Planet| planet.name.to_owned()).sortable(),
		Column::new("moons", "Moons", |planet: &Planet| planet.moons.to_string())
			.sort_by(|a, b| a.moons.cmp(&b.moons)),
	]
}

fn data() -> Vec<Planet> {
	vec![
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
	]
}

fn names(state: &TableState) -> Vec<&'static str> {
	rows(&data(), &columns(), state)
		.rows
		.iter()
		.map(|planet| planet.name)
		.collect()
}

#[test]
fn a_header_press_cycles_none_ascending_descending_none() {
	let mut state = TableState::new();

	state = for_header_press(&state, "moons");
	assert_eq!(
		state.sorting,
		Some(SortSpec {
			column: "moons".to_owned(),
			direction: SortDirection::Ascending,
		})
	);

	state = for_header_press(&state, "moons");
	assert_eq!(
		state.sorting,
		Some(SortSpec {
			column: "moons".to_owned(),
			direction: SortDirection::Descending,
		})
	);

	state = for_header_press(&state, "moons");
	assert_eq!(state.sorting, None);
}

#[test]
fn sorting_orders_rows_by_the_active_column() {
	let ascending = TableState {
		sorting: Some(SortSpec {
			column: "moons".to_owned(),
			direction: SortDirection::Ascending,
		}),
		..TableState::new()
	};
	assert_eq!(names(&ascending), ["Venus", "Mars", "Jupiter", "Saturn"]);

	let descending = TableState {
		sorting: Some(SortSpec {
			column: "moons".to_owned(),
			direction: SortDirection::Descending,
		}),
		..TableState::new()
	};
	assert_eq!(names(&descending), ["Saturn", "Jupiter", "Mars", "Venus"]);

	// The text sort reads the cell, not the row.
	let by_name = TableState {
		sorting: Some(SortSpec {
			column: "name".to_owned(),
			direction: SortDirection::Ascending,
		}),
		..TableState::new()
	};
	assert_eq!(names(&by_name), ["Jupiter", "Mars", "Saturn", "Venus"]);

	// A sort on an id no column carries leaves the source order.
	let ghost = TableState {
		sorting: Some(SortSpec {
			column: "distance".to_owned(),
			direction: SortDirection::Ascending,
		}),
		..TableState::new()
	};
	assert_eq!(names(&ghost), ["Jupiter", "Mars", "Saturn", "Venus"]);
}

#[test]
fn the_global_filter_matches_any_cells_text_case_insensitively() {
	let state = for_filter(&TableState::new(), "mars");
	assert_eq!(names(&state), ["Mars"]);

	let state = for_filter(&TableState::new(), "s");
	assert_eq!(names(&state), ["Mars", "Saturn", "Venus"]);

	let state = for_filter(&TableState::new(), "146");
	assert_eq!(names(&state), ["Saturn"]);

	// A filter that keeps nothing still resolves to a page of nothing.
	let state = for_filter(&TableState::new(), "pluto");
	let (data, columns) = (data(), columns());
	let page = rows(&data, &columns, &state);
	assert_eq!(page.total, 0);
	assert!(page.rows.is_empty());
	assert_eq!(page.pages, 1);
}

#[test]
fn a_column_can_define_its_own_filter() {
	let columns = vec![
		Column::new("name", "Name", |planet: &Planet| planet.name.to_owned()),
		Column::new("many", "Many moons", |planet: &Planet| {
			(planet.moons > 50).to_string()
		})
		.filter_by(|planet, query| query.eq_ignore_ascii_case("many") == (planet.moons > 50)),
	];
	let state = for_filter(&TableState::new(), "many");
	let data = data();
	let page = rows(&data, &columns, &state);
	assert_eq!(
		page.rows
			.iter()
			.map(|planet| planet.name)
			.collect::<Vec<_>>(),
		["Jupiter", "Saturn"]
	);
}

#[test]
fn pagination_slices_the_filtered_rows_and_clamps() {
	let state = TableState::new().with_page_size(2);
	let (data, columns_) = (data(), columns());
	let page = rows(&data, &columns_, &state);
	assert_eq!(page.pages, 2);
	assert_eq!(page.total, 4);
	assert_eq!(
		page.rows
			.iter()
			.map(|planet| planet.name)
			.collect::<Vec<_>>(),
		["Jupiter", "Mars"]
	);
	assert_eq!(page.page_number(&state), 1);

	let next = for_page(&state, page.total, 1);
	let page = rows(&data, &columns_, &next);
	assert_eq!(
		page.rows
			.iter()
			.map(|planet| planet.name)
			.collect::<Vec<_>>(),
		["Saturn", "Venus"]
	);

	// Page turns clamp to the last real page, and filtering restarts at one.
	let jumped = for_page(&state, page.total, 99);
	assert_eq!(jumped.page, 1);
	let filtered = for_filter(&state, "mars");
	assert_eq!(filtered.page, 0);

	// A page size of zero is no pagination, the same as `None`.
	assert_eq!(page_count(4, None), 1);
	assert_eq!(page_count(4, Some(0)), 1);
	assert_eq!(page_count(0, Some(2)), 1);
}

#[test]
fn sorting_is_stable_for_equal_keys() {
	let columns =
		vec![Column::new("name", "Name", |planet: &Planet| planet.name.to_owned()).sortable()];
	let state = TableState {
		sorting: Some(SortSpec {
			column: "name".to_owned(),
			direction: SortDirection::Ascending,
		}),
		..TableState::new()
	};
	let data = data();
	let first = rows(&data, &columns, &state);
	let second = rows(&data, &columns, &state);
	let first_names: Vec<_> = first.rows.iter().map(|p| p.name).collect();
	let second_names: Vec<_> = second.rows.iter().map(|p| p.name).collect();
	assert_eq!(first_names, second_names);
}
