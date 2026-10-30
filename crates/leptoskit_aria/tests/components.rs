//! Behavior tests for the headless components, over the real headless
//! renderer: press dispatch, reconciliation, validation, and semantics.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_aria::Dialog;
use leptoskit_aria::DisclosureButton;
use leptoskit_aria::DisclosurePanel;
use leptoskit_aria::Field;
use leptoskit_aria::Tabs;
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
fn tabs_switch_panels_and_stay_in_the_tab_order() -> Result<(), leptoskit_testing::TestError> {
	const LABELS: [&str; 3] = ["General", "Notifications", "Privacy"];
	let selected = Arc::new(AtomicUsize::new(0));

	let mut tester = WidgetTester::new(
		{
			let selected = Arc::clone(&selected);
			move || {
				let tabs = Tabs::new("settings", LABELS.len(), selected.load(Ordering::Relaxed));
				let list = LABELS
					.iter()
					.enumerate()
					.fold(tabs.list(), |list, (index, label)| {
						let state = Arc::clone(&selected);
						list.child(tabs.tab(index, *label, move |index| {
							state.store(index, Ordering::Relaxed);
						}))
					});
				View().child(list).child(View().children(
					LABELS.iter().enumerate().map(|(index, label)| {
						tabs.panel(index).child(Text(format!("{label} panel")))
					}),
				))
			}
		},
		390.0,
		844.0,
	)?;

	let first = tester.find(&Finder::aria_role(Role::Tab, "General"))?;
	assert_eq!(first.props.aria().selected, Some(true));
	let second = tester.find(&Finder::aria_role(Role::Tab, "Notifications"))?;
	assert_eq!(second.props.aria().selected, Some(false));
	assert_eq!(second.props.aria().pos_in_set, Some(2));
	assert_eq!(second.props.aria().set_size, Some(3));
	// Without arrow-key motion a roving tabindex would strand this tab out of
	// keyboard reach, so no tab leaves the sequential focus order.
	assert_eq!(second.props.aria().tab_index, None);
	tester.find(&Finder::text("General panel"))?;
	assert!(tester.find(&Finder::text("Notifications panel")).is_err());

	tester.tap(&Finder::aria_role(Role::Tab, "Notifications"))?;

	let first = tester.find(&Finder::aria_role(Role::Tab, "General"))?;
	assert_eq!(first.props.aria().selected, Some(false));
	assert_eq!(first.props.aria().tab_index, None);
	let second = tester.find(&Finder::aria_role(Role::Tab, "Notifications"))?;
	assert_eq!(second.props.aria().selected, Some(true));
	assert!(tester.find(&Finder::text("General panel")).is_err());
	tester.find(&Finder::text("Notifications panel"))?;
	Ok(())
}

#[test]
#[should_panic(expected = "selected tab 3 is outside a set of 3")]
fn tabs_reject_a_selection_outside_the_set() {
	let _ = Tabs::new("settings", 3, 3);
}

#[test]
#[should_panic(expected = "tab 2 is outside a set of 2")]
fn tabs_reject_a_tab_outside_the_set() {
	let _ = Tabs::new("settings", 2, 0).tab(2, "Extra", |_| ());
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
fn field_keeps_descriptions_the_input_already_carries() {
	let field = Field::new("bio", "Biography").help("Shown on your profile.");
	let screen = View()
		.child(field.label_element())
		.child(Text("0 of 160 characters").id("bio-count"))
		.child(field.wire(TextInput().aria_described_by(["bio-count"])))
		.children(field.help_element());

	assert!(screen.validate().is_ok());
	let input = &screen.children[2];
	assert_eq!(input.props.id.as_deref(), Some("bio"));
	assert_eq!(input.props.aria().described_by, ["bio-count", "bio-help"]);
}

#[test]
fn dialog_opens_named_and_described() -> Result<(), leptoskit_testing::TestError> {
	let open = Arc::new(AtomicBool::new(false));
	let dialog = Dialog::new("discard", "Discard draft?").description("This cannot be undone.");
	let mut tester = WidgetTester::new(
		{
			let open = Arc::clone(&open);
			let dialog = dialog.clone();
			move || {
				dialog
					.container(open.load(Ordering::Relaxed))
					.child(dialog.title_element(2))
					.children(dialog.description_element())
			}
		},
		390.0,
		844.0,
	)?;

	assert!(tester.find(&Finder::text("Discard draft?")).is_err());

	open.store(true, Ordering::Relaxed);
	tester.pump();
	tester.find(&Finder::text("Discard draft?"))?;
	tester.find(&Finder::text("This cannot be undone."))?;

	// The container carries its role and both references, and no modal
	// promise the renderer cannot keep without focus containment.
	let tree = dialog
		.container(true)
		.child(dialog.title_element(2))
		.children(dialog.description_element());
	assert_eq!(tree.props.id.as_deref(), Some("discard"));
	assert_eq!(tree.props.aria().role, Some(Role::Dialog));
	assert_eq!(tree.props.aria().modal, None);
	assert_eq!(tree.props.aria().labelled_by, ["discard-title"]);
	assert_eq!(tree.props.aria().described_by, ["discard-description"]);
	assert!(tree.validate().is_ok());
	Ok(())
}

#[test]
fn dialog_without_description_references_none() {
	let dialog = Dialog::new("discard", "Discard draft?");
	let tree = dialog.container(true).child(dialog.title_element(2));
	assert_eq!(tree.props.aria().described_by, [] as [String; 0]);
	assert!(dialog.description_element().is_none());
	assert!(tree.validate().is_ok());
}

#[test]
fn dialog_without_title_fails_validation() {
	let dialog = Dialog::new("discard", "Discard draft?");
	let tree = dialog.container(true).child(Text("This cannot be undone."));
	assert!(
		tree.validate().is_err(),
		"a dialog must be named by its title"
	);
}
