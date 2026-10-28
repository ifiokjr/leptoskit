//! Form specification: the fields, their rules, and their initial values.

use std::sync::Arc;

/// Why a field's value is rejected, or `None` when it stands.
pub type Validator = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// One field of a [`Form`](crate::Form): its identity, its label, and its
/// rules.
///
/// A field is data, not state: build the set once. `required` rejects empty
/// values; `validate` adds a rule whose message becomes the field's error.
///
/// ```
/// use leptoskit_form::FieldSpec;
///
/// let email = FieldSpec::new("email", "Email address")
/// 	.required()
/// 	.validate(|value| (!value.contains('@')).then(|| "That address has no @.".to_owned()));
/// assert_eq!(email.label, "Email address");
/// ```
#[derive(Clone)]
pub struct FieldSpec {
	/// The field's stable identity, used by state, wiring, and callbacks.
	pub id: String,
	/// The label element's text.
	pub label: String,
	/// Help text placed beside the input.
	pub help: Option<String>,
	/// The value the form starts with.
	pub initial: String,
	/// Whether an empty value is an error.
	pub required: bool,
	/// The field's validation rules, in order; the first failing rule's
	/// message is the field's error.
	pub validators: Vec<Validator>,
}

impl std::fmt::Debug for FieldSpec {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("FieldSpec")
			.field("id", &self.id)
			.field("label", &self.label)
			.field("required", &self.required)
			.finish_non_exhaustive()
	}
}

impl FieldSpec {
	/// A field over `id` with `label`, optional, starting empty.
	pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
		Self {
			id: id.into(),
			label: label.into(),
			help: None,
			initial: String::new(),
			required: false,
			validators: Vec::new(),
		}
	}

	/// Attaches help text below the input.
	#[must_use]
	pub fn with_help(mut self, help: impl Into<String>) -> Self {
		self.help = Some(help.into());
		self
	}

	/// Sets the value the form starts with.
	#[must_use]
	pub fn initial_value(mut self, value: impl Into<String>) -> Self {
		self.initial = value.into();
		self
	}

	/// Rejects an empty value.
	#[must_use]
	pub fn required(mut self) -> Self {
		self.required = true;
		self
	}

	/// Adds a validation rule: `None` accepts the value, `Some(message)`
	/// rejects it with the message.
	#[must_use]
	pub fn validate(
		mut self,
		rule: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
	) -> Self {
		self.validators.push(Arc::new(rule));
		self
	}

	/// The field's error for a value: the first failing rule's message.
	pub(crate) fn error_for(&self, value: &str) -> Option<String> {
		if self.required && value.trim().is_empty() {
			return Some(format!("{} is required.", self.label));
		}

		self.validators.iter().find_map(|rule| rule(value))
	}
}

/// A form's fields: the specification state evolves over.
///
/// ```
/// use leptoskit_form::FieldSpec;
/// use leptoskit_form::Form;
/// use leptoskit_form::FormState;
///
/// let form = Form::new("signup").with_field(FieldSpec::new("email", "Email address").required());
/// let state = FormState::initial(&form);
/// assert_eq!(state.value_of("email"), Some(""));
/// ```
#[derive(Debug, Clone)]
pub struct Form {
	/// The form's stable identity, the prefix its fields wire ids from.
	pub id: String,
	/// The form's fields, in display order.
	pub fields: Vec<FieldSpec>,
}

impl Form {
	/// A form with no fields yet.
	pub fn new(id: impl Into<String>) -> Self {
		Self {
			id: id.into(),
			fields: Vec::new(),
		}
	}

	/// Adds a field.
	///
	/// # Panics
	///
	/// When the form already has a field with the same id: state, wiring, and
	/// callbacks all address fields by id.
	#[must_use]
	pub fn with_field(mut self, field: FieldSpec) -> Self {
		assert!(
			self.field(&field.id).is_none(),
			"form `{}` already has a field `{}`",
			self.id,
			field.id
		);
		self.fields.push(field);
		self
	}

	/// The field with this id, if the form has one.
	#[must_use]
	pub fn field(&self, id: &str) -> Option<&FieldSpec> {
		self.fields.iter().find(|field| field.id == id)
	}
}
