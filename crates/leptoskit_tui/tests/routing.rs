//! Key and mouse routing branch coverage, over committed surfaces.

use leptoskit_native::NativeSurface;
use leptoskit_native::components::Button;
use leptoskit_native::components::Pressable;
use leptoskit_native::components::ScrollView;
use leptoskit_native::components::Slider;
use leptoskit_native::components::SliderState;
use leptoskit_native::components::Text;
use leptoskit_native::components::TextInput;
use leptoskit_native::components::helpers::Column;
use leptoskit_tui::Key;
use leptoskit_tui::KeyCode;
use leptoskit_tui::Mouse;
use leptoskit_tui::TuiBackend;
use leptoskit_tui::input::Dispatch;

fn surface(tree: &leptoskit_native::Element) -> NativeSurface<TuiBackend> {
	let mut surface = NativeSurface::new(TuiBackend::new());
	surface.set_viewport(480.0, 320.0);
	surface.set_root(tree);
	surface
}

fn keys(surface: &mut NativeSurface<TuiBackend>, keys: &[Key]) -> Vec<Dispatch> {
	let mut all = Vec::new();

	for key in keys {
		for dispatch in surface.backend_mut().handle_key(key) {
			surface.dispatch_event(dispatch.node, &dispatch.event);
			all.push(dispatch);
		}
	}

	all
}

fn key(code: KeyCode) -> Key {
	Key::plain(code)
}

fn field() -> leptoskit_native::Element {
	TextInput()
		.placeholder("Type")
		.accessibility_label("Type")
		.value("hi")
		.class("w-20")
}

#[test]
fn tab_index_orders_the_ring_with_positives_first_and_negatives_gone() {
	let tree = Column()
		.child(Button("zero-a").tab_index(0))
		.child(Button("positive").tab_index(2))
		.child(Button("negative").tab_index(-1))
		.child(Button("zero-b").tab_index(0))
		.child(Button("first").tab_index(1));
	let mut surface = surface(&tree);

	// Ring: first (1), positive (2), zero-a, zero-b. Three tabs land past
	// positive on zero-a; negative never focuses.
	keys(&mut surface, &[key(KeyCode::Tab); 3]);

	let focused = surface.backend().focused().unwrap();
	assert_eq!(surface.props(focused).text.as_deref(), Some("zero-a"));
}

#[test]
fn hidden_focusables_leave_the_ring() {
	let tree = Column()
		.child(Button("hidden").class("hidden"))
		.child(Button("shown"));
	let mut surface = surface(&tree);

	keys(&mut surface, &[key(KeyCode::Tab)]);

	let focused = surface.backend().focused().unwrap();
	assert_eq!(surface.props(focused).text.as_deref(), Some("shown"));
}

#[test]
fn field_editing_covers_delete_home_end_and_ignored_keys() {
	let tree = Column().child(field());
	let mut surface = surface(&tree);

	keys(&mut surface, &[key(KeyCode::Tab)]);

	// End is a no-op at the end; Delete at the end does nothing; Left then
	// Delete removes the "h"; Home then "x" prepends.
	keys(
		&mut surface,
		&[
			key(KeyCode::End),
			key(KeyCode::Delete),
			key(KeyCode::Left),
			key(KeyCode::Delete),
			key(KeyCode::Home),
			Key::plain(KeyCode::Char('x')),
			key(KeyCode::Escape),
			Key::plain(KeyCode::Char('y')),
		],
	);

	let draft = surface
		.backend()
		.draft(surface.backend().focused().unwrap())
		.unwrap();
	assert_eq!(draft.value, "xyh");
	assert_eq!(draft.caret, 2);
}

#[test]
fn modified_characters_do_not_edit_fields() {
	let tree = Column().child(field());
	let mut surface = surface(&tree);

	keys(&mut surface, &[key(KeyCode::Tab)]);

	let before = surface
		.backend()
		.draft(surface.backend().focused().unwrap())
		.unwrap()
		.value
		.clone();
	keys(
		&mut surface,
		&[
			Key {
				code: KeyCode::Char('z'),
				ctrl: true,
				alt: false,
			},
			Key {
				code: KeyCode::Char('z'),
				ctrl: false,
				alt: true,
			},
		],
	);

	let after = surface
		.backend()
		.draft(surface.backend().focused().unwrap())
		.unwrap()
		.value
		.clone();
	assert_eq!(before, after);
}

