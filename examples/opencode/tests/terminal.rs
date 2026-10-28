//! The routed interface driven headlessly through the terminal host loop:
//! typing, submitting, the palette, and agent switching, at the sizes
//! opencode's layout changes at.

use leptoskit_native::NativeSurface;
use leptoskit_tui::Buffer;
use leptoskit_tui::Exit;
use leptoskit_tui::Key;
use leptoskit_tui::KeyCode;
use leptoskit_tui::SourceEvent;
use leptoskit_tui::TuiBackend;
use leptoskit_tui::TuiColor;
use leptoskit_tui::TuiHost;
use leptoskit_tui::TuiOptions;
use leptoskit_tui::testing::Recorder;
use leptoskit_tui::testing::Scripted;
use opencode_example::Agent;
use opencode_example::Store;
use opencode_example::routes::Navigation;
use opencode_example::routes::Route;

const PROMPT: &str = "The timer test fails. Can you fix it?";

fn typed(text: &str) -> Vec<SourceEvent> {
	text.chars()
		.map(|symbol| SourceEvent::Key(Key::plain(KeyCode::Char(symbol))))
		.collect()
}

fn key(code: KeyCode) -> SourceEvent {
	SourceEvent::Key(Key::plain(code))
}

fn ctrl(symbol: char) -> SourceEvent {
	SourceEvent::Key(Key::controlled(KeyCode::Char(symbol)))
}

/// Runs the routed app through the terminal host loop, the way the
/// generated runner does, and returns its router, store, and last frame.
fn run(events: Vec<SourceEvent>, cols: u16, rows: u16) -> (Navigation, Store, Buffer) {
	let store = Store::current();
	store.reset();
	store.resize(cols, rows);
	let navigation = Navigation::new(Route::Home);
	let mut surface = NativeSurface::new(TuiBackend::new());
	let mut app = leptoskit_tui::testing::surface_app(&mut surface, {
		let navigation = navigation.clone();
		move || opencode_example::screen(&navigation)
	});
	let mut source = Scripted::new(events);
	let mut sink = Recorder::new(cols, rows);

	let exit = TuiHost::new(TuiOptions::default())
		.run(&mut *app, &mut source, &mut sink)
		.expect("session runs");
	assert_eq!(exit, Exit::SourceEnded);

	let frame = sink.frames.pop().expect("a frame was drawn");
	(navigation, store, frame)
}

fn submitted() -> Vec<SourceEvent> {
	let mut events = typed(PROMPT);
	events.push(key(KeyCode::Enter));
	events
}

fn text(frame: &Buffer) -> String {
	frame
		.to_plain_lines()
		.iter()
		.map(|line| line.trim_end())
		.collect::<Vec<_>>()
		.join("\n")
}

#[test]
fn home_centers_the_logo_and_prompt() {
	let (navigation, _, frame) = run(Vec::new(), 120, 36);

	assert_eq!(navigation.current(), Route::Home);
	// The caret is the terminal's cursor, at the start of the empty prompt.
	assert_eq!(frame.cursor(), Some((26, 19)));
	assert_eq!(
		frame.cell(23, 18).unwrap().fg,
		TuiColor::Rgb(0x5c, 0x9c, 0xf5)
	);
	insta::assert_snapshot!(text(&frame));
}

#[test]
fn submitting_plays_the_scripted_session() {
	let (navigation, store, frame) = run(submitted(), 120, 36);

	assert_eq!(navigation.current(), Route::Session);
	assert!(store.with(|state| state.draft.is_empty()));
	insta::assert_snapshot!(text(&frame));
}

#[test]
fn wide_sessions_show_the_sidebar() {
	let (_, _, frame) = run(submitted(), 160, 45);
	insta::assert_snapshot!(text(&frame));
}

#[test]
fn wider_sessions_split_the_diff() {
	let (_, _, frame) = run(submitted(), 200, 50);
	insta::assert_snapshot!(text(&frame));
}

#[test]
fn the_palette_dims_the_screen_and_takes_focus() {
	let mut events = submitted();
	events.extend([ctrl('p'), key(KeyCode::Down), key(KeyCode::Down)]);
	let (_, store, frame) = run(events, 160, 45);

	let selected = store.with(|state| state.palette.as_ref().map(|palette| palette.selected));
	assert_eq!(selected, Some(2), "the palette is open");
	// The search field holds the cursor, and the scrim dims what is behind.
	assert_eq!(frame.cursor(), Some((54, 14)));
	assert_eq!(frame.cell(0, 44).unwrap().bg, TuiColor::Rgb(4, 4, 4));
	insta::assert_snapshot!(text(&frame));
}

#[test]
fn the_palette_filters_and_closes() {
	let mut events = submitted();
	events.push(ctrl('p'));
	events.extend(typed("model"));
	let (_, store, frame) = run(events.clone(), 160, 45);
	let query = store.with(|state| state.palette.as_ref().map(|palette| palette.query.clone()));
	assert_eq!(query.as_deref(), Some("model"));
	assert!(text(&frame).contains("Switch model"));
	assert!(!text(&frame).contains("Clear session"));

	events.push(key(KeyCode::Escape));
	let (_, store, frame) = run(events, 160, 45);
	assert!(store.with(|state| state.palette.is_none()));
	assert_eq!(frame.cursor(), Some((5, 39)), "focus returns to the prompt");
}

#[test]
fn new_session_returns_home() {
	let mut events = submitted();
	events.push(ctrl('p'));
	events.extend(typed("new"));
	events.push(key(KeyCode::Enter));
	let (navigation, store, _) = run(events, 120, 36);

	assert!(store.with(|state| state.palette.is_none()));
	assert_eq!(navigation.current(), Route::Home);
	assert!(store.with(|state| state.session.messages.is_empty()));
}

#[test]
fn shift_tab_switches_agents_and_recolors_the_prompt() {
	let (_, store, frame) = run(vec![key(KeyCode::BackTab)], 120, 36);

	assert_eq!(store.with(|state| state.agent), Agent::Plan);
	assert_eq!(
		frame.cell(23, 18).unwrap().fg,
		TuiColor::Rgb(0x9d, 0x7c, 0xd8)
	);
	assert!(text(&frame).contains("Plan · Demo Local"));
}

#[test]
fn later_prompts_get_the_offline_reply() {
	let mut events = submitted();
	events.extend(typed("Thanks!"));
	events.push(key(KeyCode::Enter));
	let (_, store, frame) = run(events, 120, 36);

	assert_eq!(store.with(|state| state.session.messages.len()), 4);
	assert!(text(&frame).contains("This demo runs offline"));
}
