//! Testing Library style queries over anything that can list [`Finder`]
//! matches.

use leptoskit_native::Role;

use crate::Finder;
use crate::TestError;

/// Defines the `get_by`, `get_all_by`, `query_by`, and `query_all_by`
/// methods for one kind of [`Finder`], with the arguments that build it.
macro_rules! finder_queries {
	(
		$what:literal,
		[$get:ident, $get_all:ident, $query:ident, $query_all:ident]
		($($argument:ident: $kind:ty),+) => $finder:expr
	) => {
		#[doc = concat!("Exactly one visible node ", $what, "; see [`Self::get`].")]
		fn $get(&self, $($argument: $kind),+) -> Result<Self::Node, TestError> {
			self.get(&$finder)
		}

		#[doc = concat!("Every visible node ", $what, ", at least one; see [`Self::get_all`].")]
		fn $get_all(&self, $($argument: $kind),+) -> Result<Vec<Self::Node>, TestError> {
			self.get_all(&$finder)
		}

		#[doc = concat!("The visible node ", $what, ", if any; see [`Self::query`].")]
		fn $query(&self, $($argument: $kind),+) -> Result<Option<Self::Node>, TestError> {
			self.query(&$finder)
		}

		#[doc = concat!("Every visible node ", $what, ", possibly none; see [`Self::query_all`].")]
		fn $query_all(&self, $($argument: $kind),+) -> Vec<Self::Node> {
			self.query_all(&$finder)
		}
	};
}

/// Testing Library's queries, shared by every surface a test can render to.
///
/// [`WidgetTester`](crate::WidgetTester) implements it over the headless
/// native renderer, and `leptoskit_testing_library::Screen` over the browser
/// DOM, so the same assertion reads the same on both. Hidden nodes never
/// match.
///
/// Each query family has a fixed contract for how many matches it accepts:
///
/// | Query                         | No match       | One match    | Several matches |
/// | ----------------------------- | -------------- | ------------ | --------------- |
/// | [`get`](Self::get)            | error          | the node     | error           |
/// | [`get_all`](Self::get_all)    | error          | `vec![node]` | every node      |
/// | [`query`](Self::query)        | `Ok(None)`     | `Some(node)` | error           |
/// | [`query_all`](Self::query_all)| empty          | `vec![node]` | every node      |
///
/// Errors carry [`describe`](Self::describe)'s output, so a failed assertion
/// shows what was on screen. The `_by_text`, `_by_label`, `_by_role`, and
/// `_by_test_id` forms build the [`Finder`] for you.
///
/// ```
/// use leptoskit_native::Element;
/// use leptoskit_native::Role;
/// use leptoskit_testing::Queries;
/// use leptoskit_testing::WidgetTester;
///
/// let tester = WidgetTester::new(
/// 	|| {
/// 		Element::view()
/// 			.child(Element::text("Inbox"))
/// 			.child(Element::button("Archive"))
/// 	},
/// 	390.0,
/// 	844.0,
/// )?;
/// tester.get_by_role(Role::Button, "Archive")?;
/// assert!(tester.query_by_text("Spam")?.is_none());
/// assert!(tester.get_by_text("Spam").is_err());
/// # Ok::<(), leptoskit_testing::TestError>(())
/// ```
pub trait Queries {
	/// What a match is: an owned widget observation natively, a DOM element
	/// in the browser.
	type Node;

	/// Every visible match, in tree order.
	fn query_all(&self, finder: &Finder) -> Vec<Self::Node>;

	/// The current tree, as failed queries show it.
	fn describe(&self) -> String;

	/// Exactly one visible match; none or several is an error.
	fn get(&self, finder: &Finder) -> Result<Self::Node, TestError> {
		match <[Self::Node; 1]>::try_from(self.query_all(finder)) {
			Ok([node]) => Ok(node),
			Err(found) => {
				Err(query_error(
					self,
					format!("expected one match for {finder:?}, found {}", found.len()),
				))
			}
		}
	}

	/// Every visible match; none is an error.
	fn get_all(&self, finder: &Finder) -> Result<Vec<Self::Node>, TestError> {
		let found = self.query_all(finder);

		if found.is_empty() {
			return Err(query_error(
				self,
				format!("expected at least one match for {finder:?}, found 0"),
			));
		}

		Ok(found)
	}

	/// The visible match, or `None`; several is an error, as an ambiguous
	/// absence check would hide a duplicate.
	fn query(&self, finder: &Finder) -> Result<Option<Self::Node>, TestError> {
		let mut found = self.query_all(finder);

		if found.len() > 1 {
			return Err(query_error(
				self,
				format!(
					"expected at most one match for {finder:?}, found {}",
					found.len()
				),
			));
		}

		Ok(found.pop())
	}

	finder_queries!(
		"whose text is exactly `text`",
		[get_by_text, get_all_by_text, query_by_text, query_all_by_text]
		(text: &str) => Finder::text(text)
	);

	finder_queries!(
		"whose accessibility label is exactly `label`",
		[get_by_label, get_all_by_label, query_by_label, query_all_by_label]
		(label: &str) => Finder::label(label)
	);

	finder_queries!(
		"with the ARIA `role`, explicit or implied, and the accessible `name`",
		[get_by_role, get_all_by_role, query_by_role, query_all_by_role]
		(role: Role, name: &str) => Finder::aria_role(role, name)
	);

	finder_queries!(
		"with the test identifier `id`",
		[get_by_test_id, get_all_by_test_id, query_by_test_id, query_all_by_test_id]
		(id: &str) => Finder::test_id(id)
	);
}

/// A failed query's error: the reason and the tree it was evaluated against.
fn query_error<Q: Queries + ?Sized>(queries: &Q, reason: String) -> TestError {
	TestError {
		reason,
		tree: queries.describe(),
	}
}
