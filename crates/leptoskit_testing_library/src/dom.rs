//! Matches the DOM the web renderer emits against a [`Finder`].
//!
//! Roles and accessible names follow what the renderer writes: an explicit
//! `role` attribute where no element implies the role, and otherwise the
//! semantic element itself (`<button>`, `<h2>`, `<ul>`, `<nav>`, …).

use leptoskit_native::Role;
use leptoskit_testing::Finder;
use wasm_bindgen::JsCast;
use web_sys::Element;
use web_sys::HtmlElement;
use web_sys::HtmlInputElement;
use web_sys::Node;

/// How much of the rendered markup an error shows, in characters, like
/// Testing Library's default print limit.
const DESCRIBE_LIMIT: usize = 7000;

/// Every visible HTML element under `root`, in document order.
///
/// An element with `hidden`, `aria-hidden="true"`, or `display: none` hides
/// its whole subtree; `visibility: hidden` hides only the elements that
/// inherit it.
pub(crate) fn visible_elements(root: &Element) -> Vec<HtmlElement> {
	let mut found = Vec::new();
	let mut pending = children(root);
	pending.reverse();

	while let Some(element) = pending.pop() {
		let style = computed_style(&element);
		let property = |name: &str| {
			style
				.as_ref()
				.map(|style| {
					style
						.get_property_value(name)
						.expect("a computed style reads any property")
				})
				.unwrap_or_default()
		};

		if element.has_attribute("hidden")
			|| element.get_attribute("aria-hidden").as_deref() == Some("true")
			|| property("display") == "none"
		{
			continue;
		}

		if !matches!(property("visibility").as_str(), "hidden" | "collapse")
			&& let Some(element) = element.dyn_ref::<HtmlElement>()
		{
			found.push(element.clone());
		}

		let mut descendants = children(&element);
		descendants.reverse();
		pending.extend(descendants);
	}

	found
}

/// Whether `element` matches `finder`, given that it is visible.
pub(crate) fn matches(finder: &Finder, element: &Element) -> bool {
	match finder {
		Finder::Text(text) => has_own_text(element, text),
		Finder::Label(label) => label_of(element).as_deref() == Some(label.as_str()),
		Finder::Role(kind, name) => {
			Role::implicit(*kind).is_some_and(|role| has_role(element, role, name))
		}
		Finder::AriaRole(role, name) => has_role(element, *role, name),
		Finder::TestId(id) => element.get_attribute("data-testid").as_deref() == Some(id.as_str()),
	}
}

/// Whether `element` refuses interaction: natively disabled, or inside an
/// `aria-disabled="true"` subtree, as the renderer marks disabled
/// containers.
pub(crate) fn disabled(element: &Element) -> bool {
	element
		.matches(":disabled")
		.expect("`:disabled` is a valid selector")
		|| element
			.closest("[aria-disabled=\"true\"]")
			.expect("an attribute selector is valid")
			.is_some()
}

/// The rendered markup under `root` for diagnostics, without the scoped
/// stylesheets, style hashes, and hydration markers that would drown it, cut
/// to a readable length.
pub(crate) fn describe(root: &Element) -> String {
	let copy = root
		.clone_node_with_deep(true)
		.expect("an element clones")
		.unchecked_into::<Element>();
	strip_noise(&copy);

	let markup = copy.outer_html();
	match markup.char_indices().nth(DESCRIBE_LIMIT) {
		Some((cut, _)) => {
			format!(
				"{}… ({} more characters)",
				&markup[..cut],
				markup[cut..].chars().count()
			)
		}
		None => markup,
	}
}

/// The role an element exposes: its first recognized `role` token, else the
/// role its HTML element implies.
fn role_of(element: &Element) -> Option<Role> {
	element
		.get_attribute("role")
		.and_then(|roles| roles.split_whitespace().find_map(Role::from_wire))
		.or_else(|| implied_role(element))
}

/// The role an HTML element implies without a `role` attribute.
fn implied_role(element: &Element) -> Option<Role> {
	let role = match element.local_name().as_str() {
		"button" => Role::Button,
		"a" if element.has_attribute("href") => Role::Link,
		"h1" | "h2" | "h3" | "h4" | "h5" | "h6" => Role::Heading,
		"input" => return element.dyn_ref::<HtmlInputElement>().and_then(input_role),
		"textarea" => Role::TextBox,
		"ul" | "ol" | "menu" => Role::List,
		"li" => Role::ListItem,
		"img" => Role::Img,
		"progress" => Role::ProgressBar,
		"table" => Role::Table,
		"thead" | "tbody" | "tfoot" => Role::RowGroup,
		"tr" => Role::Row,
		"td" => Role::Cell,
		"th" => Role::ColumnHeader,
		"nav" => Role::Navigation,
		"main" => Role::Main,
		"header" => Role::Banner,
		"footer" => Role::ContentInfo,
		"aside" => Role::Complementary,
		"article" => Role::Article,
		// A section is a region only once it has a name.
		"section" if label_of(element).is_some() => Role::Region,
		"search" => Role::Search,
		"form" => Role::Form,
		"dialog" => Role::Dialog,
		"p" => Role::Paragraph,
		"hr" => Role::Separator,
		_ => return None,
	};

	Some(role)
}

