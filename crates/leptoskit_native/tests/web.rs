#![cfg(all(feature = "web", not(target_arch = "wasm32")))]

use leptos::prelude::RenderHtml;
use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_native::FontWeight;
use leptoskit_native::NativeKind;
use leptoskit_native::components::Pressable;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_native::web::render;
use leptoskit_yoga::Dimension;

#[test]
fn rich_pressables_render_semantic_buttons_with_an_explicit_action_name() {
	let html = render(
		Pressable("Open <profile>")
			.class("p-4 rounded-xl")
			.child(View().child(Text("Visible profile content"))),
	)
	.to_html();
	assert!(html.contains("<button"));
	assert!(html.contains("type=\"button\""));
	assert!(html.contains("aria-label=\"Open &lt;profile&gt;\""));
	assert!(html.contains("Visible profile content"));
	assert!(!html.contains(">Open &lt;profile&gt;"));
}

#[test]
#[should_panic(expected = "pressable requires a non-whitespace accessibility label")]
fn rich_pressables_reject_an_empty_accessible_name_before_rendering() {
	let _ = render(Pressable(" \t").child(Text("Content")));
}

#[test]
#[should_panic(expected = "pressable cannot contain interactive view descendants")]
fn rich_pressables_reject_hidden_interaction_in_decorative_content() {
	let _ = render(Pressable("Action").child(View().on_press(|| {})));
}

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
#[should_panic(expected = "image is a leaf control and cannot contain children")]
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

#[test]
fn new_controls_render_semantic_state_without_browser_only_radio_names() {
	use leptoskit_native::components::*;
	let html = render(control_fixture()).to_html();

	assert!(html.contains("type=\"checkbox\" role=\"switch\""));
	assert!(html.contains("type=\"radio\""));
	assert!(!html.contains("name=\"lk-radio"));
	assert!(html.contains("type=\"range\""));
	assert!(html.contains("min=\"0\" max=\"10\" step=\"2\" value=\"4\""));
	assert!(html.contains("<progress max=\"1\" value=\"0.4\""));
	assert!(html.contains("role=\"progressbar\" aria-busy=\"true\""));
	assert!(html.contains("type=\"password\""));
	assert!(html.contains("appearance:auto;"));
	assert!(html.contains("@keyframes lk-spinner"));
	assert_eq!(html, render(control_fixture()).to_html());
	assert!(
		!render(LinearProgressIndicator(Progress::Indeterminate))
			.to_html()
			.contains("<progress max=\"1\" value=")
	);
}

#[test]
fn controls_hydration_fixture_matches_server_rendering() {
	let html = render(control_fixture()).to_html();

	if std::env::var_os("UPDATE_LEPTOSKIT_WEB_FIXTURE").is_some() {
		std::fs::write(
			concat!(
				env!("CARGO_MANIFEST_DIR"),
				"/tests/fixtures/web_controls_hydration.html"
			),
			&html,
		)
		.expect("write explicit controls fixture update");
		return;
	}

	assert_eq!(html, include_str!("fixtures/web_controls_hydration.html"));
}

fn control_fixture() -> Element {
	use leptoskit_native::components::*;
	Element::view()
		.child(Switch(true).accessibility_label("Switch"))
		.child(Checkbox(false).accessibility_label("Checkbox"))
		.child(Radio(RadioGroup::new("choice").expect("group"), true).accessibility_label("Radio"))
		.child(
			Slider(SliderState::new(0.0, 10.0, 4.0, Some(2.0)).expect("slider"))
				.accessibility_label("Slider"),
		)
		.child(
			LinearProgressIndicator(Progress::Determinate(
				ProgressValue::new(0.4).expect("progress"),
			))
			.accessibility_label("Progress"),
		)
		.child(ActivityIndicator(true).accessibility_label("Loading"))
		.child(
			PasswordInput()
				.value("secret")
				.accessibility_label("Password"),
		)
}

#[test]
fn visibility_utilities_override_hidden_without_changing_enabled_state() {
	use leptoskit_native::StyleContext;
	use leptoskit_native::components::Checkbox;
	use leptoskit_yoga::Size;

	for classes in ["flex", "md:flex"] {
		let element = Checkbox(false)
			.with_props(|props| props.hidden = Some(true))
			.class(classes);
		let wide = StyleContext {
			viewport: Size::new(900.0, 400.0),
			..StyleContext::default()
		};
		let resolved = element.resolved(&wide);
		let html = render(element).to_html();
		let input = html.split("<input").nth(1).expect("semantic checkbox");

		assert_eq!(resolved.props.hidden, Some(false));
		assert_ne!(resolved.props.enabled, Some(false));
		assert!(
			html.contains("display:none;"),
			"hidden remains a low-specificity CSS base"
		);
		assert!(html.contains("display:flex"));
		assert!(!input.contains(" disabled"));
		assert!(!input.contains("aria-disabled"));
		assert!(!input.contains("data-lk-hidden"));
	}

	let disabled = Element::view()
		.enabled(false)
		.with_props(|props| props.hidden = Some(true))
		.class("flex")
		.child(Checkbox(false).enabled(true));
	let html = render(disabled).to_html();
	let input = html.split("<input").nth(1).expect("disabled checkbox");
	assert!(input.contains(" disabled"));
	assert!(input.contains("aria-disabled=\"true\""));
}

#[test]
fn radio_group_names_are_valid_before_server_rendering_and_hydration() {
	use leptoskit_native::components::*;

	for invalid in ["a\0b", "a\rb", "a\nb", "a\r\nb", "a\tb"] {
		assert!(RadioGroup::new(invalid).is_err());
	}

	let html = render(View().children([
		Radio(RadioGroup::new("a�b").unwrap(), true).accessibility_label("Replacement"),
		Radio(RadioGroup::new("aéb").unwrap(), true).accessibility_label("Unicode"),
	]))
	.to_html();
	assert!(html.contains("data-lk-radio-group=\"a�b\""));
	assert!(html.contains("data-lk-radio-group=\"aéb\""));

	if std::env::var_os("UPDATE_LEPTOSKIT_WEB_FIXTURE").is_some() {
		std::fs::write(
			concat!(
				env!("CARGO_MANIFEST_DIR"),
				"/tests/fixtures/web_radio_unicode_hydration.html"
			),
			&html,
		)
		.unwrap();
		return;
	}

	assert_eq!(
		html,
		include_str!("fixtures/web_radio_unicode_hydration.html")
	);
}

#[test]
fn pressables_with_an_href_render_as_links_and_disabled_links_lose_their_target() {
	let html = render(
		View().children([
			Pressable("Open note").href("/notes/42").child(Text("Open")),
			Pressable("Closed")
				.href("/closed")
				.enabled(false)
				.child(Text("Closed")),
		]),
	)
	.to_html();
	assert!(html.contains("<a href=\"/notes/42\""), "{html}");
	assert!(html.contains("aria-label=\"Open note\""), "{html}");
	assert!(!html.contains("href=\"/closed\""), "{html}");
	assert!(html.contains("aria-disabled=\"true\""), "{html}");
	assert!(!html.contains("<button"), "{html}");
}