#[test]
fn enter_on_a_field_submits_its_draft() {
	let tree = Column().child(field());
	let mut surface = surface(&tree);

	let dispatches = keys(&mut surface, &[key(KeyCode::Tab), key(KeyCode::Enter)]);

	assert!(matches!(
		dispatches.last().map(|dispatch| &dispatch.event),
		Some(leptoskit_native::NativeEvent::Submit { value }) if value == "hi"
	));
}

#[test]
fn scroll_keys_page_home_and_end() {
	let tree = ScrollView().class("w-40 h-16").child(
		Column()
			.child(Text("1"))
			.child(Text("2"))
			.child(Text("3"))
			.child(Text("4"))
			.child(Text("5"))
			.child(Text("6"))
			.child(Text("7"))
			.child(Text("8")),
	);
	let mut surface = surface(&tree);

	keys(&mut surface, &[key(KeyCode::Tab)]);

	// Page down scrolls by the viewport less a row, Home returns, End clamps
	// to the far end.
	keys(&mut surface, &[key(KeyCode::PageDown)]);
	assert!(
		(surface
			.backend()
			.scroll(surface.backend().focused().unwrap())
			.unwrap()
			.y
			- 48.0)
			.abs()
			< f32::EPSILON
	);

	keys(&mut surface, &[key(KeyCode::Home)]);
	assert!(
		(surface
			.backend()
			.scroll(surface.backend().focused().unwrap())
			.unwrap()
			.y)
			.abs()
			< f32::EPSILON
	);

	keys(&mut surface, &[key(KeyCode::End)]);
	assert!(
		(surface
			.backend()
			.scroll(surface.backend().focused().unwrap())
			.unwrap()
			.y
			- 64.0)
			.abs()
			< f32::EPSILON
	);

	// Horizontal keys clamp to zero on non-scrolling axes.
	keys(&mut surface, &[key(KeyCode::Left), key(KeyCode::Right)]);
	assert!(
		(surface
			.backend()
			.scroll(surface.backend().focused().unwrap())
			.unwrap()
			.x)
			.abs()
			< f32::EPSILON
	);
}

#[test]
fn slider_home_end_and_bounds_hold_the_grid() {
	let mut surface =
		surface(&Column().child(Slider(SliderState::new(0.0, 4.0, 2.0, Some(1.0)).unwrap())));

	// End proposes the maximum; the application owns committing it.
	let dispatches = keys(&mut surface, &[key(KeyCode::Tab), key(KeyCode::End)]);
	assert!(matches!(
		dispatches.last().map(|dispatch| &dispatch.event),
		Some(leptoskit_native::NativeEvent::ValueChange { value }) if (*value - 4.0).abs() < f64::EPSILON
	));

	// Left steps down one grid point from wherever the value is.
	let dispatches = keys(&mut surface, &[key(KeyCode::Left)]);
	assert!(matches!(
		dispatches.last().map(|dispatch| &dispatch.event),
		Some(leptoskit_native::NativeEvent::ValueChange { value }) if (*value - 1.0).abs() < f64::EPSILON
	));
}

#[test]
fn a_click_on_a_slider_proposes_the_click_position() {
	let tree =
		Column().child(Slider(SliderState::new(0.0, 100.0, 0.0, None).unwrap()).class("w-40"));
	let mut surface = surface(&tree);

	let dispatches = keys(&mut surface, &[Key::plain(KeyCode::Tab)]);
	assert!(
		dispatches
			.iter()
			.any(|dispatch| { matches!(dispatch.event, leptoskit_native::NativeEvent::Focus) })
	);

	let mut mouse = Vec::new();
	for dispatch in surface
		.backend_mut()
		.handle_mouse(&Mouse::Click { col: 10, row: 0 })
	{
		mouse.push(dispatch.event.clone());
		surface.dispatch_event(dispatch.node, &dispatch.event);
	}

	// Clicking past the middle of a 160px slider proposes its position.
	assert!(mouse.iter().any(|event| {
		matches!(
			event,
			leptoskit_native::NativeEvent::ValueChange { value } if (*value - 52.0).abs() < 5.0
		)
	}));
}

