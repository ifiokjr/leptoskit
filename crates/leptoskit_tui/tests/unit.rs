//! Unit coverage: wrapping, measurement, color depths, and escape streaming.

use std::borrow::Cow;

use leptoskit_native::TextMeasurer;
use leptoskit_native::TextStyle;
use leptoskit_tui::Attributes;
use leptoskit_tui::Buffer;
use leptoskit_tui::Cell;
use leptoskit_tui::ColorDepth;
use leptoskit_tui::Key;
use leptoskit_tui::KeyCode;
use leptoskit_tui::Rect;
use leptoskit_tui::TuiColor;
use leptoskit_tui::TuiTextMeasurer;
use leptoskit_tui::color::encode;
use leptoskit_tui::color::nearest_16;
use leptoskit_tui::color::nearest_256;
use leptoskit_tui::diff::diff;
use leptoskit_tui::display_width;
use leptoskit_tui::text::BUTTON_CHROME_CELLS;
use leptoskit_tui::text::CELL_HEIGHT;
use leptoskit_tui::text::CELL_WIDTH;
use leptoskit_tui::wrap;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Size;

fn wrapped(text: &str, cols: usize) -> Vec<String> {
	wrap(text, cols)
		.into_iter()
		.map(|line| {
			match line {
				Cow::Borrowed(borrowed) => borrowed.to_owned(),
				Cow::Owned(owned) => owned,
			}
		})
		.collect()
}

#[test]
fn wrapping_keeps_words_whole_until_they_exceed_a_line() {
	assert_eq!(wrapped("aa bb cc", 5), ["aa bb", "cc"]);
	assert_eq!(wrapped("aaaa bbb", 3), ["aaa", "a", "bbb"]);
	assert_eq!(
		wrapped("  leading and trailing  ", 40),
		["leading and trailing"]
	);
}

#[test]
fn wrapping_honors_every_newline_including_empty_ones() {
	assert_eq!(wrapped("a\n\nb", 10), ["a", "", "b"]);
	assert_eq!(wrapped("", 10), [""]);
	assert_eq!(wrapped("\n", 10), ["", ""]);
}

#[test]
fn wrapping_counts_wide_glyphs_as_two_cells() {
	assert_eq!(wrapped("日本 abc", 5), ["日本", "abc"]);
	assert_eq!(display_width("日本"), 4);
}

#[test]
fn a_zero_column_budget_wraps_at_one_column() {
	assert_eq!(wrapped("abc", 0), ["a", "b", "c"]);
}

#[test]
fn measurement_reports_whole_cells() {
	let style = TextStyle::default();
	let measurer = TuiTextMeasurer;

	let free = measurer.measure(
		"hi",
		style,
		Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
	);
	assert_eq!(free, Size::new(2.0 * CELL_WIDTH, CELL_HEIGHT));

	let bounded = measurer.measure(
		"aa bb cc",
		style,
		Size::new(
			AvailableSpace::Definite(5.0 * CELL_WIDTH),
			AvailableSpace::MaxContent,
		),
	);
	assert_eq!(bounded, Size::new(5.0 * CELL_WIDTH, 2.0 * CELL_HEIGHT));

	let multiline = measurer.measure(
		"a\nb",
		style,
		Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
	);
	assert_eq!(multiline, Size::new(CELL_WIDTH, 2.0 * CELL_HEIGHT));
}

#[test]
fn button_measurement_includes_bracket_chrome_only_when_native() {
	let style = TextStyle::default();
	let measurer = TuiTextMeasurer;

	let native = measurer.measure_button(
		"Save",
		style,
		Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
		true,
	);
	assert_eq!(
		native,
		Size::new((4 + BUTTON_CHROME_CELLS) as f32 * CELL_WIDTH, CELL_HEIGHT)
	);

	let custom = measurer.measure_button(
		"Save",
		style,
		Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
		false,
	);
	assert_eq!(custom, Size::new(4.0 * CELL_WIDTH, CELL_HEIGHT));
}

