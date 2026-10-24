//! Behavior tests for the headless components, over the real headless
//! renderer: press dispatch, reconciliation, validation, and semantics.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_aria::Dialog;
use leptoskit_aria::DialogTitle;
use leptoskit_aria::DisclosureButton;
use leptoskit_aria::DisclosurePanel;
use leptoskit_aria::Field;
use leptoskit_aria::Tab;
use leptoskit_aria::TabList;
use leptoskit_aria::TabPanel;
use leptoskit_aria::ToggleButton;
use leptoskit_native::Role;
use leptoskit_native::Tristate;
use leptoskit_native::components::Text;
use leptoskit_native::components::TextInput;
use leptoskit_native::components::View;
use leptoskit_testing::Finder;
use leptoskit_testing::WidgetTester;

fn find_button() -> Finder {
	Finder::aria_role(Role::Button, "Frequently asked questions")
}

#[test]
fn disclosure_toggles_expanded_and_panel_visibility() -> Result<(), leptoskit_testing::TestError> {
	let open = Arc::new(AtomicBool::new(false));
	let mut tester = WidgetTester::new(
		{
			let open = Arc::clone(&open);
			move || {
				let state = Arc::clone(&open);
				View()
					.child(DisclosureButton(
						"faq",
						"Frequently asked questions",
						open.load(Ordering::Relaxed),
						move |next| state.store(next, Ordering::Relaxed),
					))
					.child(
						DisclosurePanel("faq", open.load(Ordering::Relaxed))
							.child(Text("Only visible once expanded")),
					)
			}
		},
		390.0,
		844.0,
	)?;

	let button = tester.find(&find_button())?;
	assert_eq!(button.props.aria().expanded, Some(false));
	assert!(
		tester
			.find(&Finder::text("Only visible once expanded"))
			.is_err()
	);

	tester.tap(&find_button())?;

	let button = tester.find(&find_button())?;
	assert_eq!(button.props.aria().expanded, Some(true));
	tester.find(&Finder::text("Only visible once expanded"))?;

	tester.tap(&find_button())?;
	let button = tester.find(&find_button())?;
	assert_eq!(button.props.aria().expanded, Some(false));
	Ok(())
}

#[test]
fn disclosure_tree_without_panel_fails_validation() {
	let tree = View().child(DisclosureButton("faq", "FAQ", false, |_| ()));
	assert!(tree.validate().is_err(), "aria-controls needs its panel");
}

fn tristate_of(value: usize) -> Tristate {
	match value {
		0 => Tristate::False,
		1 => Tristate::True,
		_ => Tristate::Mixed,
	}
}

#[test]
fn toggle_button_walks_pressed_states() -> Result<(), leptoskit_testing::TestError> {
	// 0 off, 1 on, 2 mixed: the walk starts from mixed.
	let pressed = Arc::new(AtomicUsize::new(2));
	let mut tester = WidgetTester::new(
		{
			let pressed = Arc::clone(&pressed);
			move || {
				let state = Arc::clone(&pressed);
				ToggleButton(
					"Mute",
					tristate_of(pressed.load(Ordering::Relaxed)),
					move |next| state.store(usize::from(next), Ordering::Relaxed),
				)
			}
		},
		390.0,
		844.0,
	)?;

	// Mixed proposes true, pressed proposes false, unpressed proposes true.
	for expected in [Tristate::True, Tristate::False, Tristate::True] {
		tester.tap(&Finder::aria_role(Role::Button, "Mute"))?;
		let button = tester.find(&Finder::aria_role(Role::Button, "Mute"))?;
		assert_eq!(button.props.aria().pressed, Some(expected));
	}
	Ok(())
}

