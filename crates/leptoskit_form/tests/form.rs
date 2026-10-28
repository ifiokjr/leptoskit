//! Form behavior: validation, touch, submit, and the rendered wiring over
//! the real headless renderer.

use std::sync::Arc;
use std::sync::RwLock;

use leptoskit_form::FieldSpec;
use leptoskit_form::Form;
use leptoskit_form::FormState;
use leptoskit_form::input;
use leptoskit_form::submit;
use leptoskit_form::submit_button;
use leptoskit_native::components::View;
use leptoskit_testing::Finder;
use leptoskit_testing::WidgetTester;

fn signup() -> Form {
	Form::new("signup")
		.with_field(
			FieldSpec::new("email", "Email address")
				.with_help("We never share your address.")
				.required()
				.validate(|value| {
					(!value.contains('@')).then(|| "That address has no @.".to_owned())
				}),
		)
		.with_field(FieldSpec::new("name", "Full name").initial_value("Ifiok Jr."))
}

#[test]
fn an_untouched_form_is_quiet_until_submitted() {
	let form = signup();
	let state = FormState::initial(&form);

	// Initial values arrive untouched and error-free.
	assert_eq!(state.value_of("name"), Some("Ifiok Jr."));
	assert_eq!(state.error_of("email"), None);

	// A submit fills in every failing rule.
	let rejected = submit(&form, &state).expect_err("a required email is missing");
	assert_eq!(
		rejected.error_of("email"),
		Some("Email address is required.")
	);
	assert_eq!(rejected.error_of("name"), None, "the initial name stands");
	assert!(rejected.submitted);
}

#[test]
fn edits_stay_quiet_until_a_submit_then_re_resolve_live() {
	let form = signup();
	let mut state = FormState::initial(&form);

	// Typing before any submit never raises an error mid-word.
	state = state.for_change(&form, "email", "i");
	assert_eq!(state.error_of("email"), None);
	assert!(state.touched.contains("email"));

	// A submit fills in the error the value earns.
	state = submit(&form, &state).expect_err("the address has no @");
	assert_eq!(state.error_of("email"), Some("That address has no @."));

	// After it, every edit re-resolves the error: replaced while a rule
	// still rejects the value, cleared the moment it stands.
	state = state.for_change(&form, "email", "");
	assert_eq!(state.error_of("email"), Some("Email address is required."));
	state = state.for_change(&form, "email", "ifiok@example.com");
	assert_eq!(state.error_of("email"), None);

	// Submitting the valid form hands back every value.
	let values = submit(&form, &state).expect("a valid fill stands");
	assert_eq!(
		values.get("email").map(String::as_str),
		Some("ifiok@example.com")
	);
	assert_eq!(values.get("name").map(String::as_str), Some("Ifiok Jr."));

	// An edit to a field the form does not have changes nothing.
	let before = state.clone();
	state = state.for_change(&form, "phone", "555");
	assert_eq!(state, before);
}

#[test]
#[should_panic(expected = "form `signup` already has a field `email`")]
fn a_form_rejects_a_duplicate_field_id() {
	let _ = signup().with_field(FieldSpec::new("email", "Backup email"));
}

#[test]
fn two_forms_with_the_same_field_ids_share_a_screen() {
	let signup_form = signup();
	let login = Form::new("login").with_field(FieldSpec::new("email", "Email address"));
	let screen = View()
		.child(input(
			&signup_form,
			signup_form.field("email").expect("the form has it"),
			&FormState::initial(&signup_form),
			|_| (),
		))
		.child(input(
			&login,
			login.field("email").expect("the form has it"),
			&FormState::initial(&login),
			|_| (),
		));

	assert!(screen.validate().is_ok(), "each form prefixes its ids");
	assert_eq!(
		screen.children[0].children[1].props.id.as_deref(),
		Some("signup-email")
	);
	assert_eq!(
		screen.children[1].children[1].props.id.as_deref(),
		Some("login-email")
	);
}

#[test]
fn the_rendered_form_labels_help_and_invalidates() -> Result<(), leptoskit_testing::TestError> {
	let form = Arc::new(signup());
	let state = Arc::new(RwLock::new(FormState::initial(&form)));

	let mut tester = WidgetTester::new(
		{
			let form = Arc::clone(&form);
			let state = Arc::clone(&state);
			move || {
				let current = state.read().expect("uncontended").clone();
				View()
					.child(input(
						&form,
						form.field("email").expect("the form has it"),
						&current,
						{
							let form = Arc::clone(&form);
							let state = Arc::clone(&state);
							move |value| {
								if let Ok(mut next) = state.write() {
									*next = next.clone().for_change(&form, "email", value);
								}
							}
						},
					))
					.child(submit_button("Create account", &form, &current, {
						let state = Arc::clone(&state);
						move |outcome| {
							if let Err(rejected) = outcome
								&& let Ok(mut next) = state.write()
							{
								*next = rejected;
							}
						}
					}))
			}
		},
		390.0,
		644.0,
	)?;

	// The input is named after its label and described by its help, valid.
	let email_field = form.field("email").expect("the form has it");
	let initial = state.read().expect("uncontended").clone();
	let rendered = input(&form, email_field, &initial, |_| ());
	let label = &rendered.children[0];
	let field_input = &rendered.children[1];
	assert_eq!(label.props.id.as_deref(), Some("signup-email-label"));
	assert_eq!(field_input.props.aria().labelled_by, ["signup-email-label"]);
	assert_eq!(field_input.props.aria().described_by, ["signup-email-help"]);
	assert_eq!(field_input.props.aria().invalid, None);
	assert!(rendered.validate().is_ok(), "the wiring resolves");

	// Submitting through the real dispatch surfaces the error and
	// invalidates the value.
	tester.tap(&Finder::text("Create account"))?;
	tester.find(&Finder::text("Email address is required."))?;

	let rejected = state.read().expect("uncontended").clone();
	let rendered = input(&form, email_field, &rejected, |_| ());
	let field_input = &rendered.children[1];
	assert_eq!(
		field_input.props.aria().described_by,
		["signup-email-help", "signup-email-error"]
	);
	assert_eq!(
		field_input.props.aria().invalid,
		Some(leptoskit_native::Invalid::True)
	);
	Ok(())
}
