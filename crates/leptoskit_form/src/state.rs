//! Form state: values, errors, and what has been touched or submitted.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::Form;

/// Every field's value, indexed by field id.
pub type Values = BTreeMap<String, String>;

/// The state of one form fill: the values, the errors, and how far the user
/// got.
///
/// The state never mutates itself. Typing proposes
/// [`for_change`](Self::for_change); submitting runs every rule through
/// [`submit`](crate::submit), which either hands back the values or the
/// state with its errors filled in. Errors only appear once a field is
/// touched or the form is submitted, the way react-aria and `TanStack Form`
/// surface them — an untouched form never shouts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormState {
	/// Every field's current value.
	pub values: Values,
	/// The errors validation filled in, by field id.
	pub errors: BTreeMap<String, String>,
	/// The fields the user has edited.
	pub touched: BTreeSet<String>,
	/// Whether a submit has been attempted.
	pub submitted: bool,
}

impl FormState {
	/// The state a form starts in: every field's initial value, no errors.
	pub fn initial(form: &Form) -> Self {
		Self {
			values: form
				.fields
				.iter()
				.map(|field| (field.id.clone(), field.initial.clone()))
				.collect(),
			errors: BTreeMap::new(),
			touched: BTreeSet::new(),
			submitted: false,
		}
	}

	/// The state an edit proposes: the new value, the field touched, and its
	/// error re-resolved — cleared when the value now stands, set when a
	/// touched field's rule rejects it.
	#[must_use]
	pub fn for_change(mut self, form: &Form, field_id: &str, value: impl Into<String>) -> Self {
		if self.values.contains_key(field_id) {
			self.values.insert(field_id.to_owned(), value.into());
			self.touched.insert(field_id.to_owned());

			let error = form
				.field(field_id)
				.and_then(|field| field.error_for(&self.values[field_id]));
			match error {
				Some(message) => {
					self.errors.insert(field_id.to_owned(), message);
				}
				None => {
					self.errors.remove(field_id);
				}
			}
		}
		self
	}

	/// A field's current value, when the form has the field.
	pub fn value_of(&self, field_id: &str) -> Option<&str> {
		self.values.get(field_id).map(String::as_str)
	}

	/// A field's current error, when it has one.
	pub fn error_of(&self, field_id: &str) -> Option<&str> {
		self.errors.get(field_id).map(String::as_str)
	}
}

/// What a submit produced: the values when every rule stands, or the state
/// with every error filled in when one does not.
pub type SubmitOutcome = Result<Values, FormState>;

/// Runs every field's rules and finishes the submission.
///
/// `Ok` carries every value, with the submitted state the caller keeps when
/// it wants the form to stay filled. `Err` carries the state with
/// `submitted` set and every failing rule's message in place, ready to
/// render: touched or not, a submitted form shows its errors.
pub fn submit(form: &Form, state: &FormState) -> SubmitOutcome {
	let mut outcome = state.clone();
	outcome.submitted = true;

	for field in &form.fields {
		let value = outcome.values.get(&field.id).cloned().unwrap_or_default();
		match field.error_for(&value) {
			Some(message) => {
				outcome.errors.insert(field.id.clone(), message);
			}
			None => {
				outcome.errors.remove(&field.id);
			}
		}
	}

	if outcome.errors.is_empty() {
		Ok(outcome.values)
	} else {
		Err(outcome)
	}
}
