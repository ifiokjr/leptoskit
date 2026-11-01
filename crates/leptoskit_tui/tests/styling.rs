//! Styled text runs, per-edge and half-row borders, translucent scrims, the
//! terminal cursor, programmatic focus, and app shortcuts.

use std::sync::Arc;
use std::sync::Mutex;

use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_native::EventOutcome;
use leptoskit_native::KeyPress;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeSurface;
use leptoskit_native::TextSpan;
use leptoskit_native::components::Text;
use leptoskit_native::components::TextInput;
use leptoskit_native::components::View;
use leptoskit_tui::Attributes;
use leptoskit_tui::Buffer;
use leptoskit_tui::Dispatch;
use leptoskit_tui::Exit;
use leptoskit_tui::Key;
use leptoskit_tui::KeyCode;
use leptoskit_tui::SourceEvent;
use leptoskit_tui::TuiBackend;
use leptoskit_tui::TuiColor;
use leptoskit_tui::TuiHost;
use leptoskit_tui::TuiOptions;
use leptoskit_tui::terminal::FrameWriter;
use leptoskit_tui::testing::Recorder;
use leptoskit_tui::testing::Scripted;

fn render(tree: &Element, cols: u16, rows: u16) -> Buffer {
	let mut surface = NativeSurface::new(TuiBackend::new());
	surface.set_viewport(f32::from(cols) * 8.0, f32::from(rows) * 16.0);
	surface.set_root(tree);
	surface.backend().render(cols, rows)
}

fn cell(buffer: &Buffer, col: u16, row: u16) -> leptoskit_tui::Cell {
	*buffer.cell(col, row).expect("cell inside the buffer")
}

const RED: Color = Color::rgba(0xe0, 0x6c, 0x75, 0xff);
const PANEL: TuiColor = TuiColor::Rgb(0x14, 0x14, 0x14);

#[test]
fn spans_paint_their_own_styles_and_wrap_as_one_paragraph() {
	let tree = View().class("w-[48px]").child(Text("").spans([
		TextSpan::new("let ").italic(),
		TextSpan::new("x").color(RED).bold(),
		TextSpan::new(" = 1"),
	]));
	let buffer = render(&tree, 6, 3);

	assert_eq!(buffer.to_plain_lines()[0].trim_end(), "let x");
	assert_eq!(buffer.to_plain_lines()[1].trim_end(), "= 1");
	assert!(cell(&buffer, 0, 0).attrs.italic());
	assert!(!cell(&buffer, 0, 0).attrs.bold());
	let x = cell(&buffer, 4, 0);
	assert_eq!(x.fg, TuiColor::Rgb(0xe0, 0x6c, 0x75));
	assert!(x.attrs.bold() && !x.attrs.italic());
	assert_eq!(cell(&buffer, 0, 1).fg, TuiColor::Default);
}

#[test]
fn text_keeps_the_background_painted_under_it() {
	let tree = View().class("w-full h-full bg-[#141414]").child(Text("hi"));
	let buffer = render(&tree, 4, 1);

	assert_eq!(cell(&buffer, 0, 0).bg, PANEL);
	assert_eq!(cell(&buffer, 0, 0).symbol, 'h');
}

#[test]
fn italic_class_marks_glyphs_italic() {
	let buffer = render(&Text("slanted").class("italic"), 10, 1);
	assert!(cell(&buffer, 0, 0).attrs.italic());
}

#[test]
fn no_break_spaces_print_as_blank_cells() {
	let buffer = render(&Text("\u{a0}\u{a0}x"), 4, 1);
	assert_eq!(buffer.to_plain_lines()[0], "  x ");
}