#[test]
fn tabs_rove_focus_and_switch_panels() -> Result<(), leptoskit_testing::TestError> {
	const LABELS: [&str; 3] = ["General", "Notifications", "Privacy"];
	let selected = Arc::new(AtomicUsize::new(0));

	let mut tester = WidgetTester::new(
		{
			let selected = Arc::clone(&selected);
			move || {
				let current = selected.load(Ordering::Relaxed);
				let list = (0..LABELS.len()).fold(TabList(), |list, index| {
					let state = Arc::clone(&selected);
					list.child(Tab(
						"settings",
						index,
						LABELS.len(),
						index == current,
						LABELS[index],
						move |index| state.store(index, Ordering::Relaxed),
					))
				});
				View()
					.child(list)
					.child(View().children((0..LABELS.len()).map(|index| {
						TabPanel("settings", index, index == current)
							.child(Text(format!("{} panel", LABELS[index])))
					})))
			}
		},
		390.0,
		844.0,
	)?;

	let first = tester.find(&Finder::aria_role(Role::Tab, "General"))?;
	assert_eq!(first.props.aria().selected, Some(true));
	assert_eq!(first.props.aria().tab_index, Some(0));
	let second = tester.find(&Finder::aria_role(Role::Tab, "Notifications"))?;
	assert_eq!(second.props.aria().selected, Some(false));
	assert_eq!(second.props.aria().tab_index, Some(-1));
	tester.find(&Finder::text("General panel"))?;
	assert!(tester.find(&Finder::text("Notifications panel")).is_err());

	tester.tap(&Finder::aria_role(Role::Tab, "Notifications"))?;

	let first = tester.find(&Finder::aria_role(Role::Tab, "General"))?;
	assert_eq!(first.props.aria().selected, Some(false));
	assert_eq!(first.props.aria().tab_index, Some(-1));
	let second = tester.find(&Finder::aria_role(Role::Tab, "Notifications"))?;
	assert_eq!(second.props.aria().selected, Some(true));
	assert_eq!(second.props.aria().tab_index, Some(0));
	assert!(tester.find(&Finder::text("General panel")).is_err());
	tester.find(&Finder::text("Notifications panel"))?;
	Ok(())
}

#[test]
fn field_wires_label_help_and_error_into_the_input() {
	let build = |error: Option<&'static str>| {
		let field = Field::new("email", "Email address").help("We never share your address.");
		let field = match error {
			Some(text) => field.error(text),
			None => field,
		};
		View()
			.child(field.label_element())
			.child(field.wire(TextInput()))
			.children(field.help_element())
			.children(field.error_element())
	};

	// Without an error: labelled by the label, described by the help alone,
	// valid, and no error element.
	let screen = build(None);
	assert!(screen.validate().is_ok());
	let input = &screen.children[1];
	assert_eq!(input.props.aria().labelled_by, ["email-label"]);
	assert_eq!(input.props.aria().described_by, ["email-help"]);
	assert_eq!(input.props.aria().invalid, None);
	assert_eq!(screen.children.len(), 3, "no error element when unset");

	// With one: the error joins the description and the value is invalid.
	let screen = build(Some("That address has no @."));
	assert!(screen.validate().is_ok());
	let input = &screen.children[1];
	assert_eq!(
		input.props.aria().described_by,
		["email-help", "email-error"]
	);
	assert_eq!(
		input.props.aria().invalid,
		Some(leptoskit_native::Invalid::True)
	);
	assert_eq!(
		screen.children[3].props.text.as_deref(),
		Some("That address has no @.")
	);
}

#[test]
fn field_without_label_element_fails_validation() {
	let field = Field::new("email", "Email address");
	// The wired input references a label id nothing carries.
	let tree = View().child(field.wire(TextInput()));
	assert!(tree.validate().is_err());
}

#[test]
fn dialog_opens_named_and_modal() -> Result<(), leptoskit_testing::TestError> {
	let open = Arc::new(AtomicBool::new(false));
	let mut tester = WidgetTester::new(
		{
			let open = Arc::clone(&open);
			move || {
				Dialog("discard", open.load(Ordering::Relaxed))
					.child(DialogTitle("discard", 2, "Discard draft?"))
					.child(Text("This cannot be undone."))
			}
		},
		390.0,
		844.0,
	)?;

	assert!(tester.find(&Finder::text("Discard draft?")).is_err());

	open.store(true, Ordering::Relaxed);
	tester.pump();
	tester.find(&Finder::text("Discard draft?"))?;

	// The committed dialog carries role, modal, and its labelling reference.
	let tree = Dialog("discard", true)
		.child(DialogTitle("discard", 2, "Discard draft?"))
		.child(Text("This cannot be undone."));
	assert_eq!(tree.props.aria().role, Some(Role::Dialog));
	assert_eq!(tree.props.aria().modal, Some(true));
	assert_eq!(tree.props.aria().labelled_by, ["discard-title"]);
	assert!(tree.validate().is_ok());
	Ok(())
}

#[test]
fn dialog_without_title_fails_validation() {
	let tree = Dialog("discard", true).child(Text("This cannot be undone."));
	assert!(
		tree.validate().is_err(),
		"a dialog must be named by its title"
	);
}
