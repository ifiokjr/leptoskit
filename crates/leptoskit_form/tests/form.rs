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
fn errors_wait_for_leaving_the_field_then_track_every_edit() {
	let form = signup();
	let mut state = FormState::initial(&form);

	// Typing into a fresh field never raises an error mid-word.
	state = state.for_change(&form, "email", "i");
	assert_eq!(state.error_of("email"), None);
	assert!(!state.touched.contains("email"));

	// Leaving the field touches it and resolves the error its value earns.
	state = state.for_blur(&form, "email");
	assert!(state.touched.contains("email"));
	assert_eq!(state.error_of("email"), Some("That address has no @."));

	// From then on every edit re-resolves the error: replaced while a rule
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

	// Edits and blurs on a field the form does not have change nothing.
	let before = state.clone();
	state = state
		.for_change(&form, "phone", "555")
		.for_blur(&form, "phone");
	assert_eq!(state, before);
}

#[test]
fn a_submit_shows_errors_on_untouched_fields_and_tracks_edits_after() {
	let form = signup();
	let state = FormState::initial(&form).for_change(&form, "email", "i");

	let mut state = submit(&form, &state).expect_err("the address has no @");
	assert_eq!(state.error_of("email"), Some("That address has no @."));
	assert!(
		!state.touched.contains("email"),
		"submitting leaves nothing"
	);

	state = state.for_change(&form, "email", "ifiok@example.com");
	assert_eq!(state.error_of("email"), None);
}

#[test]
fn leaving_a_field_with_a_valid_value_raises_nothing() {
	let form = signup();
	let state = FormState::initial(&form).for_blur(&form, "name");
	assert!(state.touched.contains("name"));
	assert_eq!(state.error_of("name"), None, "the initial name stands");
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
			|| (),
		))
		.child(input(
			&login,
			login.field("email").expect("the form has it"),
			&FormState::initial(&login),
			|_| (),
			|| (),
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

/// The signup email field and submit button over shared state, the way a
/// screen owns its form.
fn signup_screen(
	form: &Arc<Form>,
	state: &Arc<RwLock<FormState>>,
) -> Result<WidgetTester, leptoskit_testing::TestError> {
	let form = Arc::clone(form);
	let state = Arc::clone(state);
	WidgetTester::new(
		move || {
			let current = state.read().expect("uncontended").clone();
			let update = {
				let state = Arc::clone(&state);
				move |change: &dyn Fn(FormState) -> FormState| {
					if let Ok(mut next) = state.write() {
						*next = change(next.clone());
					}
				}
			};
			let (on_change, on_blur, on_submit) = (update.clone(), update.clone(), update);
			let (change_form, blur_form) = (Arc::clone(&form), Arc::clone(&form));
			View()
				.child(input(
					&form,
					form.field("email").expect("the form has it"),
					&current,
					move |value| on_change(&|state| state.for_change(&change_form, "email", value)),
					move || on_blur(&|state| state.for_blur(&blur_form, "email")),
				))
				.child(submit_button(
					"Create account",
					&form,
					&current,
					move |outcome| {
						if let Err(rejected) = outcome {
							on_submit(&|_| rejected.clone());
						}
					},
				))
		},
		390.0,
		644.0,
	)
}

fn email_input() -> Finder {
	Finder::label("Email address")
}

#[test]
fn the_rendered_form_labels_help_and_invalidates() -> Result<(), leptoskit_testing::TestError> {
	let form = Arc::new(signup());
	let state = Arc::new(RwLock::new(FormState::initial(&form)));
	let mut tester = signup_screen(&form, &state)?;

	// The input is named after its label and described by its help, valid.
	let field_input = tester.find(&email_input())?;
	assert_eq!(field_input.props.id.as_deref(), Some("signup-email"));
	assert_eq!(field_input.props.aria().labelled_by, ["signup-email-label"]);
	assert_eq!(field_input.props.aria().described_by, ["signup-email-help"]);
	assert_eq!(field_input.props.aria().invalid, None);

	// Submitting through the real dispatch surfaces the error and
	// invalidates the value.
	tester.tap(&Finder::text("Create account"))?;
	tester.find(&Finder::text("Email address is required."))?;

	let field_input = tester.find(&email_input())?;
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

#[test]
fn leaving_the_rendered_input_shows_its_error_and_a_fix_clears_it()
-> Result<(), leptoskit_testing::TestError> {
	let form = Arc::new(signup());
	let state = Arc::new(RwLock::new(FormState::initial(&form)));
	let mut tester = signup_screen(&form, &state)?;

	// Typing alone stays quiet.
	tester.enter_text(&email_input(), "ifiok")?;
	assert!(
		tester
			.find(&Finder::text("That address has no @."))
			.is_err()
	);

	// Leaving the field through its blur handler shows the error.
	tester.blur(&email_input())?;
	tester.find(&Finder::text("That address has no @."))?;
	assert_eq!(
		tester.find(&email_input())?.props.aria().invalid,
		Some(leptoskit_native::Invalid::True)
	);

	// The edit that fixes the value clears the error at once.
	tester.enter_text(&email_input(), "ifiok@example.com")?;
	assert!(
		tester
			.find(&Finder::text("That address has no @."))
			.is_err()
	);
	assert_eq!(tester.find(&email_input())?.props.aria().invalid, None);
	Ok(())
}
