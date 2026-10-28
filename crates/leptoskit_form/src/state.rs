//! Form state: values, errors, the fields the user has left, and whether a
//! submit was attempted.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::FieldSpec;
use crate::Form;

/// Every field's value, indexed by field id.
pub type Values = BTreeMap<String, String>;

/// The state of one form fill: the values, the errors, and how far the user
/// got.
///
/// The state never mutates itself. Typing proposes
/// [`for_change`](Self::for_change), leaving a field proposes
/// [`for_blur`](Self::for_blur), and submitting runs every rule through
/// [`submit`](crate::submit), which either hands back the values or the
/// state with its errors filled in.
///
/// A field's error first appears when the user leaves the field or submits
/// the form, the way react-aria and `TanStack Form` surface errors; typing
/// into a fresh field never raises one mid-word. From then on every edit
/// re-resolves the field's error live, so a fix clears it at once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormState {
	/// Every field's current value.
	pub values: Values,
	/// The errors validation filled in, by field id.
	pub errors: BTreeMap<String, String>,
	/// The fields the user has left at least once.
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

	/// The state an edit proposes: the field's new value.
	///
	/// Once the field is touched or the form is submitted, the field's error
	/// re-resolves against the new value — replaced while a rule still
	/// rejects it, cleared when it stands. Before that the edit raises no
	/// error. An edit to a field the form does not have changes nothing.
	#[must_use]
	pub fn for_change(mut self, form: &Form, field_id: &str, value: impl Into<String>) -> Self {
		let Some(field) = form.field(field_id) else {
			return self;
		};

		let value = value.into();
		if self.submitted || self.touched.contains(field_id) {
			self.resolve(field, &value);
		}
		self.values.insert(field_id.to_owned(), value);
		self
	}

	/// The state leaving a field proposes: the field touched and its error
	/// resolved against its current value. Leaving a field the form does not
	/// have changes nothing.
	#[must_use]
	pub fn for_blur(mut self, form: &Form, field_id: &str) -> Self {
		let Some(field) = form.field(field_id) else {
			return self;
		};

		let value = self.values.get(field_id).cloned().unwrap_or_default();
		self.resolve(field, &value);
		self.touched.insert(field_id.to_owned());
		self
	}

	/// Sets or clears one field's error from its rules.
	fn resolve(&mut self, field: &FieldSpec, value: &str) {
		match field.error_for(value) {
			Some(message) => {
				self.errors.insert(field.id.clone(), message);
			}
			None => {
				self.errors.remove(&field.id);
			}
		}
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
/// `Ok` carries every value; the caller decides whether the form resets or
/// stays filled. `Err` carries the state with `submitted` set and every
/// failing rule's message in place, ready to render: touched or not, a
/// submitted form shows its errors, and later edits re-resolve them live.
pub fn submit(form: &Form, state: &FormState) -> SubmitOutcome {
	let mut outcome = state.clone();
	outcome.submitted = true;

	for field in &form.fields {
		let value = outcome.values.get(&field.id).cloned().unwrap_or_default();
		outcome.resolve(field, &value);
	}

	if outcome.errors.is_empty() {
		Ok(outcome.values)
	} else {
		Err(outcome)
	}
}
