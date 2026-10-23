//! Color and wire contracts stay stable while their private layout changes.

use leptoskit_native::Color;

#[test]
fn color_channels_and_hex_forms_preserve_rgba_bytes() {
	let color = Color::rgba(0x12, 0x34, 0x56, 0x78);
	assert_eq!(color, Color(0x1234_5678));
	assert_eq!(
		(color.red(), color.green(), color.blue(), color.alpha()),
		(0x12, 0x34, 0x56, 0x78)
	);
	assert_eq!(Color::hex("#12345678"), Some(color));
	assert_eq!(Color::hex("#123456"), Some(Color(0x1234_56ff)));
	assert_eq!(Color::hex("#ABCDEF"), Some(Color(0xabcd_efff)));

	for invalid in ["123456", "#12345", "#1234567", "#123456789", "#abcdefzz"] {
		assert_eq!(Color::hex(invalid), None, "{invalid}");
	}
}

#[cfg(feature = "serde")]
#[test]
fn every_kind_retains_canonical_and_legacy_wire_names() {
	use leptoskit_native::NativeKind;
	for (kind, legacy) in [
		(NativeKind::View, "View"),
		(NativeKind::Text, "Text"),
		(NativeKind::TextRun, "TextRun"),
		(NativeKind::Image, "Image"),
		(NativeKind::ScrollView, "ScrollView"),
		(NativeKind::TextInput, "TextInput"),
		(NativeKind::Button, "Button"),
		(NativeKind::Switch, "Switch"),
		(NativeKind::Checkbox, "Checkbox"),
		(NativeKind::Radio, "Radio"),
		(NativeKind::Slider, "Slider"),
		(
			NativeKind::LinearProgressIndicator,
			"LinearProgressIndicator",
		),
		(NativeKind::ActivityIndicator, "ActivityIndicator"),
		(NativeKind::PasswordInput, "PasswordInput"),
		(NativeKind::Pressable, "Pressable"),
	] {
		let canonical = serde_json::json!(kind.wire_name());
		assert_eq!(
			serde_json::to_value(kind).expect("canonical encoding"),
			canonical
		);
		assert_eq!(
			serde_json::from_value::<NativeKind>(canonical).expect("canonical decoding"),
			kind
		);
		assert_eq!(
			serde_json::from_value::<NativeKind>(serde_json::json!(legacy))
				.expect("legacy decoding"),
			kind
		);
	}
}

#[cfg(feature = "serde")]
#[test]
fn custom_kind_strings_and_legacy_map_validation_are_unchanged() {
	use leptoskit_native::NativeKind;

	for value in [
		serde_json::json!("canvas"),
		serde_json::json!({"Custom":"canvas"}),
	] {
		assert_eq!(
			serde_json::from_value::<NativeKind>(value).expect("custom kind"),
			NativeKind::Custom("canvas")
		);
	}

	for (value, message) in [
		(serde_json::json!({}), "empty legacy custom kind"),
		(
			serde_json::json!({"Wrong":"canvas"}),
			"expected a single legacy Custom kind",
		),
		(
			serde_json::json!({"Custom":"canvas", "Other":"extra"}),
			"expected a single legacy Custom kind",
		),
	] {
		let error = serde_json::from_value::<NativeKind>(value).expect_err("invalid custom map");
		assert!(error.to_string().contains(message), "{error}");
	}
}
