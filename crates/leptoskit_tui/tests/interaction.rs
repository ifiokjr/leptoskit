//! Whole-session tests: scripted input through the host loop, asserted
//! against recorded frames.

use std::sync::Arc;
use std::sync::Mutex;

use leptoskit_native::Element;
use leptoskit_native::EventOutcome;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeEventSlot;
use leptoskit_native::NativeSurface;
use leptoskit_native::components::Button;
use leptoskit_native::components::Checkbox;
use leptoskit_native::components::Radio;
use leptoskit_native::components::RadioGroup;
use leptoskit_native::components::ScrollView;
use leptoskit_native::components::Slider;
use leptoskit_native::components::SliderState;
use leptoskit_native::components::Text;
use leptoskit_native::components::TextInput;
use leptoskit_native::components::helpers::Column;
use leptoskit_tui::Exit;
use leptoskit_tui::Key;
use leptoskit_tui::KeyCode;
use leptoskit_tui::Mouse;
use leptoskit_tui::SourceEvent;
use leptoskit_tui::TuiBackend;
use leptoskit_tui::TuiHost;
use leptoskit_tui::TuiOptions;
use leptoskit_tui::testing::Recorder;
use leptoskit_tui::testing::Scripted;

fn shared<T>(value: T) -> Arc<Mutex<T>> {
	Arc::new(Mutex::new(value))
}

fn plain(code: KeyCode) -> SourceEvent {
	SourceEvent::Key(Key::plain(code))
}

fn char(key: char) -> SourceEvent {
	SourceEvent::Key(Key::plain(KeyCode::Char(key)))
}

/// Whether the bracketed button label on `row` draws reversed, as the focused
/// control does. A column stretches its buttons across the terminal, so the
/// label sits wherever centring puts it.
fn label_is_focused(frame: &leptoskit_tui::Buffer, row: u16) -> bool {
	let line = &frame.to_plain_lines()[usize::from(row)];
	let column = line.find('[').expect("a button label on the row");
	let column = u16::try_from(line[..column].chars().count()).expect("a terminal column");
	frame.cell(column, row).expect("a cell").attrs.reverse()
}

/// Runs a screen through a scripted session at a terminal size.
fn session(
	screen: impl FnMut() -> Element + 'static,
	events: Vec<SourceEvent>,
	cols: u16,
	rows: u16,
) -> Recorder {
	let mut surface = NativeSurface::new(TuiBackend::new());
	let mut app = leptoskit_tui::testing::surface_app(&mut surface, screen);
	let mut source = Scripted::new(events);
	let mut sink = Recorder::new(cols, rows);

	let exit = TuiHost::new(TuiOptions::default())
		.run(&mut *app, &mut source, &mut sink)
		.expect("session runs");

	assert_eq!(exit, Exit::SourceEnded);
	sink
}

#[test]
fn tab_moves_focus_through_the_ring() {
	let sink = session(
		|| {
			Column()
				.child(Button("One"))
				.child(Button("Two"))
				.child(Text("plain"))
		},
		vec![plain(KeyCode::Tab), plain(KeyCode::Tab)],
		24,
		4,
	);

	// The text label is not focusable; two tabs land on the second button.
	let frame = sink.frames.last().unwrap();
	assert!(label_is_focused(frame, 1));
	assert!(!label_is_focused(frame, 0));
}

#[test]
fn backtab_from_nothing_lands_on_the_last_focusable() {
	let sink = session(
		|| Column().child(Button("One")).child(Button("Two")),
		vec![plain(KeyCode::BackTab)],
		24,
		4,
	);

	let frame = sink.frames.last().unwrap();
	assert!(label_is_focused(frame, 1));
	assert!(!label_is_focused(frame, 0));
}

#[test]
fn enter_presses_the_focused_button_and_re_renders() {
	let count = shared(0u32);
	let screen = {
		let count = Arc::clone(&count);
		move || {
			let current = *count.lock().unwrap();
			Column().child(Button(format!("Pressed {current}")).on_press({
				let count = Arc::clone(&count);
				move || *count.lock().unwrap() += 1
			}))
		}
	};

	let sink = session(
		screen,
		vec![plain(KeyCode::Tab), plain(KeyCode::Enter)],
		24,
		2,
	);

	assert_eq!(*count.lock().unwrap(), 1);
	assert!(sink.lines()[0].contains("Pressed 1"));
}

#[test]
fn space_toggles_checkboxes() {
	let checked = shared(false);
	let screen = {
		let checked = Arc::clone(&checked);
		move || {
			let state = *checked.lock().unwrap();
			Column()
				.child(Checkbox(state).on_toggle({
					let checked = Arc::clone(&checked);
					move |next: bool| *checked.lock().unwrap() = next
				}))
				.child(Text(if state { "on" } else { "off" }))
		}
	};

	let sink = session(
		screen,
		vec![plain(KeyCode::Tab), plain(KeyCode::Char(' '))],
		16,
		3,
	);

	assert!(*checked.lock().unwrap());
	assert!(sink.lines()[1].contains("on"));
}

