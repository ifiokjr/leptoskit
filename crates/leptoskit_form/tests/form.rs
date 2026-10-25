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
fn touched_fields_re_resolve_as_the_user_types() {
	let form = signup();
	let mut state = FormState::initial(&form);

	state = state.for_change(&form, "email", "ifiok");
	assert_eq!(state.error_of("email"), Some("That address has no @."));

	// A rule that passes clears the error it had set.
	state = state.for_change(&form, "email", "ifiok@example.com");
	assert_eq!(state.error_of("email"), None);

	// Submitting the touched, valid form hands back every value.
	let values = submit(&form, &state).expect("a valid fill stands");
	assert_eq!(
		values.get("email").map(String::as_str),
		Some("ifiok@example.com")
	);
	assert_eq!(values.get("name").map(String::as_str), Some("Ifiok Jr."));

	// An edit to a field the form does not have changes nothing.
	let before = state.values.clone();
	state = state.for_change(&form, "phone", "555");
	assert_eq!(state.values, before);
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
							if let Err(rejected) = outcome {
								if let Ok(mut next) = state.write() {
									*next = rejected;
								}
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
	let rendered = input(email_field, &initial, |_| ());
	let label = &rendered.children[0];
	let field_input = &rendered.children[1];
	assert_eq!(label.props.id.as_deref(), Some("email-label"));
	assert_eq!(field_input.props.aria().labelled_by, ["email-label"]);
	assert_eq!(field_input.props.aria().described_by, ["email-help"]);
	assert_eq!(field_input.props.aria().invalid, None);
	assert!(rendered.validate().is_ok(), "the wiring resolves");

	// Submitting through the real dispatch surfaces the error and
	// invalidates the value.
	tester.tap(&Finder::text("Create account"))?;
	tester.find(&Finder::text("Email address is required."))?;

	let rejected = state.read().expect("uncontended").clone();
	let rendered = input(email_field, &rejected, |_| ());
	let field_input = &rendered.children[1];
	assert_eq!(
		field_input.props.aria().described_by,
		["email-help", "email-error"]
	);
	assert_eq!(
		field_input.props.aria().invalid,
		Some(leptoskit_native::Invalid::True)
	);
	Ok(())
}