#[test]
fn a_left_border_draws_one_heavy_edge_that_ends_in_a_half_line() {
	// 16 + 16 + 8 pixels: two whole rows and the top half of a third. The
	// two-pixel border shares its cell with the padding before the panel.
	let tree = View()
		.class("w-[48px] border-l-2 border-[#5c9cf5] pl-1.5")
		.child(View().class("bg-[#1e1e1e] pt-4 pb-2").child(Text("ok")));
	let buffer = render(&tree, 6, 4);

	let lines = buffer.to_plain_lines();
	assert_eq!(lines[0], "┃     ");
	assert_eq!(lines[1], "┃ok   ");
	assert_eq!(lines[2], "╹▀▀▀▀▀");
	assert_eq!(lines[3], "      ");
	assert_eq!(cell(&buffer, 0, 0).fg, TuiColor::Rgb(0x5c, 0x9c, 0xf5));
	// The half row's glyph takes the panel color over what was underneath.
	assert_eq!(cell(&buffer, 1, 2).fg, TuiColor::Rgb(0x1e, 0x1e, 0x1e));
	assert_eq!(cell(&buffer, 1, 2).bg, TuiColor::Default);
}

#[test]
fn half_rows_snap_their_top_to_a_whole_row() {
	// The box starts half a row down, which rounds to the next row like
	// text does, and keeps its one-and-a-half-row height.
	let tree = View()
		.class("w-full h-full")
		.child(View().class("mt-2 h-6 w-full bg-[#141414]"));
	let buffer = render(&tree, 2, 4);

	assert_eq!(buffer.to_plain_lines(), ["  ", "  ", "▀▀", "  "]);
	assert_eq!(cell(&buffer, 0, 1).bg, PANEL);
	assert_eq!(cell(&buffer, 0, 0).bg, TuiColor::Default);
}

#[test]
fn edge_borders_draw_only_their_edges() {
	let tree = View().class("w-[32px] h-[48px] border-t border-b");
	assert_eq!(
		render(&tree, 4, 3).to_plain_lines(),
		["────", "    ", "────"]
	);

	let tree = View().class("w-[32px] h-[48px] border-x-2");
	assert_eq!(
		render(&tree, 4, 3).to_plain_lines(),
		["┃  ┃", "┃  ┃", "┃  ┃"]
	);
}

#[test]
fn heavy_box_borders_use_heavy_corners() {
	let tree = View().class("w-[32px] h-[48px] border-2");
	assert_eq!(
		render(&tree, 4, 3).to_plain_lines(),
		["┏━━┓", "┃  ┃", "┗━━┛"]
	);
}

#[test]
fn a_translucent_background_dims_what_is_behind_it() {
	let tree = View()
		.class("w-full h-full bg-[#eeeeee]")
		.child(Text("ab").class("text-[#808080]"))
		.child(View().class("absolute inset-0 bg-[#00000096]"));
	let buffer = render(&tree, 2, 1);
	let dimmed = cell(&buffer, 0, 0);

	// 0x96 of black over each channel: 238 becomes 98, 128 becomes 53.
	assert_eq!(dimmed.bg, TuiColor::Rgb(98, 98, 98));
	assert_eq!(dimmed.fg, TuiColor::Rgb(53, 53, 53));
	assert_eq!(dimmed.symbol, 'a');
}

#[test]
fn an_opaque_overlay_hides_the_glyphs_behind_it() {
	let tree = View()
		.class("w-full h-full")
		.child(Text("hidden"))
		.child(View().class("absolute inset-0 bg-[#141414]"));
	let buffer = render(&tree, 6, 1);

	assert_eq!(buffer.to_plain_lines()[0], "      ");
	assert_eq!(cell(&buffer, 0, 0).bg, PANEL);
}

#[test]
fn the_buffer_blends_and_half_fills_directly() {
	let mut buffer = Buffer::new(2, 1);
	buffer.fill(
		leptoskit_tui::Rect::new(0, 0, 2, 1),
		TuiColor::Rgb(200, 100, 0),
	);
	buffer.tint(leptoskit_tui::Rect::new(0, 0, 1, 1), (0, 0, 0), 255);
	assert_eq!(cell(&buffer, 0, 0).bg, TuiColor::Rgb(0, 0, 0));
	assert_eq!(cell(&buffer, 1, 0).bg, TuiColor::Rgb(200, 100, 0));

	buffer.half_fill(leptoskit_tui::Rect::new(1, 0, 1, 1), PANEL, false);
	let lower = cell(&buffer, 1, 0);
	assert_eq!(
		(lower.symbol, lower.fg, lower.bg),
		('▄', PANEL, TuiColor::Rgb(200, 100, 0))
	);
	assert_eq!(lower.attrs, Attributes::EMPTY);
}

