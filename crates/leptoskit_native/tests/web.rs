#![cfg(all(feature = "web", not(target_arch = "wasm32")))]

use leptos::prelude::RenderHtml;
use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_native::FontWeight;
use leptoskit_native::NativeKind;
use leptoskit_native::web::render;
use leptoskit_yoga::Dimension;

#[test]
fn renders_semantic_controls_and_escapes_user_content() {
	let element = Element::view()
		.class("p-4 md:p-8 text-slate-900")
		.child(Element::text("<script>alert('hello') & goodbye</script>"))
		.child(Element::button("Save").with_props(|props| props.enabled = Some(false)))
		.child(Element::image("/photo.png").with_props(|props| {
			props.accessibility_label = Some("A \"safe\" photo".into());
		}))
		.child(Element::text_input().with_props(|props| {
			props.value = Some("<hello> & \"world\"".into());
			props.placeholder = Some("Your name".into());
			props.accessibility_label = Some("Name".into());
		}));
	let html = render(element).to_html();

	assert!(html.contains("<div"));
	assert!(html.contains("<span"));
	assert!(html.contains("<button"));
	assert!(html.contains("type=\"button\""));
	assert!(html.contains(" disabled"));
	assert!(html.contains("<input"));
	assert!(html.contains("placeholder=\"Your name\""));
	assert!(html.contains("aria-label=\"Name\""));
	assert!(html.contains("src=\"/photo.png\""));
	assert!(html.contains("alt=\"A &quot;safe&quot; photo\""));
	assert!(html.contains("&lt;script&gt;"));
	assert!(!html.contains("<script>"));
	assert!(html.contains("value=\"&lt;hello&gt; &amp; &quot;world&quot;\""));
}

#[test]
fn emits_deterministic_scoped_styles_below_class_specificity() {
	let element = Element::view()
		.class("p-4 md:p-8")
		.with_style(|style| style.width = Dimension::Points(230.0))
		.background(Color::rgba(1, 2, 3, 255))
		.child(Element::text("Inherited").class("font-semibold"));
	let first = render(element.clone()).to_html();
	let second = render(element).to_html();

	assert_eq!(first, second);
	assert!(first.contains(":where([data-lk-style=\"lk-"));
	assert!(first.contains("width:230px;"));
	assert!(first.contains("background-color:#010203ff;"));
	assert!(first.contains("flex-direction:column;"));
	assert!(first.contains("flex-shrink:0;"));
	assert!(first.contains("box-sizing:border-box;"));
	assert!(first.contains("@media"));
	assert!(first.contains("font-weight:600"));
}

#[test]
fn classes_override_typed_bases_regardless_of_builder_order() {
	let before = Element::text("Hello")
		.font(17.0, FontWeight::Bold)
		.class("text-sm font-medium");
	let after = Element::text("Hello")
		.class("text-sm font-medium")
		.font(17.0, FontWeight::Bold);

	assert_eq!(render(before).to_html(), render(after).to_html());
}

#[test]
#[should_panic(expected = "platform-specific Custom widget `metal-view`")]
fn rejects_custom_native_widgets() {
	let _ = render(Element::new(NativeKind::Custom("metal-view")));
}

#[test]
#[should_panic(expected = "cannot render children of image")]
fn rejects_children_of_void_controls() {
	let _ = render(Element::image("/image.png").child(Element::text("Invalid child")));
}

#[test]
fn hydration_fixture_matches_server_rendering() {
	let html = render(hydration_fixture()).to_html();

	if std::env::var_os("UPDATE_LEPTOSKIT_WEB_FIXTURE").is_some() {
		std::fs::write(
			concat!(
				env!("CARGO_MANIFEST_DIR"),
				"/tests/fixtures/web_hydration.html"
			),
			&html,
		)
		.expect("write explicit fixture update");
		return;
	}

	assert_eq!(html, include_str!("fixtures/web_hydration.html"));
}

fn hydration_fixture() -> Element {
	Element::view()
		.class("p-4")
		.child(Element::text("Hydrated"))
		.child(Element::button("Activate"))
}
