//! The sky log the way an observer uses it: scrolling a year of rows,
//! narrowing to a night from the ribbon, opening rows, and recording live —
//! over the real renderer on the headless backend.
//!
//! The headless tester dispatches scroll events without moving content, so
//! rows deep in a scrolled list are committed but off-screen; the helpers
//! here read the committed tree rather than what is visible.
//!
//! Row sizes are whole points, so frame offsets compare exactly.
#![allow(clippy::float_cmp)]

use leptoskit::native::Role;
use leptoskit_testing::Finder;
use leptoskit_testing::TestError;
use leptoskit_testing::WidgetTester;
use observatory_example::Observatory;
use observatory_example::VIEWPORT;
use observatory_example::grouped;
use observatory_example::log::LogEvent;
use observatory_example::sky;
use observatory_example::sky::NIGHTS;

fn mount() -> Result<(Observatory, WidgetTester), TestError> {
	let observatory = Observatory::new();
	let render = observatory.clone();
	let tester = WidgetTester::new(move || render.render(), VIEWPORT.0, VIEWPORT.1)?;
	Ok((observatory, tester))
}

/// The accessible names of every committed widget whose name starts with
/// `prefix`, in tree order.
fn committed(tester: &WidgetTester, prefix: &str) -> Vec<String> {
	let surface = tester.surface();
	let mut pending: Vec<_> = surface.root().into_iter().collect();
	let mut names = Vec::new();
	while let Some(id) = pending.pop() {
		if let Some(name) = surface.props(id).accessible_name() {
			if name.starts_with(prefix) {
				names.push(name.to_owned());
			}
		}
		pending.extend(surface.children(id).iter().rev().copied());
	}
	names
}

#[test]
fn the_sky_is_the_same_every_run_and_in_order() {
	let log = sky::observations();
	assert_eq!(log, sky::observations());
	assert!((14_600..=26_000).contains(&log.len()), "40 to 71 a night");
	assert!(
		log.windows(2)
			.all(|pair| { (pair[0].night, pair[0].minute) < (pair[1].night, pair[1].minute) })
	);
	assert_eq!(log.first().map(|first| first.night), Some(1));
	assert_eq!(log.last().map(|last| last.night), Some(NIGHTS));
	assert_eq!(sky::moon_phase(1), '🌑');
	assert_eq!(sky::moon_phase(15), '🌕');
}

#[test]
fn a_year_of_rows_mounts_only_the_window() -> Result<(), TestError> {
	let (observatory, mut tester) = mount()?;
	let rows = observation_rows(&tester);
	assert!(observatory.log_count() > 14_000);
	assert!(
		(9..=20).contains(&rows.len()),
		"a 400-point window of 52- and 76-point rows plus overscan, not {} rows",
		rows.len()
	);
	assert!(rows[0].starts_with("Night 1, 18:"));
	assert_eq!(
		readout(&tester),
		[format!("At night 1, {}", sky::observations()[0].time())]
	);

	// Scrolling a long way down swaps the whole window and moves the readout.
	tester.scroll(&Finder::label("Observations"), 0.0, 400_000.0)?;
	let deep = observation_rows(&tester);
	assert!(!deep.iter().any(|name| name.starts_with("Night 1,")));
	let nights: Vec<u16> = deep.iter().map(|name| night_of(name)).collect();
	assert!(
		nights.iter().all(|&night| night > 100),
		"deep in the year: {nights:?}"
	);

	let [position] = readout(&tester).try_into().expect("one readout");
	let night = night_of(&position["At n".len()..]);
	assert!(
		nights.contains(&night),
		"the readout names a row in the window"
	);
	Ok(())
}

/// The accessible names of the committed log rows.
fn observation_rows(tester: &WidgetTester) -> Vec<String> {
	committed(tester, "Night ")
		.into_iter()
		.filter(|name| name.contains("seeing"))
		.collect()
}

/// The readout of where the log is.
fn readout(tester: &WidgetTester) -> Vec<String> {
	committed(tester, "At night ")
}

/// The night a row or readout names: `Night 12, …` or `ight 12, …`.
fn night_of(name: &str) -> u16 {
	name.split_once(' ')
		.and_then(|(_, rest)| rest.split(',').next())
		.and_then(|night| night.parse().ok())
		.unwrap_or_else(|| panic!("{name:?} names no night"))
}