#[test]
fn color_depth_detection_follows_the_environment() {
	use std::borrow::Cow;

	fn borrowed(value: &str) -> Cow<'_, str> {
		Cow::Borrowed(value)
	}

	assert_eq!(
		ColorDepth::detect_from(Some(borrowed("xterm-256color")), None, true),
		ColorDepth::Mono
	);
	assert_eq!(
		ColorDepth::detect_from(
			Some(borrowed("xterm-256color")),
			Some(borrowed("truecolor")),
			false
		),
		ColorDepth::TrueColor
	);
	assert_eq!(
		ColorDepth::detect_from(
			Some(borrowed("xterm-256color")),
			Some(borrowed("24bit")),
			false
		),
		ColorDepth::TrueColor
	);
	assert_eq!(
		ColorDepth::detect_from(Some(borrowed("xterm-256color")), None, false),
		ColorDepth::Indexed256
	);
	assert_eq!(
		ColorDepth::detect_from(Some(borrowed("ansi")), None, false),
		ColorDepth::Indexed16
	);
	assert_eq!(
		ColorDepth::detect_from(Some(borrowed("vt100")), None, false),
		ColorDepth::Mono
	);
}

#[test]
fn quantizing_to_256_hits_exact_palette_entries() {
	// Pure primaries are cube corners.
	assert_eq!(nearest_256(0, 0, 0), 16);
	assert_eq!(nearest_256(255, 0, 0), 196);
	assert_eq!(nearest_256(255, 255, 255), 231);
	// Mid gray lands on the gray ramp.
	assert_eq!(nearest_256(128, 128, 128), 244);
}

#[test]
fn quantizing_to_16_keeps_hue_and_brightness_classes() {
	assert_eq!(nearest_16(128, 0, 0), 1); // red
	assert_eq!(nearest_16(128, 128, 0), 3); // yellow
	assert_eq!(nearest_16(0, 0, 128), 4); // blue
	assert_eq!(nearest_16(0, 128, 128), 6); // cyan
	assert_eq!(nearest_16(255, 0, 0), 9); // bright red
	assert_eq!(nearest_16(0, 0, 0), 0); // black
	assert_eq!(nearest_16(255, 255, 255), 15); // bright white
	assert_eq!(nearest_16(128, 128, 128), 8); // dark gray
}

#[test]
fn encoding_truecolor_and_mono() {
	let depth = ColorDepth::TrueColor;
	assert_eq!(
		encode(TuiColor::Rgb(1, 2, 3), depth),
		leptoskit_tui::color::TerminalColor::TrueColor(1, 2, 3)
	);
	assert_eq!(
		encode(TuiColor::Default, ColorDepth::Mono),
		leptoskit_tui::color::TerminalColor::Default
	);
}

#[test]
fn the_differ_streams_only_changed_runs() {
	let before = Buffer::new(4, 1);
	let mut after = Buffer::new(4, 1);

	after.set_glyph(
		1,
		0,
		'x',
		TuiColor::Default,
		TuiColor::Default,
		Attributes::EMPTY,
	);

	let mut out = Vec::new();
	assert!(diff(&before, &after, ColorDepth::Mono, &mut out).unwrap());
	let text = String::from_utf8(out).unwrap();
	assert!(
		text.contains("\x1b[1;2H"),
		"moves once to the changed cell: {text:?}"
	);
	assert!(text.contains('x'));
	assert!(text.ends_with("\x1b[0m"));

	let mut out = Vec::new();
	assert!(!diff(&after, &after, ColorDepth::Mono, &mut out).unwrap());
	assert!(out.is_empty());
}