#[test]
fn a_focused_field_shows_the_terminal_cursor_at_its_caret() {
	let field = |focused| {
		TextInput()
			.aria_label("Prompt")
			.value("abc")
			.class("w-[64px] bg-[#141414]")
			.focused(focused)
	};
	let mut surface = NativeSurface::new(TuiBackend::new());
	surface.set_viewport(80.0, 32.0);
	surface.set_root(&field(false));
	assert_eq!(surface.backend().render(10, 2).cursor(), None);

	// Asking for focus moves it on the next commit, once.
	surface.set_root(&field(true));
	let events = surface.backend_mut().take_events();
	assert_eq!(events.len(), 1);
	assert_eq!(events[0].event, NativeEvent::Focus);
	surface.set_root(&field(true));
	assert_eq!(surface.backend_mut().take_events(), [] as [Dispatch; 0]);

	// Focus lands after the value, and the caret is the cursor, not a
	// reversed cell.
	let buffer = surface.backend().render(10, 2);
	assert_eq!(buffer.cursor(), Some((3, 0)));
	assert!(!cell(&buffer, 3, 0).attrs.reverse());

	surface.backend_mut().handle_key(&Key::plain(KeyCode::Home));
	assert_eq!(surface.backend().render(10, 2).cursor(), Some((0, 0)));
}

#[test]
fn the_last_widget_asking_for_focus_gets_it() {
	let tree = |dialog: bool| {
		let mut root = View().child(TextInput().id("prompt").aria_label("Prompt").focused(true));

		if dialog {
			root = root.child(TextInput().id("search").aria_label("Search").focused(true));
		}

		root
	};
	let focused_id = |surface: &NativeSurface<TuiBackend>| {
		let backend = surface.backend();
		backend
			.focused()
			.and_then(|node| backend.props(node))
			.and_then(|props| props.id.clone())
	};
	let mut surface = NativeSurface::new(TuiBackend::new());
	surface.set_viewport(160.0, 64.0);

	surface.set_root(&tree(false));
	assert_eq!(focused_id(&surface).as_deref(), Some("prompt"));
	surface.set_root(&tree(true));
	assert_eq!(focused_id(&surface).as_deref(), Some("search"));
	// Closing the dialog hands focus back to the widget still asking.
	surface.set_root(&tree(false));
	assert_eq!(focused_id(&surface).as_deref(), Some("prompt"));
}

#[test]
fn the_cursor_stays_inside_the_buffer() {
	let mut buffer = Buffer::new(2, 2);
	buffer.set_cursor(Some((1, 1)));
	assert_eq!(buffer.cursor(), Some((1, 1)));
	buffer.set_cursor(Some((2, 0)));
	assert_eq!(buffer.cursor(), None);
	buffer.set_cursor(Some((0, 0)));
	buffer.clear();
	assert_eq!(buffer.cursor(), None);
}

#[test]
fn frames_move_and_show_the_cursor_after_their_cells() {
	let mut writer = FrameWriter::new(leptoskit_tui::ColorDepth::TrueColor);
	let mut frame = Buffer::new(3, 2);
	frame.set_cursor(Some((2, 1)));

	let mut out = Vec::new();
	assert!(writer.draw(&frame, &mut out).unwrap());
	assert!(
		String::from_utf8(out)
			.unwrap()
			.ends_with("\x1b[2;3H\x1b[?25h")
	);

	// Only the cursor changes: the frame still writes, to hide it.
	let mut hidden = frame.clone();
	hidden.set_cursor(None);
	let mut out = Vec::new();
	assert!(writer.draw(&hidden, &mut out).unwrap());
	assert_eq!(out, b"\x1b[?25l");

	let mut out = Vec::new();
	assert!(!writer.draw(&hidden, &mut out).unwrap());
	assert_eq!(out, [] as [u8; 0]);
}

