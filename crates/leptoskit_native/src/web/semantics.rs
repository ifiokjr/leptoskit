//! ARIA attributes and semantic HTML elements for the web renderer.
//!
//! The first rule of ARIA is to prefer native HTML: a `View` with the `list`
//! role renders as `<ul>`, a `Text` heading as `<h2>`, and landmarks as their
//! elements, so the `role` attribute only appears where no element implies
//! it. Every other property becomes the matching `aria-*` attribute.

use leptos::tachys::html::attribute::any_attribute::AnyAttribute;
use leptos::tachys::html::attribute::any_attribute::IntoAnyAttribute;
use leptos::tachys::html::attribute::custom::custom_attribute;

use crate::NativeKind;
use crate::Role;
use crate::aria::Aria;

/// The HTML element a container or text node renders as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Tag {
	/// A `<div>`.
	Div,
	/// A `<span>`.
	Span,
	/// An `<h1>` to `<h6>`, by level.
	Heading(u8),
	/// A `<ul>`.
	List,
	/// An `<li>`.
	ListItem,
	/// A `<nav>`.
	Nav,
	/// A `<main>`.
	Main,
	/// A `<header>`.
	Header,
	/// A `<footer>`.
	Footer,
	/// An `<aside>`.
	Aside,
	/// An `<article>`.
	Article,
	/// A `<section>`, which is a region once named.
	Section,
	/// A `<search>`.
	Search,
	/// A `<p>`.
	Paragraph,
	/// An `<hr>`.
	Rule,
}

impl Tag {
	/// The element name.
	pub(super) const fn name(self) -> &'static str {
		match self {
			Self::Div => "div",
			Self::Span => "span",
			Self::Heading(1) => "h1",
			Self::Heading(2) => "h2",
			Self::Heading(3) => "h3",
			Self::Heading(4) => "h4",
			Self::Heading(5) => "h5",
			Self::Heading(_) => "h6",
			Self::List => "ul",
			Self::ListItem => "li",
			Self::Nav => "nav",
			Self::Main => "main",
			Self::Header => "header",
			Self::Footer => "footer",
			Self::Aside => "aside",
			Self::Article => "article",
			Self::Section => "section",
			Self::Search => "search",
			Self::Paragraph => "p",
			Self::Rule => "hr",
		}
	}

	/// The role the element implies without a `role` attribute.
	const fn implied_role(self) -> Option<Role> {
		match self {
			Self::Div | Self::Span => None,
			Self::Heading(_) => Some(Role::Heading),
			Self::List => Some(Role::List),
			Self::ListItem => Some(Role::ListItem),
			Self::Nav => Some(Role::Navigation),
			Self::Main => Some(Role::Main),
			Self::Header => Some(Role::Banner),
			Self::Footer => Some(Role::ContentInfo),
			Self::Aside => Some(Role::Complementary),
			Self::Article => Some(Role::Article),
			Self::Section => Some(Role::Region),
			Self::Search => Some(Role::Search),
			Self::Paragraph => Some(Role::Paragraph),
			Self::Rule => Some(Role::Separator),
		}
	}
}

/// Picks the element for a container or text node from its explicit role.
///
/// Typed controls keep their elements; this only applies to `View`,
/// `ScrollView`, `Text`, and `TextRun`.
pub(super) fn tag(kind: NativeKind, aria: &Aria, named: bool, leaf: bool) -> Tag {
	let container = matches!(kind, NativeKind::View | NativeKind::ScrollView);

	match (container, aria.role) {
		(true, Some(Role::List)) => Tag::List,
		(true, Some(Role::ListItem)) => Tag::ListItem,
		(true, Some(Role::Navigation)) => Tag::Nav,
		(true, Some(Role::Main)) => Tag::Main,
		(true, Some(Role::Banner)) => Tag::Header,
		(true, Some(Role::ContentInfo)) => Tag::Footer,
		(true, Some(Role::Complementary)) => Tag::Aside,
		(true, Some(Role::Article)) => Tag::Article,
		(true, Some(Role::Region)) if named => Tag::Section,
		(true, Some(Role::Search)) => Tag::Search,
		(true, Some(Role::Paragraph)) => Tag::Paragraph,
		(true, Some(Role::Separator)) if leaf => Tag::Rule,
		(true, _) => Tag::Div,
		(false, Some(Role::Heading)) => {
			match aria.level {
				Some(level @ 1..=6) => Tag::Heading(level as u8),
				_ => Tag::Span,
			}
		}
		(false, _) => Tag::Span,
	}
}

/// Whether a container or text node declares a control role, which the
/// renderer must make focusable and activate from the keyboard, since no
/// native element does it for them.
pub(super) fn custom_control(kind: NativeKind, aria: &Aria) -> bool {
	matches!(
		kind,
		NativeKind::View | NativeKind::ScrollView | NativeKind::Text | NativeKind::TextRun
	) && aria.role.is_some_and(Role::is_interactive)
}

