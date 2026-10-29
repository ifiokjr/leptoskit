//! The star catalogue: a paginated, sortable, searchable table of the
//! brightest stars, and a hand-built grid of the three brightest.

use std::num::NonZeroUsize;
use std::sync::Arc;

use leptoskit::native::Role;
use leptoskit::prelude::*;
use leptoskit::table::Column;
use leptoskit::table::DataTable;
use leptoskit::table::SortDirection;
use leptoskit::table::SortSpec;
use leptoskit::table::TableLabels;
use leptoskit::table::TableState;
use leptoskit::table::page_count;
use leptoskit::table::rows;

use crate::sky::STARS;
use crate::sky::Star;

/// Stars per catalogue plate.
pub const PLATE_SIZE: usize = 8;

/// The catalogue as the observatory first shows it: unsorted, unfiltered,
/// on its first plate.
pub fn initial_state() -> TableState {
	TableState::new().with_page_size(PLATE_SIZE)
}

/// The catalogue's columns. Names sort by their text; magnitudes and
/// distances sort by their numbers, so `10 ly` follows `9 ly`; and the
/// magnitude column also answers searches like `mag<1`.
pub fn columns() -> Vec<Column<Star>> {
	vec![
		Column::new("name", "Star", |star: &Star| star.name.to_owned()).sortable(),
		Column::new("constellation", "Constellation", |star: &Star| {
			star.constellation.to_owned()
		}),
		Column::new("magnitude", "Magnitude", |star: &Star| {
			magnitude_text(star.magnitude)
		})
		.sort_by(|a, b| a.magnitude.total_cmp(&b.magnitude))
		.filter_by(matches_magnitude),
		Column::new("distance", "Distance", |star: &Star| {
			distance_text(star.distance)
		})
		.sort_by(|a, b| a.distance.total_cmp(&b.distance)),
		Column::new("class", "Class", |star: &Star| star.class.to_owned()),
	]
}

/// The catalogue's words: an observatory pages through photographic plates.
pub fn labels() -> TableLabels {
	TableLabels {
		filter: "Search stars, or mag<1".to_owned(),
		previous_page: "Previous plate".to_owned(),
		next_page: "Next plate".to_owned(),
		page_status: Arc::new(|plate, plates| format!("Plate {plate} of {plates}")),
	}
}

/// The catalogue section: its heading, a hint built from the columns that
/// sort, the table, and the brightest three.
pub fn catalogue(
	state: &TableState,
	on_change: impl Fn(TableState) + Send + Sync + 'static,
) -> Element {
	let columns = columns();
	let sortable: Vec<&str> = columns
		.iter()
		.filter(|column| column.is_sortable())
		.map(|column| column.header.as_str())
		.collect();
	let plates = page_count(STARS.len(), NonZeroUsize::new(PLATE_SIZE));

	view! {
		<View class="gap-3 w-[620px]">
			<Heading aria_level=2 class="text-2xl font-bold text-slate-100">"Catalogue"</Heading>
			<Text class="text-sm text-slate-400">
				{format!(
					"{} stars on {plates} plates. Sort by {}.",
					STARS.len(),
					sortable.join(", ")
				)}
			</Text>
			{DataTable(&columns, &STARS, state, &labels(), on_change)
				.class("rounded-xl bg-slate-900 p-3 text-slate-100")}
			{brightest()}
		</View>
	}
}

/// The three brightest stars, whatever the catalogue's own sort: a custom
/// grid built from the same pipeline and roles the table uses.
pub fn brightest() -> Element {
	let columns = columns();
	let podium = TableState {
		sorting: Some(SortSpec {
			column: "magnitude".to_owned(),
			direction: SortDirection::Ascending,
		}),
		..TableState::new().with_page_size(3)
	};
	let shown: Vec<&Column<Star>> = columns
		.iter()
		.filter(|column| matches!(column.id.as_str(), "name" | "magnitude"))
		.collect();

	let stars = rows(&STARS, &columns, &podium).rows;

	view! {
		<View class="gap-2 rounded-xl bg-slate-900 p-3 text-slate-100">
			<Heading aria_level=3 class="font-semibold">"Brightest tonight"</Heading>
			<View role=Role::Table aria_label="Brightest stars" class="gap-1">
				<View role=Role::Row class="flex-row">
					<Text role=Role::ColumnHeader class="w-[60px]">"Rank"</Text>
					{shown
						.iter()
						.map(|column| {
							view! {
								<Text role=Role::ColumnHeader class="flex-1">
									{column.header.clone()}
								</Text>
							}
						})
						.collect::<Fragment>()}
				</View>
				{stars
					.iter()
					.enumerate()
					.map(|(rank, star)| {
						view! {
							<View role=Role::Row class="flex-row">
								<Text role=Role::Cell class="w-[60px] text-amber-300">
									{format!("#{}", rank + 1)}
								</Text>
								{shown
									.iter()
									.map(|column| {
										view! {
											<Text role=Role::Cell class="flex-1">
												{column.cell_text(star)}
											</Text>
										}
									})
									.collect::<Fragment>()}
							</View>
						}
					})
					.collect::<Fragment>()}
			</View>
		</View>
	}
}

/// A magnitude as an almanac prints it, with a true minus sign.
pub fn magnitude_text(magnitude: f64) -> String {
	if magnitude < 0.0 {
		format!("−{:.2}", -magnitude)
	} else {
		format!("{magnitude:.2}")
	}
}

/// A distance in light years, to a tenth for the nearest stars.
pub fn distance_text(distance: f64) -> String {
	if distance < 100.0 {
		format!("{distance:.1} ly")
	} else {
		format!("{distance:.0} ly")
	}
}

/// The magnitude column's part of the search: `mag<1` or `mag>1.5` compare
/// brightness, and anything else matches the printed magnitude.
fn matches_magnitude(star: &Star, query: &str) -> bool {
	let query = query.trim();
	let comparison = query
		.strip_prefix("mag<")
		.map(|limit| (limit, true))
		.or_else(|| query.strip_prefix("mag>").map(|limit| (limit, false)));

	match comparison {
		Some((limit, brighter)) => {
			limit.trim().parse::<f64>().is_ok_and(|limit| {
				if brighter {
					star.magnitude < limit
				} else {
					star.magnitude > limit
				}
			})
		}
		None => magnitude_text(star.magnitude).contains(query),
	}
}
