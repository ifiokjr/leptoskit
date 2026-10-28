//! Add selected plugin declarations while preserving app-owned XML verbatim.
mod queries;
#[cfg(test)]
mod scheme_tests;

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::ops::Range;

use anyhow::Context;
use anyhow::Result;

const ANDROID: &str = "http://schemas.android.com/apk/res/android";
type Edit = (Range<usize>, String);

pub fn merge(
	source: &str,
	permissions: &BTreeSet<&str>,
	actions: &BTreeSet<&str>,
	intents: &BTreeSet<(&str, &str)>,
) -> Result<String> {
	let document = roxmltree::Document::parse(source).context("invalid AndroidManifest.xml")?;
	let root = document.root_element();
	anyhow::ensure!(
		root.has_tag_name("manifest"),
		"Android XML root must be manifest"
	);
	let prefix = root
		.lookup_prefix(ANDROID)
		.context("Android manifest must declare the Android attribute namespace")?;
	let application = root
		.children()
		.find(|node| node.has_tag_name("application"))
		.context("Android manifest has no application element")?;
	let additions = missing_permissions(root, permissions, prefix)?;
	let queries: Vec<_> = root
		.children()
		.filter(|node| node.has_tag_name("queries"))
		.collect();
	anyhow::ensure!(
		queries.len() <= 1,
		"Android manifest must contain at most one queries element"
	);
	let mut edits: Vec<Edit> = Vec::new();
	let missing_actions = queries::missing(&queries, actions, intents, prefix)?;
	let insertion = application.range().start;

	if !additions.is_empty() {
		edits.push((insertion..insertion, additions));
	}

	if let Some(edit) =
		queries::insertion(source, queries.first().copied(), insertion, missing_actions)?
	{
		edits.push(edit);
	}

	edits.sort_by_key(|edit| std::cmp::Reverse(edit.0.start));
	let mut output = source.to_owned();

	for (range, replacement) in edits {
		output.replace_range(range, &replacement);
	}

	// Validated identifiers and namespace-aware insertion must preserve XML.
	roxmltree::Document::parse(&output).context("generated Android manifest is invalid")?;
	Ok(output)
}

fn missing_permissions(
	root: roxmltree::Node<'_, '_>,
	permissions: &BTreeSet<&str>,
	prefix: &str,
) -> Result<String> {
	let declared: BTreeSet<_> = root
		.children()
		.filter(|node| node.has_tag_name("uses-permission"))
		.filter_map(|node| node.attribute((ANDROID, "name")))
		.collect();
	let mut additions = String::new();

	for permission in permissions.difference(&declared) {
		writeln!(
			additions,
			"<uses-permission {prefix}:name=\"{permission}\"/>"
		)?;
	}

	Ok(additions)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn preserves_queries_comments_and_existing_permission_without_duplicates() {
		let source = r#"<manifest xmlns:a="http://schemas.android.com/apk/res/android">
<!-- keep my comment -->
<uses-permission a:name="android.permission.CAMERA" a:maxSdkVersion="30"/>
<queries><package a:name="custom.app"/><intent><action a:name="custom.action"/></intent></queries>
<application a:label="App &amp; More"/>
</manifest>"#;
		let permissions =
			BTreeSet::from(["android.permission.CAMERA", "android.permission.INTERNET"]);
		let actions = BTreeSet::from(["custom.action", "android.intent.action.TTS_SERVICE"]);
		let output = merge(source, &permissions, &actions, &BTreeSet::new()).unwrap();
		assert!(output.contains("<!-- keep my comment -->"));
		assert!(output.contains("<package a:name=\"custom.app\"/>"));
		assert!(output.contains("a:maxSdkVersion=\"30\""));
		assert_eq!(output.matches("<queries>").count(), 1);
		assert_eq!(output.matches("android.permission.CAMERA").count(), 1);
		assert_eq!(output.matches("custom.action").count(), 1);
		assert_eq!(
			merge(&output, &permissions, &actions, &BTreeSet::new()).unwrap(),
			output
		);
	}

	#[test]
	fn merges_self_closing_queries_and_ignores_comments_as_markup() {
		let source = r#"<manifest xmlns:android="http://schemas.android.com/apk/res/android"><!-- <application fake="yes"/> --><queries /><application/></manifest>"#;
		let output = merge(
			source,
			&BTreeSet::new(),
			&BTreeSet::from(["android.intent.action.TTS_SERVICE"]),
			&BTreeSet::new(),
		)
		.unwrap();
		let document = roxmltree::Document::parse(&output).unwrap();
		assert_eq!(
			document
				.root_element()
				.children()
				.filter(|node| node.has_tag_name("queries"))
				.count(),
			1
		);
		assert_eq!(
			document
				.descendants()
				.filter(|node| node.has_tag_name("action"))
				.count(),
			1
		);
		assert!(output.contains("<!-- <application fake=\"yes\"/> -->"));
	}

	#[test]
	fn rejects_multiple_query_roots_and_malformed_xml() {
		assert!(
			merge(
				"<manifest>",
				&BTreeSet::new(),
				&BTreeSet::new(),
				&BTreeSet::new()
			)
			.is_err()
		);
		let source = r#"<manifest xmlns:android="http://schemas.android.com/apk/res/android"><queries/><queries/><application/></manifest>"#;
		assert!(merge(source, &BTreeSet::new(), &BTreeSet::new(), &BTreeSet::new()).is_err());
	}
}
