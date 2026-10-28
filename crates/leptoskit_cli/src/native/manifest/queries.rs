//! Match the complete visibility constraint before adding a missing intent.
use std::collections::BTreeSet;
use std::fmt::Write as _;

use anyhow::Context;
use anyhow::Result;
use roxmltree::Node;

use super::ANDROID;
use super::Edit;

pub(super) fn insertion(
	source: &str,
	query: Option<Node<'_, '_>>,
	application: usize,
	missing: String,
) -> Result<Option<Edit>> {
	if missing.is_empty() {
		return Ok(None);
	}

	let Some(query) = query else {
		return Ok(Some((
			application..application,
			format!("<queries>\n{missing}</queries>\n"),
		)));
	};
	let range = query.range();
	let element = &source[range.clone()];

	if element.ends_with("/>") {
		return Ok(Some((
			range.end - 2..range.end,
			format!(">\n{missing}</queries>"),
		)));
	}

	let closing = range.start
		+ element
			.rfind("</")
			.context("queries closing element missing")?;
	Ok(Some((closing..closing, missing)))
}

pub(super) fn missing(
	queries: &[Node<'_, '_>],
	actions: &BTreeSet<&str>,
	intents: &BTreeSet<(&str, &str)>,
	prefix: &str,
) -> Result<String> {
	let requested = actions.iter().map(|action| (*action, None)).chain(
		intents
			.iter()
			.map(|(action, scheme)| (*action, Some(*scheme))),
	);
	let mut output = String::new();

	for (action, scheme) in requested {
		if queries.iter().any(|queries| {
			queries
				.children()
				.any(|intent| matches(intent, action, scheme))
		}) {
			continue;
		}

		write!(
			output,
			"<intent xmlns:{prefix}=\"{ANDROID}\"><action {prefix}:name=\"{action}\"/>"
		)?;

		if let Some(scheme) = scheme {
			write!(output, "<data {prefix}:scheme=\"{scheme}\"/>")?;
		}

		writeln!(output, "</intent>")?;
	}

	Ok(output)
}

fn matches(intent: Node<'_, '_>, action: &str, scheme: Option<&str>) -> bool {
	if !intent.has_tag_name("intent") {
		return false;
	}

	let children: Vec<_> = intent.children().filter(Node::is_element).collect();
	let required_children = 1 + usize::from(scheme.is_some());
	let has_action = children.iter().any(|node| {
		node.has_tag_name("action") && node.attribute((ANDROID, "name")) == Some(action)
	});
	let has_scheme = scheme.is_none_or(|scheme| {
		children.iter().any(|node| {
			node.has_tag_name("data")
				&& node.attributes().len() == 1
				&& node.attribute((ANDROID, "scheme")) == Some(scheme)
		})
	});
	children.len() == required_children && has_action && has_scheme
}
