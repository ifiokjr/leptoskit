//! The interface driven headlessly through the terminal host: typing,
//! submitting, the palette, and agent switching, at the sizes opencode's
//! layout changes at.

use leptoskit_tui::Buffer;
use leptoskit_tui::Exit;
use leptoskit_tui::Key;
use leptoskit_tui::KeyCode;
use leptoskit_tui::SourceEvent;
use leptoskit_tui::TuiColor;
use leptoskit_tui::TuiHost;
use leptoskit_tui::TuiOptions;
use leptoskit_tui::testing::Recorder;
use leptoskit_tui::testing::Scripted;
use opencode_example::Agent;
use opencode_example::Screen;
use opencode_example::terminal::TerminalApp;

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

/// Runs a session and returns the app and its last frame.
fn run(events: Vec<SourceEvent>, cols: u16, rows: u16) -> (TerminalApp, Buffer) {
	let mut app = TerminalApp::new("~/code/clockwork:main");
	let mut source = Scripted::new(events);
	let mut sink = Recorder::new(cols, rows);

	let exit = TuiHost::new(TuiOptions::default())
		.run(&mut app, &mut source, &mut sink)
		.expect("session runs");
	assert_eq!(exit, Exit::SourceEnded);

	let frame = sink.frames.pop().expect("a frame was drawn");
	(app, frame)
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
	let (app, frame) = run(Vec::new(), 120, 36);

	assert_eq!(app.state().screen, Screen::Home);
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
	let (app, frame) = run(submitted(), 120, 36);

	assert_eq!(app.state().screen, Screen::Session);
	assert!(app.state().draft.is_empty());
	insta::assert_snapshot!(text(&frame));
}

#[test]
fn wide_sessions_show_the_sidebar() {
	let (_, frame) = run(submitted(), 160, 45);
	insta::assert_snapshot!(text(&frame));
}

#[test]
fn wider_sessions_split_the_diff() {
	let (_, frame) = run(submitted(), 200, 50);
	insta::assert_snapshot!(text(&frame));
}

#[test]
fn the_palette_dims_the_screen_and_takes_focus() {
	let mut events = submitted();
	events.extend([ctrl('p'), key(KeyCode::Down), key(KeyCode::Down)]);
	let (app, frame) = run(events, 160, 45);

	let palette = app.state().palette.as_ref().expect("the palette is open");
	assert_eq!(palette.selected, 2);
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
	let (app, frame) = run(events.clone(), 160, 45);
	assert_eq!(app.state().palette.as_ref().unwrap().query, "model");
	assert!(text(&frame).contains("Switch model"));
	assert!(!text(&frame).contains("Clear session"));

	events.push(key(KeyCode::Escape));
	let (app, frame) = run(events, 160, 45);
	assert!(app.state().palette.is_none());
	assert_eq!(frame.cursor(), Some((5, 39)), "focus returns to the prompt");
}

#[test]
fn new_session_returns_home() {
	let mut events = submitted();
	events.push(ctrl('p'));
	events.extend(typed("new"));
	events.push(key(KeyCode::Enter));
	let (app, _) = run(events, 120, 36);

	assert!(app.state().palette.is_none());
	assert_eq!(app.state().screen, Screen::Home);
}

#[test]
fn shift_tab_switches_agents_and_recolors_the_prompt() {
	let (app, frame) = run(vec![key(KeyCode::BackTab)], 120, 36);

	assert_eq!(app.state().agent, Agent::Plan);
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
	let (app, frame) = run(events, 120, 36);

	assert_eq!(app.state().session.messages.len(), 4);
	assert!(text(&frame).contains("This demo runs offline"));
}
