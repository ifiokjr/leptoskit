//! Every widget kind painted through the real surface into the cell buffer.

use leptoskit_native::Element;
use leptoskit_native::NativeSurface;
use leptoskit_native::ProgressValue;
use leptoskit_native::components::ActivityIndicator;
use leptoskit_native::components::Button;
use leptoskit_native::components::Checkbox;
use leptoskit_native::components::Image;
use leptoskit_native::components::LinearProgressIndicator;
use leptoskit_native::components::PasswordInput;
use leptoskit_native::components::Pressable;
use leptoskit_native::components::Progress;
use leptoskit_native::components::Radio;
use leptoskit_native::components::RadioGroup;
use leptoskit_native::components::ScrollView;
use leptoskit_native::components::Slider;
use leptoskit_native::components::SliderState;
use leptoskit_native::components::Switch;
use leptoskit_native::components::Text;
use leptoskit_native::components::TextInput;
use leptoskit_native::components::View;
use leptoskit_native::components::helpers::Column;
use leptoskit_native::components::helpers::Row;
use leptoskit_tui::Key;
use leptoskit_tui::KeyCode;
use leptoskit_tui::TuiBackend;
use leptoskit_tui::TuiColor;

/// Renders a tree at a terminal size and returns its rows.
fn lines(tree: &Element, cols: u16, rows: u16) -> Vec<String> {
	surface(tree, cols, rows)
		.backend()
		.render(cols, rows)
		.to_plain_lines()
}

/// Commits a tree at a terminal size.
fn surface(tree: &Element, cols: u16, rows: u16) -> NativeSurface<TuiBackend> {
	let mut surface = NativeSurface::new(TuiBackend::new());
	surface.set_dark_mode(true);
	surface.set_viewport(f32::from(cols) * 8.0, f32::from(rows) * 16.0);
	surface.set_root(tree);
	surface
}

fn first_line(tree: &Element, cols: u16) -> String {
	lines(tree, cols, 3)
		.first()
		.map(|line| line.trim_end().to_owned())
		.unwrap_or_default()
}

fn tab(surface: &mut NativeSurface<TuiBackend>) {
	press(surface, KeyCode::Tab);
}

fn press(surface: &mut NativeSurface<TuiBackend>, code: KeyCode) {
	for dispatch in surface.backend_mut().handle_key(&Key::plain(code)) {
		surface.dispatch_event(dispatch.node, &dispatch.event);
	}
}

#[test]
fn labels_wrap_into_the_measured_rows() {
	let label = Text("the quick brown fox jumps over the lazy dog").class("w-20");
	let surface = surface(&label, 10, 5);

	let id = surface.root().unwrap();
	let frame = surface.frame(id).unwrap();
	let rows = surface.backend().render(10, 5).to_plain_lines();

	// The measured height is whole rows, and the rows come from the same
	// wrap the measurer used.
	assert!((frame.height - 80.0).abs() < f32::EPSILON);
	assert_eq!(
		rows.iter()
			.map(|row| row.trim_end().to_owned())
			.collect::<Vec<_>>(),
		["the quick", "brown fox", "jumps over", "the lazy", "dog"]
	);
}

#[test]
fn a_button_draws_bracket_chrome() {
	let tree = Button("Save");
	assert_eq!(first_line(&tree, 20), "[ Save ]");
}

#[test]
fn a_custom_drawn_button_has_no_chrome() {
	let button = Button("Save").class("bg-blue-500 px-2").on_press(|| {});
	assert_eq!(first_line(&button, 20), " Save");
}

#[test]
fn boolean_controls_draw_their_states() {
	let tree = Column()
		.child(Row().child(Checkbox(true)).child(Text("done")))
		.child(Row().child(Checkbox(false)).child(Text("todo")))
		.child(Row().child(Switch(true)).child(Text("on")))
		.child(Row().child(Switch(false)).child(Text("off")));

	insta::assert_snapshot!(lines(&tree, 12, 5).join("\n"));
}