#[test]
fn the_differ_groups_runs_of_one_style_and_splits_on_changes() {
	let before = Buffer::new(6, 1);
	let mut after = Buffer::new(6, 1);
	after.set_glyph(
		0,
		0,
		'a',
		TuiColor::Default,
		TuiColor::Default,
		Attributes::BOLD,
	);
	after.set_glyph(
		1,
		0,
		'b',
		TuiColor::Default,
		TuiColor::Default,
		Attributes::BOLD,
	);
	after.set_glyph(
		3,
		0,
		'c',
		TuiColor::Default,
		TuiColor::Default,
		Attributes::EMPTY,
	);

	let mut out = Vec::new();
	assert!(diff(&before, &after, ColorDepth::Mono, &mut out).unwrap());
	let text = String::from_utf8(out).unwrap();
	assert!(
		text.contains("ab"),
		"bold neighbors share one run: {text:?}"
	);
	assert!(
		text.contains("\x1b[1;4H"),
		"the style change moves the cursor: {text:?}"
	);
}

#[test]
fn a_resize_repaints_everything() {
	let small = Buffer::new(2, 1);
	let large = Buffer::new(4, 1);

	let mut out = Vec::new();
	assert!(diff(&small, &large, ColorDepth::Mono, &mut out).unwrap());
	let text = String::from_utf8(out).unwrap();
	assert!(text.starts_with("\x1b[2J"));
}

#[test]
fn wide_glyphs_write_once_and_advance_two_columns() {
	let mut buffer = Buffer::new(4, 1);
	buffer.set_glyph(
		0,
		0,
		'日',
		TuiColor::Default,
		TuiColor::Default,
		Attributes::EMPTY,
	);

	assert_eq!(buffer.cell(0, 0).unwrap().symbol, '日');
	assert!(buffer.cell(1, 0).unwrap().wide);

	let mut out = Vec::new();
	let blank = Buffer::new(4, 1);
	assert!(diff(&blank, &buffer, ColorDepth::Mono, &mut out).unwrap());
	let text = String::from_utf8(out).unwrap();
	assert_eq!(text.matches('日').count(), 1);
}

#[test]
fn rects_intersect_and_contain() {
	let a = Rect::new(0, 0, 4, 4);
	let b = Rect::new(2, 2, 4, 4);
	assert_eq!(a.intersect(b), Rect::new(2, 2, 2, 2));
	assert!(a.intersect(Rect::new(8, 8, 2, 2)).is_empty());
	assert!(a.contains(3, 3));
	assert!(!a.contains(4, 4));
}

#[test]
fn buffer_fill_is_clipped_to_bounds() {
	let mut buffer = Buffer::new(2, 1);
	buffer.fill(Rect::new(-2, -2, 10, 10), TuiColor::Rgb(1, 1, 1));

	for col in 0..2 {
		assert_eq!(buffer.cell(col, 0).unwrap().bg, TuiColor::Rgb(1, 1, 1));
	}
}

#[test]
fn styled_cells_carry_their_colors() {
	let cell = Cell::styled(
		'x',
		TuiColor::Rgb(1, 2, 3),
		TuiColor::Default,
		Attributes::BOLD,
	);
	assert_eq!(cell.symbol, 'x');
	assert!(cell.attrs.bold());
}

#[test]
fn crossterm_keys_map_to_routed_keys() {
	use crossterm::event::KeyCode as CKeyCode;
	use crossterm::event::KeyModifiers;

	let enter = crossterm::event::KeyEvent::new(CKeyCode::Enter, KeyModifiers::NONE);
	assert_eq!(Key::from(enter).code, KeyCode::Enter);

	let tab = crossterm::event::KeyEvent::new(CKeyCode::Tab, KeyModifiers::SHIFT);
	assert_eq!(Key::from(tab).code, KeyCode::BackTab);

	let ctrl_c = crossterm::event::KeyEvent::new(CKeyCode::Char('c'), KeyModifiers::CONTROL);
	let key = Key::from(ctrl_c);
	assert_eq!(key.code, KeyCode::Char('c'));
	assert!(key.ctrl);

	let function = crossterm::event::KeyEvent::new(CKeyCode::F(1), KeyModifiers::NONE);
	assert_eq!(Key::from(function).code, KeyCode::Other);
}

