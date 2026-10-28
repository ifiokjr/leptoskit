//! Headless form state for the shared leptoskit element tree.
//!
//! This is `TanStack Form`'s core loop, ported to leptoskit's grain: a
//! [`Form`] declares its [`FieldSpec`]s — labels, initial values, `required`,
//! and validation rules over the text the user entered — and a [`FormState`]
//! holds what the user did. Typing proposes [`FormState::for_change`];
//! [`submit`] runs every rule and returns the values or the error-filled
//! state. Errors first appear on submit and then track every edit live, so a
//! fix clears its error at once and typing never raises one mid-word.
//!
//! The rendering side is `leptoskit_aria`'s [`Field`](leptoskit_aria::Field)
//! wiring: [`field_wiring`] names the input after its label, carries help and
//! error through `aria-describedby`, and marks the value `aria-invalid` while
//! an error shows — so one form is announced correctly by screen readers on
//! the web and every native backend, and the renderer's validation holds the
//! contract.
//!
//! ```
//! use leptoskit_form::FieldSpec;
//! use leptoskit_form::Form;
//! use leptoskit_form::FormState;
//! use leptoskit_form::submit;
//!
//! let form = Form::new("signup").with_field(
//! 	FieldSpec::new("email", "Email address")
//! 		.required()
//! 		.validate(|value| (!value.contains('@')).then(|| "That address has no @.".to_owned())),
//! );
//!
//! // An untouched submit shows every error.
//! let state = FormState::initial(&form);
//! let rejected = submit(&form, &state).expect_err("an empty required field fails");
//! assert_eq!(
//! 	rejected.error_of("email"),
//! 	Some("Email address is required.")
//! );
//!
//! // After a submit, each edit re-resolves the error as the user types.
//! let state = rejected.for_change(&form, "email", "ifiok");
//! assert_eq!(state.error_of("email"), Some("That address has no @."));
//!
//! let state = state.for_change(&form, "email", "ifiok@example.com");
//! assert_eq!(state.error_of("email"), None);
//! let values = submit(&form, &state).expect("a valid address stands");
//! assert_eq!(
//! 	values.get("email").map(String::as_str),
//! 	Some("ifiok@example.com")
//! );
//! ```

mod spec;
mod state;
mod view;

pub use spec::FieldSpec;
pub use spec::Form;
pub use spec::Validator;
pub use state::FormState;
pub use state::SubmitOutcome;
pub use state::Values;
pub use state::submit;
pub use view::field_wiring;
pub use view::input;
pub use view::submit_button;