#[test]
fn radios_draw_their_group_states() {
	let group = RadioGroup::new("size").unwrap();
	let tree = Column()
		.child(Row().child(Radio(group.clone(), true)).child(Text("small")))
		.child(Row().child(Radio(group, false)).child(Text("large")));

	insta::assert_snapshot!(lines(&tree, 12, 3).join("\n"));
}

#[test]
fn a_slider_draws_its_track_and_knob() {
	let state = SliderState::new(0.0, 10.0, 2.0, None).unwrap();
	let row = lines(&Slider(state), 25, 1);

	assert_eq!(row[0].trim(), "────●──────────────────");
}

#[test]
fn progress_bars_fill_to_their_fraction() {
	let half = LinearProgressIndicator(Progress::Determinate(ProgressValue::new(0.5).unwrap()));
	let row = lines(&half, 24, 1);
	assert_eq!(row[0].trim_end(), "█".repeat(12) + &"░".repeat(11));

	let empty = LinearProgressIndicator(Progress::Determinate(ProgressValue::new(0.0).unwrap()));
	let row = lines(&empty, 24, 1);
	assert_eq!(row[0].trim_end(), "░".repeat(23));
}

#[test]
fn an_indeterminate_bar_sweeps_with_the_tick() {
	let tree = LinearProgressIndicator(Progress::Indeterminate);
	let mut surface = surface(&tree, 30, 1);

	let first = surface.backend().render(30, 1).to_plain_lines()[0].clone();
	surface.backend_mut().advance_tick();
	let second = surface.backend().render(30, 1).to_plain_lines()[0].clone();

	assert!(first.contains('█'));
	assert_ne!(first, second);
}

#[test]
fn spinners_advance_and_stop() {
	let tree = Column()
		.child(ActivityIndicator(true))
		.child(ActivityIndicator(false));
	let mut surface = surface(&tree, 30, 2);

	let first = surface.backend().render(30, 2).to_plain_lines();
	surface.backend_mut().advance_tick();
	let second = surface.backend().render(30, 2).to_plain_lines();

	assert_eq!(first[1].trim(), "·");
	assert_eq!(second[1], first[1]);
	assert_ne!(first[0], second[0]);
}

#[test]
fn text_fields_show_values_and_placeholders() {
	let tree = Column()
		.child(
			TextInput()
				.placeholder("Search")
				.accessibility_label("Search")
				.class("w-12"),
		)
		.child(
			TextInput()
				.value("typed")
				.placeholder("Search")
				.accessibility_label("Search")
				.class("w-12"),
		);

	insta::assert_snapshot!(lines(&tree, 16, 4).join("\n"));
}

#[test]
fn password_fields_mask_their_values() {
	let field = PasswordInput()
		.value("secret")
		.placeholder("Password")
		.accessibility_label("Password")
		.class("w-10");

	assert!(first_line(&field, 16).starts_with("••••••"));
}

#[test]
fn images_draw_a_placeholder_texture() {
	let image = Image("asset://logo.png").class("w-8 h-4");
	let row = lines(&image, 12, 2);

	assert!(row[0].starts_with("▒▒▒▒"));
}

#[test]
fn borders_draw_around_views() {
	let framed = View()
		.class("w-24 h-12 border border-slate-400 p-4")
		.child(Text("inside"));

	insta::assert_snapshot!(lines(&framed, 20, 3).join("\n"));
}

#[test]
fn rounded_corners_use_round_glyphs() {
	let framed = View()
		.class("w-8 h-12 border border-slate-400 rounded-lg")
		.child(Text("hi").class("pt-4"));

	let rows = lines(&framed, 10, 3);
	assert!(rows[0].starts_with("╭"));
	assert!(rows[0].contains("╮"));
	assert!(rows[2].starts_with("╰"));
}