#[test]
fn transparent_packed_colors_become_terminal_defaults() {
	let transparent = leptoskit_native::Color::rgba(10, 20, 30, 127);
	assert_eq!(TuiColor::from_packed(transparent), TuiColor::Default);

	let opaque = leptoskit_native::Color::rgba(10, 20, 30, 128);
	assert_eq!(TuiColor::from_packed(opaque), TuiColor::Rgb(10, 20, 30));
}

#[test]
fn attributes_combine_through_unions() {
	let combined = Attributes::BOLD.union(Attributes::DIM);
	assert!(combined.bold() && combined.dim() && !combined.reverse());
	assert_eq!(
		Attributes::for_weight(leptoskit_native::FontWeight::Bold),
		Attributes::BOLD
	);
	assert_eq!(
		Attributes::for_weight(leptoskit_native::FontWeight::Regular),
		Attributes::EMPTY
	);
}

#[test]
fn viewport_scales_cells_to_pixels() {
	assert_eq!(leptoskit_tui::viewport(80, 24), (640.0, 384.0));
}

#[test]
fn terminal_setup_and_restore_sequences_round_trip() {
	use leptoskit_tui::TuiOptions;
	use leptoskit_tui::terminal::restore_sequence;
	use leptoskit_tui::terminal::setup_sequence;

	let full = TuiOptions {
		title: "Dashboard".into(),
		..TuiOptions::default()
	};
	let setup = setup_sequence(&full);
	let setup = String::from_utf8(setup).unwrap();
	assert!(setup.contains("\x1b[?1049h"), "enters the alternate screen");
	assert!(setup.contains("\x1b[?25l"), "hides the cursor");
	assert!(setup.contains("Dashboard"), "sets the title");

	// Windows captures the mouse through the console API, not sequences.
	#[cfg(unix)]
	assert!(setup.contains("\x1b[?1000h"), "captures the mouse");

	let restore = String::from_utf8(restore_sequence(true, true)).unwrap();
	assert!(restore.contains("\x1b[?25h"), "shows the cursor");
	assert!(
		restore.contains("\x1b[?1049l"),
		"leaves the alternate screen"
	);

	#[cfg(unix)]
	assert!(restore.contains("\x1b[?1000l"), "releases the mouse");

	let bare = setup_sequence(&TuiOptions {
		alternate_screen: false,
		mouse: false,
		..TuiOptions::default()
	});
	assert_eq!(bare, b"\x1b[?25l");
	assert_eq!(restore_sequence(false, false), b"\x1b[?25h");
}

#[test]
fn crossterm_events_translate_and_drop_what_is_not_routed() {
	use crossterm::event::Event;
	use crossterm::event::KeyEvent;
	use crossterm::event::KeyEventKind;
	use crossterm::event::KeyModifiers;
	use crossterm::event::MouseButton;
	use crossterm::event::MouseEvent;
	use crossterm::event::MouseEventKind;
	use leptoskit_tui::terminal::translate_event;
	use leptoskit_tui::terminal::translate_mouse;

	let press = KeyEvent::new_with_kind(
		crossterm::event::KeyCode::Char('a'),
		KeyModifiers::NONE,
		KeyEventKind::Press,
	);
	assert!(matches!(
		translate_event(Event::Key(press)),
		Some(leptoskit_tui::SourceEvent::Key(key)) if key.code == KeyCode::Char('a')
	));

	let click = MouseEvent {
		kind: MouseEventKind::Down(MouseButton::Left),
		column: 3,
		row: 4,
		modifiers: KeyModifiers::NONE,
	};
	assert_eq!(
		translate_event(Event::Mouse(click)),
		Some(leptoskit_tui::SourceEvent::Mouse(
			leptoskit_tui::Mouse::Click { col: 3, row: 4 }
		))
	);

	assert_eq!(
		translate_event(Event::Resize(80, 24)),
		Some(leptoskit_tui::SourceEvent::Resize(80, 24))
	);

	// Focus changes and non-primary buttons never reach the backend.
	assert_eq!(translate_event(Event::FocusGained), None);
	assert_eq!(
		translate_mouse(MouseEventKind::Down(MouseButton::Right), 0, 0),
		None
	);
	assert_eq!(
		translate_mouse(MouseEventKind::Drag(MouseButton::Left), 0, 0),
		None
	);
	assert_eq!(
		translate_mouse(MouseEventKind::ScrollUp, 1, 1),
		Some(leptoskit_tui::Mouse::ScrollUp { col: 1, row: 1 })
	);
	assert_eq!(translate_mouse(MouseEventKind::Moved, 0, 0), None);
}

