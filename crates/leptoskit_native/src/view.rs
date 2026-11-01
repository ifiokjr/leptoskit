//! The values a component returns and the children a view accepts.
//!
//! [`IntoView`] turns a component's output into exactly one [`Element`];
//! [`IntoChildren`] appends zero or more elements to a parent. `leptoskit`'s
//! `view!` macro and `#[component]` attribute are built on these traits, but
//! they work just as well with the builder API:
//!
//! ```
//! use leptoskit_native::Element;
//! use leptoskit_native::components::Text;
//! use leptoskit_native::components::View;
//! use leptoskit_native::view::Fragment;
//!
//! let unread = 3;
//! let badge = (unread > 0).then(|| Text(unread.to_string()));
//! let row = View().children(
//! 	Fragment::new()
//! 		.with("Inbox")
//! 		.with(badge)
//! 		.with(|| Text("refreshed")),
//! );
//! assert_eq!(row.children.len(), 3);
//! ```
//!
//! Screens are rebuilt whenever their state changes, so a closure child is
//! called once per build. Reading a leptos signal inside it is tracked by the
//! reactive closure that rebuilds the screen.

use std::borrow::Cow;

use crate::Element;

/// A value that renders as exactly one [`Element`].
///
/// Components return `impl IntoView`. Strings and numbers render as
/// [`Text`](crate::components::Text), and a closure renders whatever it
/// returns.
pub trait IntoView {
	/// Converts this value into its element.
	fn into_view(self) -> Element;
}

impl IntoView for Element {
	fn into_view(self) -> Element {
		self
	}
}

impl IntoView for String {
	fn into_view(self) -> Element {
		Element::text(self)
	}
}

impl IntoView for &str {
	fn into_view(self) -> Element {
		Element::text(self)
	}
}

impl IntoView for &String {
	fn into_view(self) -> Element {
		Element::text(self.as_str())
	}
}

impl IntoView for Cow<'_, str> {
	fn into_view(self) -> Element {
		Element::text(self)
	}
}

impl IntoView for char {
	fn into_view(self) -> Element {
		Element::text(self.to_string())
	}
}

macro_rules! display_views {
	($($number:ty),* $(,)?) => {
		$(
			impl IntoView for $number {
				fn into_view(self) -> Element {
					Element::text(self.to_string())
				}
			}
		)*
	};
}

display_views!(
	i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64
);

/// A closure renders what it returns each time the view is built.
///
/// The bounds match a reactive region, so the same closures keep working
/// when regions start updating on their own.
impl<F, V> IntoView for F
where
	F: Fn() -> V + Send + Sync + 'static,
	V: IntoView,
{
	fn into_view(self) -> Element {
		self().into_view()
	}
}

/// A value that adds zero or more children to an element.
///
/// Every [`IntoView`] adds itself. `None` adds nothing, and vectors, arrays
/// and [`Fragment`]s add each of their items in order.
pub trait IntoChildren {
	/// Appends this value's elements to `children`, in order.
	fn append_to(self, children: &mut Vec<Element>);
}

impl<V: IntoView> IntoChildren for V {
	fn append_to(self, children: &mut Vec<Element>) {
		children.push(self.into_view());
	}
}

impl<C: IntoChildren> IntoChildren for Option<C> {
	fn append_to(self, children: &mut Vec<Element>) {
		if let Some(child) = self {
			child.append_to(children);
		}
	}
}

impl<C: IntoChildren> IntoChildren for Vec<C> {
	fn append_to(self, children: &mut Vec<Element>) {
		for child in self {
			child.append_to(children);
		}
	}
}

impl<C: IntoChildren, const N: usize> IntoChildren for [C; N] {
	fn append_to(self, children: &mut Vec<Element>) {
		for child in self {
			child.append_to(children);
		}
	}
}

impl IntoChildren for Fragment {
	fn append_to(self, children: &mut Vec<Element>) {
		children.extend(self.elements);
	}
}

/// An ordered list of sibling elements with no parent of its own.
///
/// `view!` returns a fragment for `<>…</>` and for markup with several root
/// nodes, and passes a tag's children to its builder as one. Collect an
/// iterator of views into a fragment with `collect::<Fragment>()`.
#[derive(Debug, Clone, Default)]
pub struct Fragment {
	elements: Vec<Element>,
}

/// The children a component receives from `view!`.
///
/// Declare a `children: Children` prop and pass it on with
/// [`Element::children`] or as `{children}` inside `view!`.
pub type Children = Fragment;

impl Fragment {
	/// An empty fragment.
	pub fn new() -> Self {
		Self::default()
	}

	/// Appends a child, or nothing, or several children.
	#[must_use]
	pub fn with(mut self, child: impl IntoChildren) -> Self {
		child.append_to(&mut self.elements);
		self
	}

	/// The elements, in order.
	pub fn elements(&self) -> &[Element] {
		&self.elements
	}

	/// How many elements the fragment holds.
	pub fn len(&self) -> usize {
		self.elements.len()
	}

	/// Whether the fragment holds no elements.
	pub fn is_empty(&self) -> bool {
		self.elements.is_empty()
	}
}

