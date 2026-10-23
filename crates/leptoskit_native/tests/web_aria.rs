//! Server-rendered ARIA: attributes, semantic elements, and inherited hiding.
#![cfg(all(feature = "web", not(target_arch = "wasm32")))]

use leptos::prelude::RenderHtml;
use leptoskit_native::Current;
use leptoskit_native::Element;
use leptoskit_native::HasPopup;
use leptoskit_native::Invalid;
use leptoskit_native::Live;
use leptoskit_native::Orientation;
use leptoskit_native::Role;
use leptoskit_native::Tristate;
use leptoskit_native::components::ActivityIndicator;
use leptoskit_native::components::Alert;
use leptoskit_native::components::Button;
use leptoskit_native::components::Checkbox;
use leptoskit_native::components::Heading;
use leptoskit_native::components::Image;
use leptoskit_native::components::Pressable;
use leptoskit_native::components::Status;
use leptoskit_native::components::Text;
use leptoskit_native::components::TextInput;
use leptoskit_native::components::View;
use leptoskit_native::components::helpers::Divider;
use leptoskit_native::components::helpers::VerticalDivider;
use leptoskit_native::web::render;

fn html(element: Element) -> String {
	render(element).to_html()
}

#[test]
fn heading_status_alert_and_divider_helpers_render_semantics() {
	let page = html(View().children([
		Heading(2, "Profile"),
		Status("Saved 3 items"),
		Alert("Connection lost"),
		Divider(),
		VerticalDivider(),
	]));

	assert!(page.contains("<h2"));
	assert!(page.contains("Profile<!></h2>"));
	assert!(!page.contains("role=\"heading\""));
	assert!(page.contains("role=\"status\""));
	assert!(page.contains("Saved 3 items"));
	assert!(page.contains("role=\"alert\""));
	assert!(page.contains("Connection lost"));
	assert_eq!(page.matches("<hr").count(), 2);
	assert!(page.contains("aria-orientation=\"vertical\""));
}

#[test]
fn structural_roles_render_their_html_elements_without_redundant_roles() {
	let page = html(
		View()
			.role(Role::Main)
			.child(Text("Profile").role(Role::Heading).aria_level(2))
			.child(Text("Deep").role(Role::Heading).aria_level(7))
			.child(Text("Unlevelled").role(Role::Heading))
			.child(
				View()
					.role(Role::List)
					.child(View().role(Role::ListItem).child(Text("One")))
					.child(View().role(Role::ListItem).child(Text("Two"))),
			)
			.child(View().role(Role::Navigation))
			.child(View().role(Role::Banner))
			.child(View().role(Role::ContentInfo))
			.child(View().role(Role::Complementary))
			.child(View().role(Role::Article))
			.child(View().role(Role::Region).aria_label("Orders"))
			.child(View().role(Role::Region))
			.child(View().role(Role::Search))
			.child(View().role(Role::Paragraph).child(Text("Prose")))
			.child(View().role(Role::Separator))
			.child(View().role(Role::Separator).child(Text("Not a rule"))),
	);

	assert!(page.contains("<main"));
	assert!(page.contains("<h2"));
	assert!(page.contains("Profile<!></h2>"));
	assert!(page.contains("role=\"heading\" aria-level=\"7\""));
	assert!(page.contains("<span") && page.contains("role=\"heading\">Unlevelled"));
	assert!(!page.contains("aria-level=\"2\""));
	assert!(page.contains("<ul"));
	assert_eq!(page.matches("<li").count(), 2);
	assert!(page.contains("list-style:none;"));
	assert!(page.contains("<nav"));
	assert!(page.contains("<header"));
	assert!(page.contains("<footer"));
	assert!(page.contains("<aside"));
	assert!(page.contains("<article"));
	assert!(page.contains("<section") && page.contains("aria-label=\"Orders\""));
	assert!(page.contains("role=\"region\""));
	assert!(page.contains("<search"));
	assert!(page.contains("<p ") && page.contains(">Prose<!>"));
	assert!(page.contains("<hr"));
	assert!(page.contains("role=\"separator\""));
	assert!(!page.contains("role=\"main\""));
	assert!(!page.contains("role=\"list\""));
	assert!(!page.contains("role=\"navigation\""));
}

#[test]
fn control_roles_on_containers_join_the_tab_order_and_native_controls_do_not_repeat_roles() {
	let page = html(
		View()
			.child(
				View()
					.role(Role::Button)
					.aria_label("Custom")
					.on_press(|| {}),
			)
			.child(View().role(Role::Tab).aria_selected(true).tab_index(-1))
			.child(Pressable("Docs").role(Role::Link).child(Text("Docs")))
			.child(Button("Save").role(Role::Button))
			.child(Checkbox(false).aria_label("Agree").role(Role::Switch))
			.child(
				Checkbox(false)
					.aria_label("Mixed")
					.aria_checked(Tristate::Mixed),
			)
			.child(Text("Plain").role(Role::Group)),
	);

	assert!(page.contains("role=\"button\" tabindex=\"0\""));
	assert!(page.contains("role=\"tab\" aria-selected=\"true\" tabindex=\"-1\""));
	assert!(page.contains("role=\"link\""));
	assert!(!page.contains("role=\"link\" tabindex"));
	assert_eq!(page.matches("role=\"button\"").count(), 1);
	assert!(page.contains("role=\"switch\""));
	assert!(page.contains("aria-checked=\"mixed\""));
	assert!(page.contains("role=\"group\""));
	assert!(!page.contains("role=\"group\" tabindex"));
}

