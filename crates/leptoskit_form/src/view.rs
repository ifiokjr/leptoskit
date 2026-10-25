//! The rendered form: labeled inputs and a submit button wired through
//! `leptoskit_aria` fields.

use leptoskit_aria::Field;
use leptoskit_native::Element;
use leptoskit_native::components::TextInput;
use leptoskit_native::components::View;

use crate::FieldSpec;
use crate::Form;
use crate::FormState;
use crate::SubmitOutcome;
use crate::submit;

/// One field's accessible wiring: label, input, help, and the error the
/// state carries.
///
/// The [`Field`](Field) is `leptoskit_aria`'s: `aria-labelledby` naming the
/// input after the label, help and error in `aria-describedby`,
/// `aria-invalid` while an error shows. Build a fresh wiring on every render
/// so an error that appears rewires the input.
///
/// ```
/// use leptoskit_form::FieldSpec;
/// use leptoskit_form::Form;
/// use leptoskit_form::FormState;
/// use leptoskit_form::field_wiring;
///
/// let form = Form::new("signup").with_field(FieldSpec::new("email", "Email address").required());
/// let state = FormState::initial(&form);
///
/// let wiring = field_wiring(form.field("email").expect("the form has it"), &state);
/// assert_eq!(
/// 	wiring.label_element().props.text.as_deref(),
/// 	Some("Email address")
/// );
/// assert!(
/// 	wiring.error_element().is_none(),
/// 	"an untouched form is quiet"
/// );
/// ```
#[must_use]
pub fn field_wiring(field: &FieldSpec, state: &FormState) -> Field {
	let mut wiring = Field::new(&field.id, &field.label);
	if let Some(help) = &field.help {
		wiring = wiring.help(help.clone());
	}
	if let Some(error) = state.error_of(&field.id) {
		wiring = wiring.error(error.to_owned());
	}
	wiring
}

/// One field rendered whole: label, wired input, help, and error, in a
/// column. `on_change` receives the proposed value.
#[must_use]
pub fn input(
	field: &FieldSpec,
	state: &FormState,
	on_change: impl Fn(&str) + Send + Sync + 'static,
) -> Element {
	let wiring = field_wiring(field, state);
	View()
		.class("flex-col gap-1")
		.child(wiring.label_element())
		.child(
			wiring
				.wire(TextInput().value(state.value_of(&field.id).unwrap_or_default()))
				.on_change(on_change)
				.class("h-8 rounded-md px-2"),
		)
		.children(wiring.help_element())
		.children(wiring.error_element())
}

/// The submit control: a button that runs every rule and hands the outcome
/// to `on_submit` — the values on success, the error-filled state on
/// failure.
#[must_use]
pub fn submit_button(
	label: impl Into<String>,
	form: &Form,
	state: &FormState,
	on_submit: impl Fn(SubmitOutcome) + Send + Sync + 'static,
) -> Element {
	let form = form.clone();
	let state = state.clone();
	Element::button(label)
		.on_press(move || on_submit(submit(&form, &state)))
		.class("px-3 py-1.5 rounded-md")
}