#[test]
fn a_source_that_ends_after_poll_stops_the_host() {
	use std::time::Duration;

	use leptoskit_tui::EventSource;
	use leptoskit_tui::Exit;
	use leptoskit_tui::TuiHost;
	use leptoskit_tui::TuiOptions;
	use leptoskit_tui::host::SourceEvent;
	use leptoskit_tui::testing::Recorder;
	use leptoskit_tui::testing::surface_app;

	/// Polls ready once, then reads nothing: the host must stop.
	struct Ending;

	impl EventSource for Ending {
		fn poll(&mut self, _timeout: Duration) -> Result<bool, leptoskit_tui::TuiError> {
			Ok(true)
		}

		fn read(&mut self) -> Result<Option<SourceEvent>, leptoskit_tui::TuiError> {
			Ok(None)
		}
	}

	let mut surface = leptoskit_native::NativeSurface::new(leptoskit_tui::TuiBackend::new());
	let mut app = surface_app(&mut surface, || leptoskit_native::Element::text("x"));
	let mut sink = Recorder::new(8, 2);

	let mut host = TuiHost::new(TuiOptions::default());
	assert_eq!(host.options().tick, Duration::from_millis(80));
	let exit = host.run(&mut *app, &mut Ending, &mut sink).unwrap();
	assert_eq!(exit, Exit::SourceEnded);
}

#[test]
fn buffers_expose_their_text_and_pictures() {
	let mut buffer = Buffer::new(3, 2);
	buffer.set_glyph(
		0,
		0,
		'a',
		TuiColor::Default,
		TuiColor::Default,
		Attributes::EMPTY,
	);

	assert_eq!(buffer.to_plain_text(), "a  \n   \n");
	assert!(buffer.same_picture(&buffer.clone()));
	assert_eq!(buffer.cell(3, 0), None);
	assert_eq!(Buffer::new(3, 2).cell_mut(3, 0), None);
}

#[test]
fn hue_sectors_cover_the_wheel() {
	assert_eq!(nearest_16(0, 255, 0), 10); // bright green
	assert_eq!(nearest_16(255, 0, 255), 13); // bright magenta
	assert_eq!(nearest_16(64, 32, 32), 1); // dark red
}

#[test]
fn the_differ_emits_truecolor_for_set_colors_and_defaults_for_unset() {
	let mut buffer = Buffer::new(2, 1);
	buffer.set_glyph(
		0,
		0,
		'r',
		TuiColor::Rgb(200, 10, 10),
		TuiColor::Default,
		Attributes::EMPTY,
	);

	let mut out = Vec::new();
	let blank = Buffer::new(2, 1);
	assert!(diff(&blank, &buffer, ColorDepth::TrueColor, &mut out).unwrap());
	let text = String::from_utf8(out).unwrap();
	assert!(text.contains("38;2;200;10;10"), "{text:?}");
	assert!(text.contains(";49m"), "the default background: {text:?}");
}

#[test]
fn wide_runs_write_continuations_without_extra_moves() {
	let mut buffer = Buffer::new(3, 1);
	buffer.set_glyph(
		0,
		0,
		'あ',
		TuiColor::Default,
		TuiColor::Default,
		Attributes::EMPTY,
	);
	buffer.set_glyph(
		2,
		0,
		'b',
		TuiColor::Default,
		TuiColor::Default,
		Attributes::EMPTY,
	);

	let mut out = Vec::new();
	let blank = Buffer::new(3, 1);
	assert!(diff(&blank, &buffer, ColorDepth::Mono, &mut out).unwrap());
	let text = String::from_utf8(out).unwrap();
	assert!(
		text.contains("あb"),
		"the wide glyph and its neighbor share a run: {text:?}"
	);
}