#[test]
fn every_aria_property_renders_as_its_attribute() {
	let page = html(
		View()
			.child(Text("Email").id("email-label"))
			.child(Text("Required").id("email-help"))
			.child(
				TextInput()
					.id("email")
					.aria_labelled_by(["email-label"])
					.aria_described_by(["email-help", "email-label"])
					.aria_description("Your work address")
					.aria_required(true)
					.aria_invalid(Invalid::Spelling)
					.aria_read_only(false)
					.aria_multiline(false)
					.aria_has_popup(HasPopup::ListBox)
					.aria_controls(["suggestions"])
					.aria_expanded(false)
					.aria_busy(true)
					.aria_current(Current::Step)
					.aria_attr("aria-autocomplete", "list"),
			)
			.child(
				View()
					.id("suggestions")
					.role(Role::ListBox)
					.aria_orientation(Orientation::Vertical)
					.aria_role_description("suggestion list")
					.aria_modal(false)
					.aria_pos_in_set(1)
					.aria_set_size(4)
					.aria_level(3),
			)
			.child(
				View()
					.role(Role::Slider)
					.aria_label("Volume")
					.aria_value_now(0.5)
					.aria_value_min(0.0)
					.aria_value_max(1.0)
					.aria_value_text("50 percent")
					.aria_pressed(Tristate::False),
			)
			.child(
				View()
					.aria_live(Live::Assertive)
					.aria_atomic(true)
					.child(Text("Saved")),
			),
	);

	for attribute in [
		"id=\"email-label\"",
		"id=\"email\"",
		"aria-labelledby=\"email-label\"",
		"aria-describedby=\"email-help email-label\"",
		"aria-description=\"Your work address\"",
		"aria-required=\"true\"",
		"aria-invalid=\"spelling\"",
		"aria-readonly=\"false\"",
		"aria-multiline=\"false\"",
		"aria-haspopup=\"listbox\"",
		"aria-controls=\"suggestions\"",
		"aria-expanded=\"false\"",
		"aria-busy=\"true\"",
		"aria-current=\"step\"",
		"aria-autocomplete=\"list\"",
		"role=\"listbox\"",
		"aria-orientation=\"vertical\"",
		"aria-roledescription=\"suggestion list\"",
		"aria-modal=\"false\"",
		"aria-posinset=\"1\"",
		"aria-setsize=\"4\"",
		"aria-level=\"3\"",
		"role=\"slider\"",
		"aria-valuenow=\"0.5\"",
		"aria-valuemin=\"0\"",
		"aria-valuemax=\"1\"",
		"aria-valuetext=\"50 percent\"",
		"aria-pressed=\"false\"",
		"aria-live=\"assertive\"",
		"aria-atomic=\"true\"",
	] {
		assert!(page.contains(attribute), "missing {attribute} in {page}");
	}
}

#[test]
fn hidden_semantics_inherit_through_the_markup_but_visible_text_stays() {
	let page = html(
		View()
			.aria_hidden(true)
			.child(View().child(Text("Decorative")))
			.child(Text("Also decorative")),
	);

	assert_eq!(page.matches("aria-hidden=\"true\"").count(), 4);
	assert!(page.contains("Decorative"));
	assert!(!page.contains("aria-hidden=\"false\""));

	let shown = html(View().aria_hidden(false).child(Text("Visible")));
	assert!(!shown.contains("aria-hidden"));
}

#[test]
fn activity_indicators_are_named_loading_unless_the_app_names_them() {
	let default = html(ActivityIndicator(true));
	assert!(default.contains("aria-label=\"Loading\""));
	assert!(default.contains("role=\"progressbar\""));
	assert_eq!(default.matches("role=\"progressbar\"").count(), 1);

	let named = html(ActivityIndicator(true).aria_label("Syncing photos"));
	assert!(named.contains("aria-label=\"Syncing photos\""));
	assert!(!named.contains("Loading"));
}

#[test]
fn images_are_decorative_until_named() {
	let named = html(Image("/chart.png").aria_label("Quarterly sales"));
	assert!(named.contains("alt=\"Quarterly sales\""));
	assert!(named.contains("aria-label=\"Quarterly sales\""));

	let decorative = html(Image("/divider.png"));
	assert!(decorative.contains("alt=\"\""));
	assert!(!decorative.contains("aria-label"));

	let presentational = html(Image("/divider.png").role(Role::Presentation));
	assert!(presentational.contains("role=\"presentation\""));
}
