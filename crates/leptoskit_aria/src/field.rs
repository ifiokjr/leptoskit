//! Field wiring: the label, help, and error contract of one input.

use leptoskit_native::Element;

/// The labelling contract of one field, applied to any input element.
///
/// One `Field` names an input and carries its help text and error: the label
/// element, the help element, and the error element all exist to be placed
/// around the input, and [`wire`](Field::wire) connects them to the input
/// itself with `aria-labelledby` and `aria-describedby`, marking the value
/// `aria-invalid` while an error is set. Build a fresh `Field` on every
/// render from current state, so an error that appears rewires the input.
///
/// The renderer's validation holds the contract: an input labelled by an id
/// no element carries fails to commit, so a wired field cannot ship half
/// wired.
///
/// ```
/// use leptoskit_aria::Field;
/// use leptoskit_native::components::TextInput;
/// use leptoskit_native::components::View;
///
/// let field = Field::new("email", "Email address")
/// 	.help("We never share your address.")
/// 	.error("That address has no @.");
///
/// let screen = View()
/// 	.child(field.label_element())
/// 	.child(field.wire(TextInput()))
/// 	.children(field.help_element())
/// 	.children(field.error_element());
///
/// let input = &screen.children[1];
/// assert_eq!(input.props.id.as_deref(), Some("email"));
/// assert_eq!(input.props.aria().labelled_by, ["email-label"]);
/// assert_eq!(
/// 	input.props.aria().described_by,
/// 	["email-help", "email-error"]
/// );
/// assert_eq!(
/// 	input.props.aria().invalid,
/// 	Some(leptoskit_native::Invalid::True)
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
	id: String,
	label: String,
	help: Option<String>,
	error: Option<String>,
}

impl Field {
	/// A field whose input carries the id `id`, labelled `label`.
	pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
		Self {
			id: id.into(),
			label: label.into(),
			help: None,
			error: None,
		}
	}

	/// Attaches help text, shown beside or below the input.
	#[must_use]
	pub fn help(mut self, text: impl Into<String>) -> Self {
		self.help = Some(text.into());
		self
	}

	/// Attaches an error message; the field is invalid while one is set.
	#[must_use]
	pub fn error(mut self, text: impl Into<String>) -> Self {
		self.error = Some(text.into());
		self
	}

	/// The label element: a text node the input is named after.
	#[must_use]
	pub fn label_element(&self) -> Element {
		Element::text(self.label.clone()).id(format!("{}-label", self.id))
	}

	/// The help element, when help text is set.
	#[must_use]
	pub fn help_element(&self) -> Option<Element> {
		self.help
			.as_ref()
			.map(|text| Element::text(text.clone()).id(format!("{}-help", self.id)))
	}

	/// The error element, when an error is set. Announcing a new error is the
	/// surrounding live region's job, not the field's.
	#[must_use]
	pub fn error_element(&self) -> Option<Element> {
		self.error
			.as_ref()
			.map(|text| Element::text(text.clone()).id(format!("{}-error", self.id)))
	}

	/// Wires an input element to this field's label, help, and error.
	///
	/// Pass any control that reads a value: a `TextInput`, a `PasswordInput`,
	/// or a container acting as one. The input is returned with the field's
	/// id, `aria-labelledby`, and, while an error is set, `aria-invalid`
	/// applied. The help and error ids join any `aria-describedby` the input
	/// already carries, so a description the app set first survives.
	#[must_use]
	pub fn wire(&self, input: Element) -> Element {
		let described: Vec<String> = input
			.props
			.aria()
			.described_by
			.iter()
			.cloned()
			.chain(self.help.is_some().then(|| format!("{}-help", self.id)))
			.chain(self.error.is_some().then(|| format!("{}-error", self.id)))
			.collect();

		let wired = input
			.id(self.id.clone())
			.aria_labelled_by([format!("{}-label", self.id)])
			.aria_described_by(described);

		if self.error.is_some() {
			wired.aria_invalid(true)
		} else {
			wired
		}
	}
}
