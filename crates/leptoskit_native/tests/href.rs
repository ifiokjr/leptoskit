//! Navigation targets are validated regardless of how props were supplied.

#[path = "support/href.rs"]
mod targets;

use leptoskit_native::NativeProps;
use leptoskit_native::PropValue;
use leptoskit_native::components::Pressable;

#[test]
fn pressable_href_allows_navigation_and_rejects_executable_targets() {
	for href in targets::VALID_HREFS {
		assert_eq!(Pressable("Open").href(*href).props.href(), Some(*href));
	}

	for href in targets::INVALID_HREFS {
		assert_eq!(Pressable("Open").href(*href).props.href(), None, "{href:?}");
	}
}

#[test]
fn direct_props_cannot_bypass_the_navigation_policy() {
	let mut props = NativeProps::default();
	props.extra.insert(
		NativeProps::HREF.to_owned(),
		PropValue::Str("javascript:alert(1)".to_owned()),
	);
	assert_eq!(props.href(), None);
}

#[cfg(feature = "serde")]
#[test]
fn deserialized_props_cannot_bypass_the_navigation_policy() {
	let props: NativeProps = serde_json::from_value(serde_json::json!({
		"extra": {"href": {"Str": "javascript:alert(1)"}}
	}))
	.expect("props decode");
	assert_eq!(props.href(), None);
}
