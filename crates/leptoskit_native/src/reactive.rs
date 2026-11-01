//! Values that follow a signal instead of being rebuilt with the tree.
//!
//! An [`Element`](crate::Element) is a description: the renderer rebuilds the
//! whole tree whenever application state changes and diffs it against what the
//! host already has. That is the right shape for structural change — a row
//! added, a screen replaced — and the wrong shape for a value that changes in
//! place: rebuilding a thousand labels to change a hundred of them costs the
//! interpreter work of all thousand.
//!
//! [`Element::text_from`](crate::Element::text_from) and
//! [`Element::background_from`](crate::Element::background_from) bind one
//! element's value to a closure that reads a [`RwSignal`]. The host keeps the
//! node; a refresh reads each binding, sends the nodes whose value actually
//! changed, and commits once. The change is proportional to the nodes that
//! changed, and the application writes its state the way it does on the web.

use std::fmt;
use std::sync::Arc;

pub use reactive_graph::signal::RwSignal;
pub use reactive_graph::traits::Get;
pub use reactive_graph::traits::Set;

use crate::props::Color;

/// A value read from application state on demand.
///
/// The closure reads the signals it needs, so a node bound to it changes when
/// those signals do, without the element tree being rebuilt.
pub struct Binding<T>(Arc<dyn Fn() -> T + Send + Sync>);

impl<T> Binding<T> {
	/// Binds a value to `read`.
	pub fn new(read: impl Fn() -> T + Send + Sync + 'static) -> Self {
		Self(Arc::new(read))
	}

	/// The current value.
	pub fn read(&self) -> T {
		(self.0)()
	}
}

impl<T> Clone for Binding<T> {
	fn clone(&self) -> Self {
		Self(Arc::clone(&self.0))
	}
}

impl<T> fmt::Debug for Binding<T>
where
	T: fmt::Debug,
{
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_tuple("Binding").field(&self.read()).finish()
	}
}

/// What an element reads from application state instead of from the tree it
/// was built in.
#[derive(Clone, Debug, Default)]
pub struct Bindings {
	/// The text the element shows.
	pub text: Option<Binding<String>>,
	/// The element's background colour.
	pub background: Option<Binding<Color>>,
}

impl Bindings {
	/// Whether nothing is bound.
	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.text.is_none() && self.background.is_none()
	}
}
