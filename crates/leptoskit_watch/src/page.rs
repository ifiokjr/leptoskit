//! Interactive watch app pages.

use leptoskit_native::Element;

use crate::WatchBudget;
use crate::WatchError;
use crate::budget;

/// One interactive page inside a [`crate::WatchApp`].
///
/// A page is still a leptoskit `Element` tree — `View`, `Text`, `Button`, and
/// Tailwind classes work — but it is validated against a page budget before a
/// host presents it. Prefer a handful of large controls over a phone layout.
#[derive(Clone)]
pub struct WatchPage {
	id: String,
	title: Option<String>,
	content: Element,
	budget: WatchBudget,
}

impl std::fmt::Debug for WatchPage {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("WatchPage")
			.field("id", &self.id)
			.field("title", &self.title)
			.field("budget", &self.budget)
			.finish_non_exhaustive()
	}
}

impl WatchPage {
	/// Starts a page with the default page budget.
	#[must_use]
	pub fn new(id: impl Into<String>, content: Element) -> Self {
		Self {
			id: id.into(),
			title: None,
			content,
			budget: WatchBudget::page(),
		}
	}

	/// Sets the short title shown in system UI when the platform has one.
	#[must_use]
	pub fn title(mut self, title: impl Into<String>) -> Self {
		self.title = Some(title.into());
		self
	}

	/// Overrides the default page budget.
	#[must_use]
	pub fn budget(mut self, budget: WatchBudget) -> Self {
		self.budget = budget;
		self
	}

	/// Stable id within the app.
	#[must_use]
	pub fn id(&self) -> &str {
		&self.id
	}

	/// Optional system title.
	#[must_use]
	pub fn page_title(&self) -> Option<&str> {
		self.title.as_deref()
	}

	/// Content tree.
	#[must_use]
	pub fn content(&self) -> &Element {
		&self.content
	}

	/// Budget this page validates against.
	#[must_use]
	pub fn page_budget(&self) -> WatchBudget {
		self.budget
	}

	/// Validates the id and content against the budget.
	pub fn validate(&self) -> Result<(), WatchError> {
		validate_id(&self.id)?;
		budget::measure(&self.content, self.budget).map(|_| ())
	}
}

pub(crate) fn validate_id(id: &str) -> Result<(), WatchError> {
	if id.is_empty()
		|| !id
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
	{
		return Err(WatchError::InvalidId(id.to_owned()));
	}
	Ok(())
}
