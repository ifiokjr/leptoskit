//! The shared gallery screens, rendered by the terminal backend: the same
//! screen functions every other host renders, committed into a cell buffer.
#![cfg(not(target_arch = "wasm32"))]

use gallery::Assets;
use gallery::SCREENS;
use gallery::Screen;
use gallery::tui::GalleryApp;
use leptoskit_tui::SourceEvent;
use leptoskit_tui::TuiHost;
use leptoskit_tui::TuiOptions;
use leptoskit_tui::testing::Recorder;
use leptoskit_tui::testing::Scripted;

fn lines(screen: &'static Screen, events: Vec<SourceEvent>) -> Vec<String> {
	let (cols, rows) = gallery::tui::cells(screen.viewport);

	let mut app = GalleryApp {
		session: gallery::native::NativeGallery::new(
			leptoskit_tui::TuiBackend::new(),
			screen,
			Assets::at("assets"),
			screen.viewport,
		),
	};
	let mut sink = Recorder::new(cols.max(1), rows.max(1));
	let mut source = Scripted::new(events);

	TuiHost::new(TuiOptions::default())
		.run(&mut app, &mut source, &mut sink)
		.expect("terminal session");

	sink.lines()
}

#[test]
fn every_screen_paints_into_the_cell_grid() {
	for screen in &SCREENS {
		let lines = lines(screen, Vec::new());
		let text = lines.join("\n");
		assert!(!text.trim().is_empty(), "{} painted nothing", screen.slug);
		insta::assert_snapshot!(format!("screen_{}", screen.slug), text);
	}
}

#[test]
fn the_counter_screen_presses_through_the_focus_ring() {
	let screen = gallery::find_screen("counter").expect("counter screen");
	assert_eq!(
		count_of(&lines(screen, Vec::new())),
		gallery::ScreenState::INITIAL_COUNT
	);

	// A click on "Add one", laid out at the buttons row of the rendered
	// screen, presses through the shared handler.
	let pressed = lines(
		screen,
		vec![SourceEvent::Mouse(leptoskit_tui::Mouse::Click {
			col: 20,
			row: 43,
		})],
	);

	assert_eq!(count_of(&pressed), gallery::ScreenState::INITIAL_COUNT + 1);
}

/// The count from the rendered "N of 20" progress line, whose row may carry
/// the card's border glyphs.
fn count_of(lines: &[String]) -> i32 {
	lines
		.iter()
		.find_map(|line| {
			let index = line.find(" of ")?;
			let digits: String = line[..index]
				.chars()
				.filter(|symbol| symbol.is_ascii_digit() || *symbol == '-')
				.collect();
			digits.parse::<i32>().ok()
		})
		.unwrap_or(i32::MIN)
}