#[test]
fn a_blank_exit_option_lets_ctrl_c_reach_the_app() {
	use leptoskit_tui::Exit;
	use leptoskit_tui::Key;
	use leptoskit_tui::SourceEvent;
	use leptoskit_tui::TuiBackend;
	use leptoskit_tui::TuiHost;
	use leptoskit_tui::TuiOptions;
	use leptoskit_tui::testing::Recorder;
	use leptoskit_tui::testing::Scripted;
	use leptoskit_tui::testing::surface_app;

	let mut surface = leptoskit_native::NativeSurface::new(TuiBackend::new());
	let mut app = surface_app(&mut surface, || leptoskit_native::Element::text("x"));
	let mut source = Scripted::new(vec![
		SourceEvent::Key(Key::controlled(KeyCode::Char('c'))),
		SourceEvent::Key(Key::plain(KeyCode::Escape)),
	]);
	let mut sink = Recorder::new(8, 1);
	let options = TuiOptions {
		exit: None,
		..TuiOptions::default()
	};

	let exit = TuiHost::new(options)
		.run(&mut *app, &mut source, &mut sink)
		.unwrap();
	assert_eq!(exit, Exit::SourceEnded);
}

#[test]
fn backend_accessors_report_missing_nodes_sanely() {
	use leptoskit_tui::TuiBackend;
	use leptoskit_yoga::Yoga;

	let mut backend = TuiBackend::new();
	let mut yoga = Yoga::new();
	let missing = yoga.create_node();

	assert_eq!(backend.root(), None);
	assert!(backend.node(missing).is_none());
	assert_eq!(backend.scroll(missing), None);
	assert_eq!(backend.absolute_frame(missing), None);
	assert_eq!(backend.tick(), 0);
	assert!(!backend.dark_mode());
	assert!(!backend.advance_tick());
	assert_eq!(backend.hit_test(0.0, 0.0), None);
}

#[test]
fn panicking_buffers_reject_zero_dimensions() {
	let result = std::panic::catch_unwind(|| Buffer::new(0, 4));
	assert!(result.is_err());
}

#[test]
fn quantization_hits_cube_and_gray_midpoints() {
	assert_eq!(nearest_256(95, 135, 175), 67);
	assert_eq!(nearest_256(8, 8, 8), 232);
	assert_eq!(nearest_256(238, 238, 238), 255);
	// Red above brightness 0.6 with a magenta lean wraps the hue wheel.
	assert_eq!(nearest_16(255, 0, 100), 13);
}

#[test]
fn the_differ_writes_palette_indices_for_indexed_depths() {
	let mut buffer = Buffer::new(2, 1);
	buffer.set_glyph(
		0,
		0,
		'r',
		TuiColor::Rgb(255, 0, 0),
		TuiColor::Default,
		Attributes::EMPTY,
	);

	let mut out = Vec::new();
	let blank = Buffer::new(2, 1);
	assert!(diff(&blank, &buffer, ColorDepth::Indexed256, &mut out).unwrap());
	let text = String::from_utf8(out).unwrap();
	assert!(text.contains("38;5;"), "{text:?}");
}

#[test]
fn focusability_follows_kind_and_state() {
	use leptoskit_native::NativeKind;
	use leptoskit_native::NativeProps;
	use leptoskit_tui::focus::is_focusable;

	assert!(is_focusable(NativeKind::Button, &NativeProps::default()));
	assert!(!is_focusable(NativeKind::Text, &NativeProps::default()));

	let disabled = NativeProps {
		enabled: Some(false),
		..NativeProps::default()
	};
	assert!(!is_focusable(NativeKind::Button, &disabled));
}
