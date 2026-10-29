//! The catalogue the way an astronomer uses it: paging through plates,
//! sorting by brightness and distance, and searching with words and
//! magnitude limits — over the real renderer on the headless backend.

use leptoskit::native::Role;
use leptoskit::table::rows;
use leptoskit_testing::Finder;
use leptoskit_testing::TestError;
use leptoskit_testing::WidgetTester;
use observatory_example::Observatory;
use observatory_example::VIEWPORT;
use observatory_example::catalogue;
use observatory_example::sky::STARS;

fn mount() -> Result<(Observatory, WidgetTester), TestError> {
	let observatory = Observatory::new();
	let render = observatory.clone();
	let tester = WidgetTester::new(move || render.render(), VIEWPORT.0, VIEWPORT.1)?;
	Ok((observatory, tester))
}

/// The star names the catalogue's current plate shows, in order.
fn plate(observatory: &Observatory) -> Vec<&'static str> {
	rows(
		&STARS,
		&catalogue::columns(),
		&observatory.catalogue_state(),
	)
	.rows
	.iter()
	.map(|star| star.name)
	.collect()
}

#[test]
fn the_first_plate_shows_eight_stars_in_catalogue_order() -> Result<(), TestError> {
	let (observatory, tester) = mount()?;
	tester.find(&Finder::text("Plate 1 of 6"))?;
	tester.find(&Finder::text(
		"44 stars on 6 plates. Sort by Star, Magnitude, Distance.",
	))?;
	assert_eq!(
		plate(&observatory),
		[
			"Sirius",
			"Canopus",
			"Rigil Kentaurus",
			"Arcturus",
			"Vega",
			"Capella",
			"Rigel",
			"Procyon"
		]
	);
	tester.find(&Finder::text("Procyon"))?;
	assert!(
		tester.find(&Finder::text("Achernar")).is_err(),
		"the ninth star is on plate 2"
	);
	Ok(())
}

#[test]
fn distance_sorts_by_number_and_reverses_on_a_second_press() -> Result<(), TestError> {
	let (observatory, mut tester) = mount()?;

	tester.tap(&Finder::aria_role(Role::Button, "Distance"))?;
	// 4.4 ly before 8.6 ly before 11.5 ly: numbers, not text.
	assert_eq!(
		plate(&observatory)[..3],
		["Rigil Kentaurus", "Sirius", "Procyon"]
	);
	let header = tester.find(&Finder::aria_role(Role::ColumnHeader, "Distance"))?;
	assert_eq!(
		header
			.props
			.aria()
			.extra
			.get("aria-sort")
			.map(String::as_str),
		Some("ascending")
	);

	tester.tap(&Finder::aria_role(Role::Button, "Distance"))?;
	assert_eq!(plate(&observatory)[..2], ["Deneb", "Alnilam"]);
	tester.find(&Finder::text("2600 ly"))?;
	Ok(())
}

#[test]
fn the_constellation_column_reads_but_does_not_sort() -> Result<(), TestError> {
	let (_, tester) = mount()?;
	tester.find(&Finder::aria_role(Role::ColumnHeader, "Constellation"))?;
	assert!(
		tester
			.find(&Finder::aria_role(Role::Button, "Constellation"))
			.is_err()
	);
	Ok(())
}

#[test]
fn a_magnitude_limit_finds_the_brightest_stars() -> Result<(), TestError> {
	let (observatory, mut tester) = mount()?;
	tester.tap(&Finder::aria_role(Role::Button, "Next plate"))?;
	assert_eq!(observatory.catalogue_state().page, 1);

	tester.enter_text(&Finder::label("Search stars, or mag<1"), "mag<0.5")?;
	assert_eq!(
		observatory.catalogue_state().page,
		0,
		"a search starts at the first plate"
	);
	assert_eq!(
		plate(&observatory),
		[
			"Sirius",
			"Canopus",
			"Rigil Kentaurus",
			"Arcturus",
			"Vega",
			"Capella",
			"Rigel",
			"Procyon"
		]
	);
	tester.find(&Finder::text("Plate 1 of 2"))?;

	tester.tap(&Finder::aria_role(Role::Button, "Next plate"))?;
	assert_eq!(plate(&observatory), ["Achernar"]);
	assert_eq!(
		tester
			.find(&Finder::aria_role(Role::Button, "Next plate"))?
			.props
			.enabled,
		Some(false)
	);
	Ok(())
}

#[test]
fn a_word_search_matches_any_column_and_dim_limits_too() -> Result<(), TestError> {
	let (observatory, mut tester) = mount()?;
	tester.enter_text(&Finder::label("Search stars, or mag<1"), "orion")?;
	assert_eq!(
		plate(&observatory),
		["Rigel", "Betelgeuse", "Bellatrix", "Alnilam", "Alnitak"]
	);

	tester.enter_text(&Finder::label("Search stars, or mag<1"), "mag>1.9")?;
	assert_eq!(
		plate(&observatory),
		["Atria", "Alhena", "Peacock", "Polaris"]
	);

	// A printed magnitude matches as text, minus sign and all.
	tester.enter_text(&Finder::label("Search stars, or mag<1"), "−1.46")?;
	assert_eq!(plate(&observatory), ["Sirius"]);
	Ok(())
}

#[test]
fn the_brightest_three_ignore_the_catalogue_sort() -> Result<(), TestError> {
	let (_, mut tester) = mount()?;
	tester.tap(&Finder::aria_role(Role::Button, "Distance"))?;

	tester.find(&Finder::label("Brightest stars"))?;
	for (rank, name, magnitude) in [
		("#1", "Sirius", "−1.46"),
		("#2", "Canopus", "−0.74"),
		("#3", "Rigil Kentaurus", "−0.27"),
	] {
		tester.find(&Finder::aria_role(Role::Cell, rank))?;
		assert_ne!(
			tester.find_all(&Finder::text(name)),
			[] as [leptoskit_testing::Widget; 0]
		);
		assert_ne!(
			tester.find_all(&Finder::text(magnitude)),
			[] as [leptoskit_testing::Widget; 0]
		);
	}
	Ok(())
}

#[test]
fn magnitudes_and_distances_print_like_an_almanac() {
	assert_eq!(catalogue::magnitude_text(-1.46), "−1.46");
	assert_eq!(catalogue::magnitude_text(0.03), "0.03");
	assert_eq!(catalogue::distance_text(4.4), "4.4 ly");
	assert_eq!(catalogue::distance_text(2600.0), "2600 ly");
}