/// The role the rendered element already implies, so a matching explicit
/// role is not repeated as an attribute.
fn implied_role(kind: NativeKind, tag: Tag) -> Option<Role> {
	match kind {
		NativeKind::View | NativeKind::ScrollView | NativeKind::Text | NativeKind::TextRun => {
			tag.implied_role()
		}
		// A drawing renders as a `<span>`, which implies no role of its own.
		NativeKind::Vector => None,
		other => Role::implicit(other),
	}
}

/// The `id`, `role`, `aria-*`, and `tabindex` attributes of one node.
pub(super) fn attributes(
	id: Option<String>,
	aria: &Aria,
	kind: NativeKind,
	tag: Tag,
	hidden: bool,
	custom_control: bool,
) -> Vec<AnyAttribute> {
	let mut attrs = Vec::new();
	let flag = |value: bool| Some(if value { "true" } else { "false" }.to_owned());
	let number = |value: f64| Some(value.to_string());
	let ids = |ids: &[String]| (!ids.is_empty()).then(|| ids.join(" "));

	push(&mut attrs, "id", id);
	push(
		&mut attrs,
		"role",
		aria.role
			.filter(|role| Some(*role) != implied_role(kind, tag))
			.map(|role| role.wire_name().to_owned()),
	);
	push(&mut attrs, "aria-labelledby", ids(&aria.labelled_by));
	push(&mut attrs, "aria-description", aria.description.clone());
	push(&mut attrs, "aria-describedby", ids(&aria.described_by));
	push(
		&mut attrs,
		"aria-roledescription",
		aria.role_description.clone(),
	);
	push(
		&mut attrs,
		"aria-level",
		aria.level
			.filter(|_| !matches!(tag, Tag::Heading(_)))
			.map(|level| level.to_string()),
	);
	push(
		&mut attrs,
		"aria-posinset",
		aria.pos_in_set.map(|position| position.to_string()),
	);
	push(
		&mut attrs,
		"aria-setsize",
		aria.set_size.map(|size| size.to_string()),
	);
	push(
		&mut attrs,
		"aria-orientation",
		aria.orientation.map(|value| value.as_str().to_owned()),
	);
	push(&mut attrs, "aria-hidden", hidden.then(|| "true".to_owned()));
	push(&mut attrs, "aria-busy", aria.busy.and_then(flag));
	push(
		&mut attrs,
		"aria-checked",
		aria.checked.map(|value| value.as_str().to_owned()),
	);
	push(
		&mut attrs,
		"aria-pressed",
		aria.pressed.map(|value| value.as_str().to_owned()),
	);
	push(&mut attrs, "aria-selected", aria.selected.and_then(flag));
	push(&mut attrs, "aria-expanded", aria.expanded.and_then(flag));
	push(
		&mut attrs,
		"aria-current",
		aria.current.map(|value| value.as_str().to_owned()),
	);
	push(&mut attrs, "aria-modal", aria.modal.and_then(flag));
	push(&mut attrs, "aria-required", aria.required.and_then(flag));
	push(
		&mut attrs,
		"aria-invalid",
		aria.invalid.map(|value| value.as_str().to_owned()),
	);
	push(&mut attrs, "aria-readonly", aria.read_only.and_then(flag));
	push(&mut attrs, "aria-multiline", aria.multiline.and_then(flag));
	push(
		&mut attrs,
		"aria-haspopup",
		aria.has_popup.map(|value| value.as_str().to_owned()),
	);
	push(&mut attrs, "aria-valuenow", aria.value_now.and_then(number));
	push(&mut attrs, "aria-valuemin", aria.value_min.and_then(number));
	push(&mut attrs, "aria-valuemax", aria.value_max.and_then(number));
	push(&mut attrs, "aria-valuetext", aria.value_text.clone());
	push(
		&mut attrs,
		"aria-live",
		aria.live.map(|value| value.as_str().to_owned()),
	);
	push(&mut attrs, "aria-atomic", aria.atomic.and_then(flag));
	push(&mut attrs, "aria-controls", ids(&aria.controls));
	push(
		&mut attrs,
		"tabindex",
		aria.tab_index
			.map(|index| index.to_string())
			.or_else(|| custom_control.then(|| "0".to_owned())),
	);

	for (name, value) in &aria.extra {
		attrs.push(custom_attribute(name.clone(), value.clone()).into_any_attr());
	}

	attrs
}

fn push(attrs: &mut Vec<AnyAttribute>, name: &'static str, value: Option<String>) {
	if let Some(value) = value {
		attrs.push(custom_attribute(name, value).into_any_attr());
	}
}
