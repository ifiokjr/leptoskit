//! The built-in component list stays in step with the crates that declare
//! the `view!` tags.

use std::collections::BTreeSet;
use std::path::Path;

/// The sources that declare built-in tags with `tag!(Name => …)`.
const TAG_SOURCES: &[&str] = &[
	"leptoskit_native/src/components.rs",
	"leptoskit_native/src/components/controls.rs",
	"leptoskit_native/src/components/helpers.rs",
];

/// The library sources whose `#[component]` functions apps call as tags.
const COMPONENT_SOURCES: &[&str] = &["leptoskit_query/src/client.rs"];

fn read(source: &str) -> String {
	let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
		.parent()
		.expect("crates directory");
	std::fs::read_to_string(crates.join(source)).expect("read component source")
}

/// The name that follows `prefix` on a line, up to the first character that
/// cannot be part of an identifier.
fn name_after<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
	let rest = &line[line.find(prefix)? + prefix.len()..];
	let end = rest
		.find(|character: char| !character.is_alphanumeric())
		.unwrap_or(rest.len());
	Some(&rest[..end])
}

#[test]
fn built_in_components_match_their_tags() {
	let mut declared = BTreeSet::new();
	for source in TAG_SOURCES {
		for line in read(source).lines() {
			if let Some(name) = name_after(line, "tag!(") {
				declared.insert(name.to_owned());
			}
		}
	}
	for source in COMPONENT_SOURCES {
		let text = read(source);
		let mut lines = text.lines();
		while let Some(line) = lines.next() {
			if line.trim() == "#[component]"
				&& let Some(name) = lines.next().and_then(|next| name_after(next, "pub fn "))
			{
				declared.insert(name.to_owned());
			}
		}
	}
	let listed: BTreeSet<String> = leptoskit_lint::BUILT_IN_COMPONENTS
		.iter()
		.map(|&name| name.to_owned())
		.collect();
	assert_eq!(listed, declared);
}

#[test]
fn every_rule_runs_by_default() {
	let rules = leptoskit_lint::LintConfig::default().rules;
	for rule in leptoskit_lint::Rule::ALL {
		assert_ne!(
			rule.level(&rules),
			leptoskit_lint::LintLevel::Allow,
			"{rule} runs by default"
		);
	}
}

#[test]
fn every_rule_has_a_section_in_the_linting_guide() {
	let guide = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/src/guide/23-linting.md");
	let guide = std::fs::read_to_string(guide).expect("read the linting guide");
	for rule in leptoskit_lint::Rule::ALL {
		let heading = format!("### `{}`", rule.name());
		assert!(
			guide.lines().any(|line| line == heading),
			"the linting guide has no `{heading}` section"
		);
	}
}