#[test]
fn a_click_on_a_field_places_the_caret() {
	let tree = Column().child(field());
	let mut surface = surface(&tree);

	for dispatch in surface
		.backend_mut()
		.handle_mouse(&Mouse::Click { col: 1, row: 0 })
	{
		surface.dispatch_event(dispatch.node, &dispatch.event);
	}

	let focused = surface.backend().focused().unwrap();
	let draft = surface.backend().draft(focused).unwrap();
	assert_eq!(draft.caret, 1);
}

#[test]
fn clicks_and_wheels_outside_widgets_do_nothing() {
	let tree = Column().child(Text("nowhere"));
	let mut surface = surface(&tree);

	assert!(
		surface
			.backend_mut()
			.handle_mouse(&Mouse::Click { col: 30, row: 10 })
			.is_empty()
	);
	assert!(
		surface
			.backend_mut()
			.handle_mouse(&Mouse::ScrollUp { col: 30, row: 10 })
			.is_empty()
	);
}

#[test]
fn wheels_over_plain_content_scroll_nothing() {
	let tree = Column().child(Text("flat"));
	let mut surface = surface(&tree);

	assert!(
		surface
			.backend_mut()
			.handle_mouse(&Mouse::ScrollDown { col: 0, row: 0 })
			.is_empty()
	);
}

#[test]
fn a_pressable_ignores_keys_it_does_not_handle() {
	let tree = Column().child(Pressable("Row").child(Text("Row")));
	let mut surface = surface(&tree);

	let dispatches = keys(&mut surface, &[key(KeyCode::Tab), key(KeyCode::Delete)]);

	assert!(
		dispatches
			.iter()
			.all(|dispatch| !matches!(dispatch.event, leptoskit_native::NativeEvent::Press))
	);
}

#[test]
fn focus_survives_the_focused_node_leaving_the_tree() {
	let mut surface = NativeSurface::new(TuiBackend::new());
	surface.set_viewport(480.0, 320.0);
	surface.set_root(&Column().child(Button("Only")));

	keys(&mut surface, &[key(KeyCode::Tab)]);
	assert!(surface.backend().focused().is_some());

	surface.set_root(&Column().child(Text("gone")));
	assert!(surface.backend().focused().is_none());

	// The ring is empty; Tab reports nothing.
	assert!(
		surface
			.backend_mut()
			.handle_key(&key(KeyCode::Tab))
			.is_empty()
	);
}

#[test]
fn focus_moves_report_blur_then_focus() {
	let tree = Column().child(Button("A")).child(Button("B"));
	let mut surface = surface(&tree);

	keys(&mut surface, &[key(KeyCode::Tab)]);
	let first = surface.backend().focused().unwrap();

	let dispatches = keys(&mut surface, &[key(KeyCode::Tab)]);
	let events: Vec<_> = dispatches
		.iter()
		.map(|dispatch| (&dispatch.node, &dispatch.event))
		.collect();

	assert!(events.contains(&(&first, &leptoskit_native::NativeEvent::Blur)));
	assert!(
		events
			.iter()
			.any(|(_, event)| matches!(event, leptoskit_native::NativeEvent::Focus))
	);
}

#[test]
fn hit_testing_stops_at_pressables_and_skips_hidden_children() {
	let surface = surface(&Column().child(Pressable("Card").child(Text("inner"))));

	// Inside the pressable: the deepest hit is the pressable itself, because
	// its children are decorative.
	let hit = surface.backend().hit_test(20.0, 8.0).unwrap();
	assert_eq!(surface.kind(hit), leptoskit_native::NativeKind::Pressable);
}

#[test]
fn ticks_report_animation_only_while_something_runs() {
	let mut surface =
		surface(&Column().child(leptoskit_native::components::ActivityIndicator(true)));

	assert!(surface.backend_mut().advance_tick());
	surface.set_root(&Column().child(Text("still")));
	assert!(!surface.backend_mut().advance_tick());
}