impl IntoIterator for Fragment {
	type IntoIter = std::vec::IntoIter<Element>;
	type Item = Element;

	fn into_iter(self) -> Self::IntoIter {
		self.elements.into_iter()
	}
}

impl<C: IntoChildren> FromIterator<C> for Fragment {
	fn from_iter<I: IntoIterator<Item = C>>(iter: I) -> Self {
		iter.into_iter().fold(Self::new(), Self::with)
	}
}

impl<C: IntoChildren> Extend<C> for Fragment {
	fn extend<I: IntoIterator<Item = C>>(&mut self, iter: I) {
		for child in iter {
			child.append_to(&mut self.elements);
		}
	}
}

/// Support for code that `view!` and `#[component]` generate. Not public API.
#[doc(hidden)]
pub mod __private {
	use crate::Element;

	/// Marks a `#[component]`'s body as running, so the elements it creates
	/// carry the component's name in `DevTools`. Without the `devtools`
	/// feature it is empty and does nothing.
	#[must_use = "the scope ends when the guard drops"]
	pub struct ComponentScope(());

	impl ComponentScope {
		/// Enters the component `name` until the guard drops.
		#[inline]
		pub fn enter(name: &'static str) -> Self {
			#[cfg(feature = "devtools")]
			crate::devtools::enter_component(name);
			#[cfg(not(feature = "devtools"))]
			let _ = name;
			Self(())
		}
	}

	impl Drop for ComponentScope {
		#[inline]
		fn drop(&mut self) {
			#[cfg(feature = "devtools")]
			crate::devtools::leave_component();
		}
	}

	/// Finishes a `view!` tag. A built-in's builder is its element, so it
	/// finishes as itself; `#[component]` builders have an inherent `build`
	/// that takes precedence.
	pub trait BuildView {
		/// The finished element.
		fn build(self) -> Element;
	}

	impl BuildView for Element {
		fn build(self) -> Element {
			self
		}
	}

	/// A `#[component]` builder slot that has not been given a value yet.
	#[diagnostic::on_unimplemented(
		message = "this prop is already set",
		label = "remove the second value for this prop"
	)]
	pub trait Unset {}

	impl Unset for () {}

	/// A `#[component]` builder slot for an optional prop: either unset, so the
	/// default applies, or set.
	#[diagnostic::on_unimplemented(
		message = "`{Self}` does not hold a value for this optional prop"
	)]
	pub trait Optional<T> {
		/// The value that was set, or `default()`.
		fn value_or(self, default: impl FnOnce() -> T) -> T;
	}

	impl<T> Optional<T> for () {
		fn value_or(self, default: impl FnOnce() -> T) -> T {
			default()
		}
	}

	impl<T> Optional<T> for (T,) {
		fn value_or(self, _default: impl FnOnce() -> T) -> T {
			self.0
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::NativeKind;
	use crate::components::Text;
	use crate::components::View;

	fn texts(fragment: &Fragment) -> Vec<&str> {
		fragment
			.elements()
			.iter()
			.map(|element| element.props.text.as_deref().unwrap_or_default())
			.collect()
	}

	#[test]
	fn strings_and_numbers_render_as_text() {
		let label = String::from("owned");
		let fragment = Fragment::new()
			.with("borrowed")
			.with(&label)
			.with(label.clone())
			.with(Cow::Borrowed("cow"))
			.with('c')
			.with(-3_i8)
			.with(7_u64)
			.with(1.5_f32)
			.with(2.25_f64)
			.with(9_usize);
		assert!(
			fragment
				.elements()
				.iter()
				.all(|element| element.kind == NativeKind::Text)
		);
		assert_eq!(
			texts(&fragment),
			[
				"borrowed", "owned", "owned", "cow", "c", "-3", "7", "1.5", "2.25", "9"
			]
		);
	}

	#[test]
	fn optional_and_repeated_children_flatten_in_order() {
		let fragment = Fragment::new()
			.with(None::<Element>)
			.with(Some("a"))
			.with(vec!["b", "c"])
			.with(["d"])
			.with(Fragment::new().with("e"))
			.with(|| Text("f"));
		assert_eq!(texts(&fragment), ["a", "b", "c", "d", "e", "f"]);
		assert_eq!(fragment.len(), 6);
		assert!(!fragment.is_empty());
		assert!(Fragment::new().is_empty());
	}

	#[test]
	fn fragments_collect_extend_and_iterate() {
		let mut fragment = (1..=2).collect::<Fragment>();
		fragment.extend(["three"]);
		assert_eq!(texts(&fragment), ["1", "2", "three"]);

		let view = View().children(fragment);
		assert_eq!(view.children.len(), 3);
	}

	#[test]
	fn builders_finish_as_themselves() {
		use __private::BuildView;
		let element = BuildView::build(View().class("p-2"));
		assert_eq!(element.kind, NativeKind::View);
	}

	#[test]
	fn optional_slots_use_the_default_only_when_unset() {
		use __private::Optional;
		assert_eq!(().value_or(|| 4), 4);
		assert_eq!((9,).value_or(|| 4), 9);
	}
}
