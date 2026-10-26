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

#[test]
fn mutation_kinds_use_canonical_names_and_read_legacy_kind_payloads() {
	use leptoskit_native::Element;
	use leptoskit_native::HeadlessBackend;
	use leptoskit_native::Mutation;
	use leptoskit_native::NativeKind;
	use leptoskit_native::NativeSurface;

	for (kind, legacy) in [
		(NativeKind::Button, serde_json::json!("Button")),
		(NativeKind::Checkbox, serde_json::json!("Checkbox")),
		(
			NativeKind::Custom("canvas"),
			serde_json::json!({ "Custom": "canvas" }),
		),
	] {
		let mut surface = NativeSurface::new(HeadlessBackend::new());
		surface.set_root(&Element::new(kind));
		let create = surface
			.backend()
			.log()
			.iter()
			.find(|mutation| matches!(mutation, Mutation::Create { .. }))
			.unwrap();
		let mut json = serde_json::to_value(create).unwrap();
		assert_eq!(json["Create"]["kind"], kind.wire_name());
		assert_eq!(
			serde_json::from_value::<Mutation>(json.clone()).unwrap(),
			*create
		);
		json["Create"]["kind"] = legacy;
		assert_eq!(serde_json::from_value::<Mutation>(json).unwrap(), *create);
	}
}

#[test]
fn drawings_travel_as_canonical_path_data() {
	use leptoskit_native::Drawing;
	use leptoskit_native::PathData;
	use leptoskit_native::Stroke;
	use leptoskit_native::VectorPath;
	use leptoskit_native::VectorScaling;
	use leptoskit_native::ViewBox;

	let ink = Color::rgba(0x11, 0x11, 0x11, 0xff);
	let drawing = Drawing::new(ViewBox::new(0.0, 0.0, 24.0, 12.0).expect("a valid view box"))
		.scaling(VectorScaling::Stretch)
		.path(
			VectorPath::new(PathData::parse("m1 1 h10 q2 0 2 2").expect("valid data"))
				.stroke(Stroke::new(ink, 1.5).expect("a positive width").round()),
		)
		.path(VectorPath::new(PathData::parse("M0 0 L4 0 L0 4 Z").expect("valid data")).fill(ink));
	let props = NativeProps {
		drawing: Some(Box::new(drawing)),
		..NativeProps::default()
	};

	let json = serde_json::to_value(&props).expect("props serialize");

	assert_eq!(
		json["drawing"],
		serde_json::json!({
			"view_box": { "min_x": 0.0, "min_y": 0.0, "width": 24.0, "height": 12.0 },
			"scaling": "stretch",
			"paths": [
				{
					"data": "M 1 1 L 11 1 Q 13 1 13 3",
					"stroke": { "color": 0x1111_11ff, "width": 1.5, "cap": "round", "join": "round" }
				},
				{ "data": "M 0 0 L 4 0 L 0 4 Z", "fill": 0x1111_11ff }
			]
		})
	);
	assert_eq!(
		serde_json::from_value::<NativeProps>(json).expect("props decode"),
		props
	);
	assert!(
		serde_json::to_value(NativeProps::default())
			.expect("props serialize")
			.get("drawing")
			.is_none(),
		"props without a drawing keep their wire form"
	);
}

#[test]
fn invalid_drawings_are_rejected_on_decode() {
	for drawing in [
		serde_json::json!({ "view_box": { "min_x": 0, "min_y": 0, "width": 0, "height": 1 } }),
		serde_json::json!({
			"view_box": { "min_x": 0, "min_y": 0, "width": 1, "height": 1 },
			"paths": [{ "data": "L 0 0" }]
		}),
		serde_json::json!({
			"view_box": { "min_x": 0, "min_y": 0, "width": 1, "height": 1 },
			"paths": [{ "data": "M 0 0", "stroke": { "color": 255, "width": -1 } }]
		}),
	] {
		assert!(
			serde_json::from_value::<NativeProps>(serde_json::json!({ "drawing": drawing }))
				.is_err()
		);
	}
}
