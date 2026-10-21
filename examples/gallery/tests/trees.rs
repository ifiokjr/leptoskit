//! Each screen's element tree, recorded node by node, so a change to how a
//! screen is written (builder calls or `view!` markup) cannot change what it
//! renders without updating these snapshots.

use gallery::Assets;
use gallery::SCREENS;
use gallery::ScreenState;
use leptoskit_native::Element;
use sha2::Digest;
use sha2::Sha256;

/// One line per node: its kind, text, and classes, plus a digest of
/// everything else about it (style, props, handler slots) so that any
/// difference shows up on the node that has it.
fn outline(element: &Element, depth: usize, lines: &mut Vec<String>) {
	let mut node = element.clone();
	node.children.clear();
	// Asset paths are joined with the platform's separator; record them the
	// same way everywhere.
	node.props.source = node.props.source.map(|source| source.replace('\\', "/"));
	let digest = Sha256::digest(format!("{node:?}").as_bytes());
	let text = element
		.props
		.text
		.as_deref()
		.map(|text| format!(" {text:?}"))
		.unwrap_or_default();
	let classes = element.classes().classes();
	let classes = if classes.is_empty() {
		String::new()
	} else {
		format!(" [{classes}]")
	};
	lines.push(format!(
		"{}{:?}{text}{classes} #{}",
		"  ".repeat(depth),
		element.kind,
		&format!("{digest:x}")[..12]
	));
	for child in &element.children {
		outline(child, depth + 1, lines);
	}
}

#[test]
fn screens_render_the_recorded_trees() {
	let state = ScreenState::new(Assets::at("assets"));
	for screen in &SCREENS {
		let mut lines = Vec::new();
		outline(&screen.render(&state), 0, &mut lines);
		insta::assert_snapshot!(screen.slug, lines.join("\n"));
	}
}