#[test]
fn scrolling_clips_and_translates_content() {
	let tree = ScrollView().class("w-20 h-8").child(
		Column()
			.child(Text("one"))
			.child(Text("two"))
			.child(Text("three"))
			.child(Text("four")),
	);
	let mut surface = surface(&tree, 20, 3);

	let before = surface.backend().render(20, 3).to_plain_lines();
	assert_eq!(before[0].trim(), "one");
	assert_eq!(before[1].trim(), "two");

	tab(&mut surface);
	press(&mut surface, KeyCode::Down);

	let after = surface.backend().render(20, 3).to_plain_lines();
	assert_eq!(after[0].trim(), "two");
	assert_eq!(after[1].trim(), "three");
}

#[test]
fn hidden_and_translucent_subtrees_disappear() {
	let tree = Column()
		.child(Text("visible"))
		.child(Text("hidden").class("hidden"))
		.child(Text("ghost").class("opacity-0"));

	let rows = lines(&tree, 20, 3);
	assert_eq!(rows[0].trim(), "visible");
	assert_eq!(rows[1], " ".repeat(20));
}

#[test]
fn colors_and_weights_reach_the_cells() {
	let tree = Text("warning").class("text-red-500 font-bold");
	let buffer = surface(&tree, 20, 1).backend().render(20, 1);

	let cell = buffer.cell(0, 0).unwrap();
	assert_eq!(cell.fg, TuiColor::Rgb(251, 44, 54));
	assert!(cell.attrs.bold());
}

#[test]
fn disabled_controls_paint_dim() {
	let tree = Button("Save").with_props(|props| props.enabled = Some(false));
	let buffer = surface(&tree, 20, 2).backend().render(20, 2);

	assert!(buffer.cell(1, 0).unwrap().attrs.dim());
}

#[test]
fn a_focused_button_reverses() {
	let tree = Button("Save");
	let mut surface = surface(&tree, 20, 2);
	tab(&mut surface);

	let buffer = surface.backend().render(20, 2);
	assert!(buffer.cell(1, 0).unwrap().attrs.reverse());
}

#[test]
fn focus_rings_underline_pressables() {
	let tree = Pressable("Open").child(Text("Open"));
	let mut surface = surface(&tree, 20, 2);
	tab(&mut surface);

	let buffer = surface.backend().render(20, 2);
	assert!(buffer.cell(0, 0).unwrap().attrs.underline());
}

#[test]
fn backgrounds_fill_cells() {
	let tree = View().class("w-8 h-4 bg-blue-600");
	let buffer = surface(&tree, 10, 1).backend().render(10, 1);

	let cell = buffer.cell(0, 0).unwrap();
	assert_eq!(cell.bg, TuiColor::Rgb(21, 93, 252));
	assert_eq!(cell.symbol, ' ');
}

#[test]
fn text_alignment_places_lines() {
	let centered = Text("hi").class("w-20 text-center");
	assert_eq!(first_line(&centered, 10), "    hi");

	let end = Text("hi").class("w-20 text-end");
	assert_eq!(first_line(&end, 10), "        hi");
}

#[test]
fn long_words_break_by_glyph() {
	let wrapped = Text("abcdefghij").class("w-10");
	let rows = lines(&wrapped, 10, 2);
	assert_eq!(
		rows.iter()
			.map(|row| row.trim_end().to_owned())
			.collect::<Vec<_>>()[..2]
			.to_vec(),
		["abcde", "fghij"]
	);
}

#[test]
fn wide_characters_occupy_two_cells() {
	let tree = Text("日本語");
	let buffer = surface(&tree, 20, 1).backend().render(20, 1);

	assert_eq!(buffer.cell(0, 0).unwrap().symbol, '日');
	assert!(buffer.cell(1, 0).unwrap().wide);
	assert_eq!(buffer.to_plain_lines()[0].trim(), "日本語");
}