#[test]
fn cursor_sequences_are_one_based() {
	assert_eq!(
		leptoskit_tui::terminal::cursor_sequence(Some((0, 0))),
		b"\x1b[1;1H\x1b[?25h"
	);
	assert_eq!(leptoskit_tui::terminal::cursor_sequence(None), b"\x1b[?25l");
}

#[test]
fn key_handlers_hear_keys_before_the_focused_field() {
	let palettes = Arc::new(Mutex::new(0_u32));
	let typed = Arc::new(Mutex::new(String::new()));
	let screen = {
		let palettes = Arc::clone(&palettes);
		let typed = Arc::clone(&typed);
		move || {
			let palettes = Arc::clone(&palettes);
			let change = Arc::clone(&typed);
			View()
				.on_key(move |key: &KeyPress| {
					if key.is("p", true, false, false) {
						*palettes.lock().unwrap() += 1;
						EventOutcome::Handled
					} else {
						EventOutcome::Unhandled
					}
				})
				.child(
					TextInput()
						.aria_label("Field")
						.value(typed.lock().unwrap().clone())
						.focused(true)
						.on_change(move |value| value.clone_into(&mut change.lock().unwrap())),
				)
		}
	};
	let mut surface = NativeSurface::new(TuiBackend::new());
	let mut app = leptoskit_tui::testing::surface_app(&mut surface, screen);
	let mut source = Scripted::new([
		SourceEvent::Key(Key::controlled(KeyCode::Char('p'))),
		SourceEvent::Key(Key::plain(KeyCode::Char('p'))),
	]);
	let mut sink = Recorder::new(10, 2);

	let exit = TuiHost::new(TuiOptions::default())
		.run(&mut *app, &mut source, &mut sink)
		.expect("session runs");

	assert_eq!(exit, Exit::SourceEnded);
	assert_eq!(*palettes.lock().unwrap(), 1);
	assert_eq!(typed.lock().unwrap().as_str(), "p");
}

#[test]
fn terminal_keys_are_named_like_the_web() {
	use leptoskit_tui::input::key_press;

	assert_eq!(
		key_press(Key::controlled(KeyCode::Char('p'))),
		Some(KeyPress::new("p").ctrl())
	);
	assert_eq!(
		key_press(Key::plain(KeyCode::Char('P'))),
		Some(KeyPress::new("P").shift())
	);
	assert_eq!(
		key_press(Key::plain(KeyCode::BackTab)),
		Some(KeyPress::new("Tab").shift())
	);
	assert_eq!(
		key_press(Key::plain(KeyCode::Up)),
		Some(KeyPress::new("ArrowUp"))
	);
	assert_eq!(key_press(Key::plain(KeyCode::Other)), None);
}

#[test]
fn runs_that_no_longer_spell_the_text_fall_back_to_plain_text() {
	let tree = Text("")
		.spans([TextSpan::new("old").color(RED)])
		.with_props(|props| props.text = Some("néw text".to_owned()));
	let buffer = render(&tree, 10, 1);

	assert_eq!(buffer.to_plain_lines()[0], "néw text  ");
	assert_eq!(cell(&buffer, 0, 0).fg, TuiColor::Default);
}

#[test]
fn an_upright_run_overrides_an_italic_label() {
	let mut upright = TextSpan::new("b");
	upright.italic = Some(false);
	let tree = Text("")
		.class("italic")
		.spans([TextSpan::new("a"), upright]);
	let buffer = render(&tree, 2, 1);

	assert!(cell(&buffer, 0, 0).attrs.italic());
	assert!(!cell(&buffer, 1, 0).attrs.italic());
}
