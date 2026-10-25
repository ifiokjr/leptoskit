//! Scheme-only visibility must not be confused with action-only or narrower
//! queries.
use super::*;

const VIEW: &str = "android.intent.action.VIEW";
const SCHEME: &str = "solana-wallet";

fn generated(existing: &str) -> String {
	let source = format!(
		"<manifest xmlns:a=\"{ANDROID}\"><queries>{existing}</queries><application/></manifest>"
	);
	merge(
		&source,
		&BTreeSet::new(),
		&BTreeSet::new(),
		&BTreeSet::from([(VIEW, SCHEME)]),
	)
	.unwrap()
}

#[test]
fn action_only_queries_do_not_hide_the_required_wallet_scheme() {
	let output = generated(&format!("<intent><action a:name=\"{VIEW}\"/></intent>"));
	let document = roxmltree::Document::parse(&output).unwrap();
	let data: Vec<_> = document
		.descendants()
		.filter(|node| node.has_tag_name("data"))
		.collect();
	assert_eq!(data.len(), 1);
	assert_eq!(data[0].attribute((ANDROID, "scheme")), Some(SCHEME));
	assert!(data[0].parent().unwrap().children().any(|node| {
		node.has_tag_name("action") && node.attribute((ANDROID, "name")) == Some(VIEW)
	}));
	assert_eq!(
		merge(
			&output,
			&BTreeSet::new(),
			&BTreeSet::new(),
			&BTreeSet::from([(VIEW, SCHEME)])
		)
		.unwrap(),
		output
	);
}

#[test]
fn a_matching_query_preserves_app_owned_xml_byte_for_byte() {
	let intent =
		format!("<intent><data a:scheme=\"{SCHEME}\"/><action a:name=\"{VIEW}\"/></intent>");
	let output = generated(&intent);
	assert_eq!(output.matches("<intent>").count(), 1);
	assert!(output.contains(&intent));
}

#[test]
fn host_constraints_categories_and_other_schemes_do_not_cover_the_requested_query() {
	for data in [
		format!("<data a:scheme=\"{SCHEME}\" a:host=\"wallet.example\"/>"),
		format!(
			"<data a:scheme=\"{SCHEME}\"/><category a:name=\"android.intent.category.DEFAULT\"/>"
		),
		"<data a:scheme=\"different-wallet\"/>".into(),
	] {
		let existing = format!("<intent><action a:name=\"{VIEW}\"/>{data}</intent>");
		let output = generated(&existing);
		let document = roxmltree::Document::parse(&output).unwrap();
		assert_eq!(
			document
				.descendants()
				.filter(|node| node.has_tag_name("intent"))
				.count(),
			2
		);
		assert!(output.contains(&existing));
	}
}
