//! ARIA finders and the semantics outline the widget tester exposes.

use leptoskit_native::Element;
use leptoskit_native::Role;
use leptoskit_native::SliderState;
use leptoskit_native::Tristate;
use leptoskit_native::components::Button;
use leptoskit_native::components::Checkbox;
use leptoskit_native::components::Slider;
use leptoskit_native::components::Switch;
use leptoskit_native::components::Text;
use leptoskit_native::components::TextInput;
use leptoskit_native::components::View;
use leptoskit_testing::Finder;
use leptoskit_testing::TestError;
use leptoskit_testing::WidgetTester;

fn screen() -> Element {
	View()
		.child(Text("Profile").role(Role::Heading).aria_level(1))
		.child(Button("Continue").enabled(false))
		.child(Switch(true).aria_label("Wi-Fi"))
		.child(
			Checkbox(false)
				.aria_label("Agree")
				.aria_checked(Tristate::Mixed),
		)
		.child(
			TextInput()
				.aria_label("Email")
				.aria_required(true)
				.aria_description("Your work address"),
		)
		.child(
			View()
				.role(Role::Button)
				.aria_label("Custom action")
				.class("w-40 h-10"),
		)
		.child(View().aria_label("Order card").child(Text("Order #42")))
		.child(View().aria_hidden(true).child(Text("Decorative")))
		.child(View().role(Role::Status).child(Text("Saved")))
		.child(
			Slider(SliderState::new(0.0, 100.0, 40.0, Some(20.0)).unwrap())
				.aria_label("Volume")
				.aria_value_text("40 percent"),
		)
}

fn mounted() -> WidgetTester {
	WidgetTester::new(screen, 390.0, 844.0).expect("mounts")
}

#[test]
fn aria_role_finds_implied_and_explicit_roles_by_accessible_name() {
	let tester = mounted();

	let button = tester
		.find(&Finder::aria_role(Role::Button, "Continue"))
		.expect("kind-implied button");
	assert_eq!(button.props.enabled, Some(false));

	let custom = tester
		.find(&Finder::aria_role(Role::Button, "Custom action"))
		.expect("declared button role");
	assert_eq!(custom.props.aria().role, Some(Role::Button));

	tester
		.find(&Finder::aria_role(Role::Switch, "Wi-Fi"))
		.expect("switch");
	tester
		.find(&Finder::aria_role(Role::TextBox, "Email"))
		.expect("textbox");
	tester
		.find(&Finder::aria_role(Role::Heading, "Profile"))
		.expect("heading uses its text as the name");
	tester
		.find(&Finder::aria_role(Role::Slider, "Volume"))
		.expect("slider");

	assert!(matches!(
		tester.find(&Finder::aria_role(Role::Button, "Order card")),
		Err(TestError { .. })
	));
	assert!(
		tester
			.find(&Finder::aria_role(Role::Button, "Continue"))
			.is_ok()
	);
	assert_eq!(
		tester
			.find_all(&Finder::aria_role(Role::Status, "Saved"))
			.len(),
		1
	);
}

#[test]
fn the_semantics_outline_shows_what_screen_readers_expose() {
	let tester = mounted();
	let outline = tester.semantics();

	assert_eq!(
		outline,
		"\
		heading(1) \"Profile\"\nbutton \"Continue\" [disabled]\nswitch \"Wi-Fi\" \
		 [checked=true]\ncheckbox \"Agree\" [checked=mixed]\ntextbox \"Email\" [required=true] \
		 description=\"Your work address\"\nbutton \"Custom action\"\nview \"Order card\"\nstatus \
		 \"Saved\" live=polite atomic\nslider \"Volume\" value=40 \"40 percent\"\n"
	);
}

#[test]
fn hidden_subtrees_and_unnamed_containers_leave_the_outline() {
	let tester = mounted();
	assert!(!tester.semantics().contains("Decorative"));
	assert!(!tester.semantics().contains("text \"Plain\""));

	let empty = WidgetTester::new(View, 10.0, 10.0).expect("mounts");
	assert_eq!(empty.semantics(), "");
	assert!(empty.semantics().is_empty());
}