/// The role an `<input>` implies from its type. Password fields count as
/// text boxes, as `Role::implicit(NativeKind::PasswordInput)` does.
fn input_role(input: &HtmlInputElement) -> Option<Role> {
	let role = match input.type_().as_str() {
		"text" | "password" | "email" | "tel" | "url" => Role::TextBox,
		"search" => Role::SearchBox,
		"number" => Role::SpinButton,
		"checkbox" => Role::Checkbox,
		"radio" => Role::Radio,
		"range" => Role::Slider,
		"button" | "submit" | "reset" | "image" => Role::Button,
		_ => return None,
	};

	Some(role)
}

fn has_role(element: &Element, role: Role, name: &str) -> bool {
	role_of(element) == Some(role) && accessible_name(element) == name
}

/// The name assistive technology announces: the label, else the trimmed
/// text content.
fn accessible_name(element: &Element) -> String {
	label_of(element).unwrap_or_else(|| trimmed_text(element))
}

/// An explicit, non-blank name: `aria-label`, else the text of the
/// `aria-labelledby` targets, else an image's `alt`.
fn label_of(element: &Element) -> Option<String> {
	let non_blank = |label: String| (!label.trim().is_empty()).then_some(label);

	element
		.get_attribute("aria-label")
		.and_then(non_blank)
		.or_else(|| {
			element
				.get_attribute("aria-labelledby")
				.map(|ids| labelled_by(element, &ids))
				.and_then(non_blank)
		})
		.or_else(|| {
			(element.local_name() == "img")
				.then(|| element.get_attribute("alt"))
				.flatten()
				.and_then(non_blank)
		})
}

/// The trimmed texts of the elements `ids` names, joined by spaces.
fn labelled_by(element: &Element, ids: &str) -> String {
	let Some(document) = element.owner_document() else {
		return String::new();
	};

	ids.split_whitespace()
		.filter_map(|id| document.get_element_by_id(id))
		.map(|label| trimmed_text(&label))
		.collect::<Vec<_>>()
		.join(" ")
}

/// Whether `element` is the deepest element whose trimmed text is `text`:
/// its own text matches and no child's does, so a `<button>` wrapping a
/// matching `<span>` is not a second match.
fn has_own_text(element: &Element, text: &str) -> bool {
	trimmed_text(element) == text
		&& !children(element)
			.iter()
			.any(|child| trimmed_text(child) == text)
}

fn trimmed_text(element: &Element) -> String {
	element
		.text_content()
		.map(|text| text.trim().to_owned())
		.unwrap_or_default()
}

fn children(element: &Element) -> Vec<Element> {
	let children = element.children();
	(0..children.length())
		.filter_map(|index| children.item(index))
		.collect()
}

/// Removes what `describe` leaves out from a detached copy: `<style>`
/// elements, leptos's comment markers, style scopes, and empty classes.
fn strip_noise(node: &Node) {
	let children = node.child_nodes();

	for index in (0..children.length()).rev() {
		let Some(child) = children.item(index) else {
			continue;
		};
		let element = child.dyn_ref::<Element>();

		if child.node_type() == Node::COMMENT_NODE
			|| element.is_some_and(|element| element.local_name() == "style")
		{
			node.remove_child(&child)
				.expect("a node removes its own child");
			continue;
		}

		if let Some(element) = element {
			element
				.remove_attribute("data-lk-style")
				.expect("an element drops an attribute");

			if element.get_attribute("class").as_deref() == Some("") {
				element
					.remove_attribute("class")
					.expect("an element drops an attribute");
			}
		}

		strip_noise(&child);
	}
}

/// The element's computed style; `None` where the engine computes none, as
/// Firefox does inside an undisplayed frame, which leaves the element visible.
fn computed_style(element: &Element) -> Option<web_sys::CssStyleDeclaration> {
	web_sys::window()
		.expect("a browser window")
		.get_computed_style(element)
		.expect("getComputedStyle accepts an element")
}