#[test]
fn typing_edits_fields_and_enter_submits() {
	let value = shared(String::new());
	let submitted = shared(String::new());

	let screen = {
		let value = Arc::clone(&value);
		let submitted = Arc::clone(&submitted);
		move || {
			let current = value.lock().unwrap().clone();
			let sent = submitted.lock().unwrap().clone();
			Column()
				.child(
					TextInput()
						.placeholder("Name")
						.accessibility_label("Name")
						.value(current)
						.on_change({
							let value = Arc::clone(&value);
							move |next: &str| *value.lock().unwrap() = next.to_owned()
						})
						.on(NativeEventSlot::Submit, {
							let submitted = Arc::clone(&submitted);
							move |event: &NativeEvent| {
								if let NativeEvent::Submit { value } = event {
									*submitted.lock().unwrap() = value.clone();
								}
								EventOutcome::Handled
							}
						})
						.class("w-16"),
				)
				.child(Text(if sent.is_empty() { "-" } else { &sent }))
		}
	};

	let sink = session(
		screen,
		vec![
			plain(KeyCode::Tab),
			char('h'),
			char('i'),
			plain(KeyCode::Backspace),
			char('!'),
			plain(KeyCode::Enter),
		],
		24,
		4,
	);

	assert_eq!(value.lock().unwrap().as_str(), "h!");
	assert_eq!(submitted.lock().unwrap().as_str(), "h!");
	assert!(sink.lines()[0].starts_with("h!"));
}

#[test]
fn caret_moves_within_the_draft() {
	let value = shared("ab".to_owned());
	let screen = {
		let value = Arc::clone(&value);
		move || {
			let current = value.lock().unwrap().clone();
			TextInput()
				.placeholder("Edit")
				.accessibility_label("Edit")
				.value(current)
				.on_change({
					let value = Arc::clone(&value);
					move |next: &str| *value.lock().unwrap() = next.to_owned()
				})
				.class("w-10")
		}
	};

	// Left moves the caret before "b"; "x" lands between them.
	let sink = session(
		screen,
		vec![plain(KeyCode::Tab), plain(KeyCode::Left), char('x')],
		16,
		2,
	);

	assert_eq!(value.lock().unwrap().as_str(), "axb");
	assert!(sink.lines()[0].starts_with("axb"));
}

#[test]
fn an_application_rewrite_replaces_the_draft() {
	let value = shared(String::new());
	let screen = {
		let value = Arc::clone(&value);
		move || {
			// The application uppercases whatever it is given.
			let current = value.lock().unwrap().to_uppercase();
			TextInput()
				.placeholder("Caps")
				.accessibility_label("Caps")
				.value(current)
				.on_change({
					let value = Arc::clone(&value);
					move |next: &str| *value.lock().unwrap() = next.to_owned()
				})
				.class("w-10")
		}
	};

	let sink = session(screen, vec![plain(KeyCode::Tab), char('a')], 16, 2);

	// The application keeps the raw draft but renders it uppercased.
	assert_eq!(value.lock().unwrap().as_str(), "a");
	assert!(sink.lines()[0].starts_with('A'));
}

fn slider_screen(value: &Arc<Mutex<f64>>) -> impl FnMut() -> Element + 'static {
	let value = Arc::clone(value);
	move || {
		let current = *value.lock().unwrap();
		Slider(SliderState::new(0.0, 10.0, current, Some(1.0)).unwrap()).on_value_change({
			let value = Arc::clone(&value);
			move |next: f64| *value.lock().unwrap() = next
		})
	}
}

#[test]
fn arrows_step_sliders_and_home_goes_to_the_minimum() {
	let stepped = shared(0.0f64);
	let sink = session(
		slider_screen(&stepped),
		vec![plain(KeyCode::Tab), plain(KeyCode::Right)],
		24,
		2,
	);
	assert!((*stepped.lock().unwrap() - 1.0).abs() < f64::EPSILON);
	assert_ne!(sink.frames[0], sink.frames[1]);

	let homed = shared(0.0f64);
	let sink = session(
		slider_screen(&homed),
		vec![
			plain(KeyCode::Tab),
			plain(KeyCode::Right),
			plain(KeyCode::Right),
			plain(KeyCode::Home),
		],
		24,
		2,
	);
	assert!((*homed.lock().unwrap() - 0.0).abs() < f64::EPSILON);
	assert!(sink.lines()[0].starts_with("●"));
}

