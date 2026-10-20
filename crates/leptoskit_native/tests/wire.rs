//! The JSON wire form of the props, which code-pushed bundles depend on.
#![cfg(feature = "serde")]

use leptoskit_native::Color;
use leptoskit_native::FontWeight;
use leptoskit_native::NativeProps;
use leptoskit_native::TextAlign;

#[test]
fn props_written_before_the_visual_fields_still_decode() {
	// Exactly what a bundle built against the first wire format sends.
	let json = r#"{"text":"Count: 1","color":255,"background":null,"enabled":true,"extra":{}}"#;

	let props: NativeProps = serde_json::from_str(json).expect("old props decode");

	assert_eq!(props.text.as_deref(), Some("Count: 1"));
	assert_eq!(props.color, Some(Color(255)));
	assert_eq!(props.enabled, Some(true));
	assert_eq!(props.corner_radius, None);
	assert_eq!(props.border_width, None);
	assert_eq!(props.border_color, None);
	assert_eq!(props.opacity, None);
	assert_eq!(props.font_size, None);
	assert_eq!(props.font_weight, None);
	assert_eq!(props.text_align, None);
}

#[test]
fn visual_fields_round_trip_with_snake_case_names() {
	let props = NativeProps {
		corner_radius: Some(12.0),
		border_width: Some(1.0),
		border_color: Color::hex("#16111f"),
		opacity: Some(0.5),
		font_size: Some(17.0),
		font_weight: Some(FontWeight::Semibold),
		text_align: Some(TextAlign::Center),
		..NativeProps::text("Hello")
	};

	let json = serde_json::to_value(&props).expect("encode");
	assert_eq!(json["font_weight"], "semibold");
	assert_eq!(json["text_align"], "center");
	let decoded: NativeProps = serde_json::from_value(json).expect("decode");
	assert_eq!(decoded, props);
}

#[test]
fn visibility_and_clipping_preserve_wire_compatibility() {
	let absent: NativeProps = serde_json::from_str("{}").expect("old empty props");
	assert_eq!(absent.hidden, None);
	assert_eq!(absent.clip, None);
	let encoded = serde_json::to_value(&absent).expect("serialize");
	assert!(encoded.get("hidden").is_none());
	assert!(encoded.get("clip").is_none());

	for flag in [false, true] {
		let props = NativeProps {
			hidden: Some(flag),
			clip: Some(flag),
			..NativeProps::default()
		};
		let encoded = serde_json::to_string(&props).expect("serialize explicit flags");
		assert_eq!(
			serde_json::from_str::<NativeProps>(&encoded).expect("decode"),
			props
		);
	}
}