#[test]
fn the_ribbon_narrows_the_log_to_one_night_and_back() -> Result<(), TestError> {
	let (observatory, mut tester) = mount()?;
	let every_night = observatory.log_count();
	assert_eq!(
		tester
			.find(&Finder::aria_role(Role::Button, "All nights"))?
			.props
			.enabled,
		Some(false)
	);

	tester.tap(&Finder::label("Night 3"))?;
	assert_eq!(observatory.night(), Some(3));
	let night_three = observatory.log_count();
	assert!((40..72).contains(&night_three));
	tester.find(&Finder::text(format!(
		"Night 3 · {night_three} observations"
	)))?;
	tester
		.find(&Finder::text(format!(
			"Every night · {} observations",
			grouped(every_night)
		)))
		.expect_err("the header names the night shown");
	assert!(
		observation_rows(&tester)
			.iter()
			.all(|name| name.starts_with("Night 3,"))
	);
	let card = tester.find(&Finder::label("Night 3"))?;
	assert_eq!(card.props.aria().pressed, Some(true.into()));

	// Pressing the chosen night again, or All nights, shows every night.
	tester.tap(&Finder::label("Night 3"))?;
	assert_eq!(observatory.night(), None);
	tester.tap(&Finder::label("Night 2"))?;
	tester.tap(&Finder::aria_role(Role::Button, "All nights"))?;
	assert_eq!(observatory.log_count(), every_night);
	Ok(())
}

#[test]
fn the_ribbon_is_a_horizontal_window_of_nights() -> Result<(), TestError> {
	let (_, mut tester) = mount()?;
	let cards = |tester: &WidgetTester| {
		committed(tester, "Night ")
			.into_iter()
			.filter(|name| !name.contains(','))
			.collect::<Vec<_>>()
	};
	// 560 points of 64-point cards, plus three of overscan after.
	assert_eq!(cards(&tester).len(), 12);
	assert_eq!(cards(&tester)[0], "Night 1");

	tester.scroll(&Finder::label("Nights"), 6_400.0, 0.0)?;
	let scrolled = cards(&tester);
	assert_eq!(scrolled.first().map(String::as_str), Some("Night 98"));
	assert!(scrolled.iter().any(|card| card == "Night 101"));
	Ok(())
}

#[test]
fn an_opened_row_shows_its_star_and_pushes_the_next_row_down() -> Result<(), TestError> {
	let (_, mut tester) = mount()?;
	let first = observatory_first_row(&tester);
	let second = tester.find(&Finder::label(&first.1))?;
	let opened = tester.find(&Finder::label(&first.0))?;
	let before = second.frame.y - opened.frame.y;

	tester.tap(&Finder::label(&first.0))?;
	let opened = tester.find(&Finder::label(&first.0))?;
	assert_eq!(opened.props.aria().expanded, Some(true));
	let second = tester.find(&Finder::label(&first.1))?;
	assert_eq!(second.frame.y - opened.frame.y, before + 48.0);

	// The details name the star's constellation and class.
	let star = sky::observations()[0].star();
	assert!(
		committed(&tester, star.constellation)
			.iter()
			.any(|text| text.contains(star.class))
	);

	// Closing it puts the next row back.
	tester.tap(&Finder::label(&first.0))?;
	let second = tester.find(&Finder::label(&first.1))?;
	let opened = tester.find(&Finder::label(&first.0))?;
	assert_eq!(second.frame.y - opened.frame.y, before);
	Ok(())
}

#[test]
fn recording_appends_without_losing_an_opened_row() -> Result<(), TestError> {
	let (observatory, mut tester) = mount()?;
	let (first, _) = observatory_first_row(&tester);
	tester.tap(&Finder::label(&first))?;
	let count = observatory.log_count();

	tester.tap(&Finder::aria_role(Role::Button, "Record an observation"))?;
	assert_eq!(observatory.log_count(), count + 1);
	tester.find(&Finder::text(format!(
		"Every night · {} observations",
		grouped(count + 1)
	)))?;
	assert_eq!(
		tester.find(&Finder::label(&first))?.props.aria().expanded,
		Some(true),
		"set_count keeps earlier measurements"
	);

	// On another night, the new row is recorded but not shown.
	observatory.log_event(LogEvent::SelectNight(Some(1)));
	let night_one = observatory.log_count();
	observatory.log_event(LogEvent::Record);
	assert_eq!(observatory.log_count(), night_one);
	Ok(())
}

/// The accessible names of the first two log rows.
fn observatory_first_row(tester: &WidgetTester) -> (String, String) {
	let rows = observation_rows(tester);
	(rows[0].clone(), rows[1].clone())
}

#[test]
fn counts_group_their_thousands() {
	assert_eq!(grouped(7), "7");
	assert_eq!(grouped(999), "999");
	assert_eq!(grouped(1_000), "1,000");
	assert_eq!(grouped(20_151), "20,151");
	assert_eq!(grouped(1_234_567), "1,234,567");
}
