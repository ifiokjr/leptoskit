//! Motion through the web renderer: which CSS a tree with motion emits, and
//! that motion never changes what native platforms build.
#![cfg(all(feature = "web", not(target_arch = "wasm32")))]

use std::time::Duration;

use leptos::prelude::RenderHtml;
use leptoskit_native::Element;
use leptoskit_native::NativeKind;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_native::flatten::is_layout_only;
use leptoskit_native::motion::Motion;
use leptoskit_native::motion::Pose;
use leptoskit_native::motion::Spring;
use leptoskit_native::motion::ViewPhase;
use leptoskit_native::motion::ViewRange;

fn html(element: Element) -> String {
	leptoskit_native::web::render(element).to_html()
}

fn rise() -> Motion {
	Motion::enter(Pose::new().y(16.0).opacity(0.0))
}

/// Each `animation:` declaration's delay, in document order.
fn delays(html: &str) -> Vec<String> {
	html.split("animation:")
		.skip(1)
		.map(|rule| {
			rule.split_whitespace()
				.nth(3)
				.expect("a timed animation names its delay")
				.to_owned()
		})
		.collect()
}

#[test]
fn staggered_children_start_one_step_after_another() {
	let list = View()
		.stagger(Duration::from_millis(70))
		.children((0..3).map(|index| Text(format!("Item {index}")).motion(rise())));
	let html = html(list);

	let mut found = delays(&html);
	found.sort();
	assert_eq!(found, ["0ms", "140ms", "70ms"]);
	assert_eq!(
		html.matches("@keyframes ").count(),
		1,
		"equal motions share one keyframes rule: {html}"
	);
}

#[test]
fn a_childs_own_delay_adds_to_its_stagger() {
	let list = View()
		.stagger(Duration::from_millis(50))
		.child(Text("First").motion(rise().delay(Duration::from_millis(200))))
		.child(Text("Second").motion(rise().delay(Duration::from_millis(200))));
	let mut found = delays(&html(list));
	found.sort();
	assert_eq!(found, ["200ms", "250ms"]);
}

/// The value of the first `attribute="…"` in `html`.
fn attribute(html: &str, attribute: &str) -> Option<String> {
	html.split(&format!("{attribute}=\""))
		.nth(1)
		.and_then(|rest| rest.split('"').next())
		.map(str::to_owned)
}

#[test]
fn motion_has_a_scope_of_its_own_and_leaves_the_style_scope_alone() {
	let plain = html(Text("Hello").class("p-4"));
	let moving = html(Text("Hello").class("p-4").motion(rise()));

	assert_eq!(
		attribute(&plain, "data-lk-style"),
		attribute(&moving, "data-lk-style"),
		"moving elements share their look's sheet"
	);
	assert!(!plain.contains("animation"), "no motion, no animation");
	assert!(attribute(&plain, "data-lk-motion").is_none());

	let scope = attribute(&moving, "data-lk-motion").expect("a motion scope");
	assert!(moving.contains(&format!(
		"@media (prefers-reduced-motion:no-preference){{[data-lk-motion=\"{scope}\"]{{animation:"
	)));
	// Content stays in the markup: motion only styles it.
	assert!(moving.contains("Hello"), "{moving}");
}

#[test]
fn equal_motions_share_one_motion_scope() {
	let html = html(
		View()
			.child(Text("One").motion(rise()))
			.child(Text("Two").class("p-2").motion(rise())),
	);
	assert_eq!(html.matches("{animation:").count(), 1, "{html}");
}

#[test]
fn followers_bind_to_a_named_stage() {
	let stage = View().timeline("stage").child(
		Text("Act one").motion(
			Motion::keyframes([
				(0.0, Pose::new().opacity(0.0)),
				(0.5, Pose::REST),
				(1.0, Pose::new().opacity(0.0)),
			])
			.follow("stage", ViewRange::during(ViewPhase::Contain, 0.0, 0.5)),
		),
	);
	let html = html(stage);

	assert!(html.contains("view-timeline:--lk-stage block"));
	assert!(html.contains("animation-timeline:--lk-stage;animation-range:contain 0% contain 50%"));
	assert!(html.contains("@supports (animation-timeline:view())"));
}

#[test]
fn springs_reach_the_page_as_linear_easing() {
	let html = html(Text("Boing").motion(rise().spring(Spring::BOUNCY)));
	let duration = Spring::BOUNCY.settle_duration().as_millis();

	assert!(html.contains(&format!(" {duration}ms linear(0,")), "{html}");
}

#[test]
fn motion_never_changes_what_native_platforms_build() {
	let container = View().stagger(Duration::from_millis(40)).timeline("stage");
	assert!(is_layout_only(NativeKind::View, &container.props));

	let moving = View().motion(rise());
	assert!(is_layout_only(NativeKind::View, &moving.props));
}

#[cfg(feature = "serde")]
#[test]
fn native_hosts_never_receive_motion() {
	let element = Text("Hi").motion(rise()).timeline("stage");
	let wire = serde_json::to_value(&element.props).unwrap();

	assert_eq!(wire, serde_json::json!({ "text": "Hi" }));
}
