use std::cell::Cell;
use std::fmt;
use std::str::FromStr;

use markdown::mdast::AttributeContent;
use markdown::mdast::AttributeValue;

use crate::render::LinkLog;

/// The attributes written on a component tag, such as `kind="tip"` in
/// `<Callout kind="tip">`.
///
/// Values are text: `kind="tip"` and `kind={tip}` both read as `tip`, and an
/// attribute without a value, such as `open` in `<Details open>`, reads as
/// `true`. Parse anything else with [`Props::parse`].
///
/// Every attribute a component never reads is reported as unknown after it
/// renders, so a misspelt prop fails the build instead of being ignored.
pub struct Props<'a> {
	component: &'a str,
	attributes: Vec<Attribute>,
	links: &'a LinkLog<'a>,
	line: usize,
	column: usize,
}

struct Attribute {
	name: String,
	value: String,
	read: Cell<bool>,
}

impl<'a> Props<'a> {
	pub(crate) fn new(
		component: &'a str,
		attributes: &[AttributeContent],
		links: &'a LinkLog<'a>,
		(line, column): (usize, usize),
	) -> Result<Self, PropError> {
		let attributes = attributes
			.iter()
			.map(|attribute| {
				match attribute {
					AttributeContent::Property(property) => {
						let value = match &property.value {
							None => "true".to_owned(),
							Some(AttributeValue::Literal(value)) => value.clone(),
							Some(AttributeValue::Expression(expression)) => {
								expression.value.trim().to_owned()
							}
						};

						Ok(Attribute {
							name: property.name.clone(),
							value,
							read: Cell::new(false),
						})
					}
					AttributeContent::Expression(expression) => {
						Err(PropError::new(format!(
							"spread attributes such as `{{{}}}` are not supported",
							expression.value.trim()
						)))
					}
				}
			})
			.collect::<Result<_, _>>()?;

		Ok(Self {
			component,
			attributes,
			links,
			line,
			column,
		})
	}

	/// The component's name, such as `Callout`.
	pub fn component(&self) -> &str {
		self.component
	}

	/// The text of the attribute `name`, if it is set.
	pub fn get(&self, name: &str) -> Option<&str> {
		self.attributes
			.iter()
			.find(|attribute| attribute.name == name)
			.map(|attribute| {
				attribute.read.set(true);
				attribute.value.as_str()
			})
	}

	/// The text of the attribute `name`, which must be set.
	pub fn require(&self, name: &str) -> Result<&str, PropError> {
		self.get(name).ok_or_else(|| {
			PropError::new(format!("<{}> needs a `{name}` attribute", self.component))
		})
	}

	/// The attribute `name` parsed with [`FromStr`], if it is set.
	pub fn parse<T>(&self, name: &str) -> Result<Option<T>, PropError>
	where
		T: FromStr,
		T::Err: fmt::Display,
	{
		self.get(name)
			.map(|value| {
				value.parse().map_err(|error| {
					PropError::new(format!(
						"`{name}=\"{value}\"` on <{}> is invalid: {error}",
						self.component
					))
				})
			})
			.transpose()
	}

	/// The attribute `name` as a link target, if it is set.
	///
	/// Pass every URL a component renders through here rather than
	/// [`Props::get`]: links to `.md` files are rewritten to the pages built
	/// from them, a [`Site`](crate::Site) checks the target exists, and the
	/// scheme is held to links a page may navigate to, so a stray
	/// `javascript:` value cannot reach the published site.
	pub fn url(&self, name: &str) -> Option<Result<String, PropError>> {
		let url = self.get(name)?;
		// A colon before any separator names a scheme; anything else is a
		// relative link or a fragment, which a page may navigate to.
		let scheme = url
			.split(['/', '?', '#'])
			.next()
			.and_then(|start| start.split_once(':').map(|(scheme, _)| scheme));
		if scheme.is_some_and(|scheme| {
			!matches!(
				scheme.to_ascii_lowercase().as_str(),
				"http" | "https" | "mailto"
			)
		}) {
			return Some(Err(PropError::new(format!(
				"`{name}=\"{url}\"` on <{}> is not an http, https, mailto, or relative link",
				self.component
			))));
		}

		Some(Ok(self.links.record(url, self.line, self.column)))
	}

	pub(crate) fn unread(&self) -> impl Iterator<Item = &str> {
		self.attributes
			.iter()
			.filter(|attribute| !attribute.read.get())
			.map(|attribute| attribute.name.as_str())
	}
}

/// Why a component could not render from the props it was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropError(String);

impl PropError {
	/// An error with a message for the author of the document.
	pub fn new(message: impl Into<String>) -> Self {
		Self(message.into())
	}
}

impl fmt::Display for PropError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(&self.0)
	}
}

impl std::error::Error for PropError {}
