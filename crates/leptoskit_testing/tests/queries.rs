//! Testing Library queries over the headless widget tester.

use leptoskit_native::Element;
use leptoskit_native::NativeKind;
use leptoskit_native::Role;
use leptoskit_testing::Finder;
use leptoskit_testing::Queries;
use leptoskit_testing::Widget;
use leptoskit_testing::WidgetTester;

fn inbox() -> WidgetTester {
	WidgetTester::new(
		|| {
			Element::view()
				.child(Element::text("Inbox"))
				.child(Element::button("Archive").test_id("archive"))
				.child(Element::button("Delete"))
				.child(Element::button("Delete"))
				.child(Element::button("x").accessibility_label("Close"))
				.child(Element::text("Hidden draft").with_props(|props| props.hidden = Some(true)))
		},
		390.0,
		844.0,
	)
	.expect("the inbox mounts")
}

#[test]
fn get_returns_the_single_match_and_rejects_none_or_several() {
	let tester = inbox();

	let inbox = tester.get(&Finder::text("Inbox")).expect("one match");
	assert_eq!(inbox.kind, NativeKind::Text);

	let missing = tester.get(&Finder::text("Spam")).unwrap_err();
	assert_eq!(
		missing.reason,
		"expected one match for Text(\"Spam\"), found 0"
	);
	assert_eq!(missing.tree, tester.tree());

	let several = tester.get(&Finder::text("Delete")).unwrap_err();
	assert_eq!(
		several.reason,
		"expected one match for Text(\"Delete\"), found 2"
	);
	assert!(several.to_string().contains("button text=Some(\"Delete\")"));
}

#[test]
fn get_all_requires_at_least_one_match() {
	let tester = inbox();

	assert_eq!(tester.get_all(&Finder::text("Delete")).unwrap().len(), 2);
	assert_eq!(tester.get_all(&Finder::text("Inbox")).unwrap().len(), 1);

	let missing = tester.get_all(&Finder::text("Spam")).unwrap_err();
	assert_eq!(
		missing.reason,
		"expected at least one match for Text(\"Spam\"), found 0"
	);
	assert_eq!(missing.tree, tester.tree());
}

#[test]
fn query_allows_absence_but_not_ambiguity() {
	let tester = inbox();

	assert!(tester.query(&Finder::text("Spam")).unwrap().is_none());
	assert!(tester.query(&Finder::text("Inbox")).unwrap().is_some());

	let several = tester.query(&Finder::text("Delete")).unwrap_err();
	assert_eq!(
		several.reason,
		"expected at most one match for Text(\"Delete\"), found 2"
	);
}

#[test]
fn query_all_lists_matches_in_tree_order_and_never_fails() {
	let tester = inbox();

	assert_eq!(
		tester.query_all(&Finder::text("Spam")),
		Vec::<Widget>::new()
	);
	let buttons = tester.query_all_by_role(Role::Button, "Delete");
	assert_eq!(buttons.len(), 2);
	assert!(
		buttons
			.iter()
			.all(|button| button.props.text.as_deref() == Some("Delete"))
	);
	assert!(buttons[0].frame.y < buttons[1].frame.y);
}

#[test]
fn hidden_widgets_never_match() {
	let tester = inbox();

	assert!(tester.query_by_text("Hidden draft").unwrap().is_none());
	assert!(tester.tree().contains("Hidden draft"));
}

#[test]
fn sugar_builds_each_finder() {
	let tester = inbox();

	assert_eq!(
		tester.get_by_text("Inbox").unwrap(),
		tester.find(&Finder::text("Inbox")).unwrap()
	);
	assert_eq!(
		tester.get_by_label("Close").unwrap().props.text.as_deref(),
		Some("x")
	);
	assert!(tester.get_by_role(Role::Button, "Close").is_ok());
	assert!(
		tester.get_by_role(Role::Button, "x").is_err(),
		"a label replaces the text as the accessible name"
	);
	assert_eq!(
		tester
			.get_by_test_id("archive")
			.unwrap()
			.props
			.text
			.as_deref(),
		Some("Archive")
	);
	assert_eq!(tester.get_all_by_text("Delete").unwrap().len(), 2);
	assert!(tester.get_all_by_label("Open").is_err());
	assert!(tester.query_by_test_id("missing").unwrap().is_none());

	assert!(tester.query_by_role(Role::Button, "Delete").is_err());
	assert_eq!(tester.query_all_by_label("Close").len(), 1);
	assert_eq!(tester.query_all_by_test_id("archive").len(), 1);
	assert!(tester.get_all_by_role(Role::Heading, "Inbox").is_err());
	assert!(tester.get_all_by_test_id("archive").is_ok());
	assert!(tester.query_by_label("Close").unwrap().is_some());
	assert!(tester.query_by_text("Inbox").unwrap().is_some());
	assert_eq!(tester.query_all_by_text("Delete").len(), 2);
}