#[test]
fn radio_arrows_walk_the_group_and_enter_selects() {
	let group = RadioGroup::new("speed").unwrap();
	let selected = shared("slow".to_owned());

	fn option(
		group: &RadioGroup,
		name: &'static str,
		checked: bool,
		selected: &Arc<Mutex<String>>,
	) -> Element {
		Radio(group.clone(), checked)
			.accessibility_label(name)
			.on_toggle({
				let selected = Arc::clone(selected);
				move |on: bool| {
					if on {
						*selected.lock().unwrap() = name.to_owned();
					}
				}
			})
	}

	let screen = {
		let selected = Arc::clone(&selected);
		move || {
			let current = selected.lock().unwrap().clone();
			let slow = current == "slow";
			let fast = current == "fast";
			Column()
				.child(option(&group, "slow", slow, &selected))
				.child(option(&group, "fast", fast, &selected))
				.child(Text(current))
		}
	};

	let sink = session(
		screen,
		vec![
			plain(KeyCode::Tab),
			plain(KeyCode::Down),
			plain(KeyCode::Enter),
		],
		16,
		4,
	);

	assert_eq!(selected.lock().unwrap().as_str(), "fast");
	assert!(sink.lines().join("\n").contains("fast"));
}

#[test]
fn clicks_focus_and_press_at_cell_coordinates() {
	let count = shared(0u32);
	let screen = {
		let count = Arc::clone(&count);
		move || {
			Button("Go").on_press({
				let count = Arc::clone(&count);
				move || *count.lock().unwrap() += 1
			})
		}
	};

	let sink = session(
		screen,
		vec![SourceEvent::Mouse(Mouse::Click { col: 2, row: 0 })],
		16,
		2,
	);

	assert_eq!(*count.lock().unwrap(), 1);
	assert!(sink.lines()[0].contains("Go"));
}

#[test]
fn the_wheel_scrolls_the_scrollview_under_the_pointer() {
	let offset = shared((0.0f32, 0.0f32));
	let screen = {
		let offset = Arc::clone(&offset);
		move || {
			ScrollView()
				.class("w-20 h-8")
				.on(NativeEventSlot::Scroll, {
					let offset = Arc::clone(&offset);
					move |event: &NativeEvent| {
						if let NativeEvent::Scroll { offset_x, offset_y } = event {
							*offset.lock().unwrap() = (*offset_x, *offset_y);
						}
						EventOutcome::Handled
					}
				})
				.child(
					Column()
						.child(Text("one"))
						.child(Text("two"))
						.child(Text("three"))
						.child(Text("four")),
				)
		}
	};

	let sink = session(
		screen,
		vec![SourceEvent::Mouse(Mouse::ScrollDown { col: 1, row: 0 })],
		20,
		3,
	);

	// Three rows of wheel travel clamp to the content's one hidden row.
	assert!((offset.lock().unwrap().1 - 32.0).abs() < f32::EPSILON);
	assert!(sink.lines()[0].trim_end().contains("three"));
}

#[test]
fn the_exit_key_stops_the_loop() {
	let mut surface = NativeSurface::new(TuiBackend::new());
	let mut app = leptoskit_tui::testing::surface_app(&mut surface, || Text("bye"));
	let mut source = Scripted::new(vec![SourceEvent::Key(Key::controlled(KeyCode::Char('c')))]);
	let mut sink = Recorder::new(20, 2);

	let exit = TuiHost::new(TuiOptions::default())
		.run(&mut *app, &mut source, &mut sink)
		.unwrap();

	assert_eq!(exit, Exit::ExitKey);
	assert_eq!(sink.frames.len(), 1);
}

#[test]
fn resize_relayouts_the_committed_tree() {
	let sink = session(
		|| Text("0123456789"),
		vec![SourceEvent::Resize(4, 1)],
		10,
		1,
	);

	// The 10-character label wraps into the 4-column terminal.
	assert_eq!(sink.lines()[0], "0123");
}

#[test]
fn disabled_buttons_neither_focus_nor_press() {
	let count = shared(0u32);
	let screen = {
		let count = Arc::clone(&count);
		move || {
			Button("Dead")
				.on_press({
					let count = Arc::clone(&count);
					move || *count.lock().unwrap() += 1
				})
				.with_props(|props| props.enabled = Some(false))
		}
	};

	let sink = session(
		screen,
		vec![plain(KeyCode::Tab), plain(KeyCode::Enter)],
		16,
		2,
	);

	assert_eq!(*count.lock().unwrap(), 0);
	let frame = sink.frames.last().unwrap();
	assert!(!frame.cell(1, 0).unwrap().attrs.reverse());
}

#[test]
fn screens_without_focusables_absorb_keys_quietly() {
	let sink = session(
		|| Column().child(Text("still")),
		vec![plain(KeyCode::Tab), plain(KeyCode::Enter), char('x')],
		16,
		2,
	);

	assert!(sink.lines()[0].contains("still"));
}
