//! The web adapter's CSS for every Tailwind declaration family.
//!
//! Native backends apply declarations to Yoga styles and props; the web
//! adapter emits them as CSS. Each case pins the property and value a class
//! produces, so the browser renders the same layout the native backends do.

use leptoskit_native::Tailwind;

fn css(class: &str) -> String {
	Tailwind::parse(class)
		.unwrap_or_else(|error| panic!("`{class}` parses: {error:?}"))
		.css()
}

#[track_caller]
fn assert_css(class: &str, expected: &[&str]) {
	let css = css(class);

	for declaration in expected {
		// Declarations must start at a boundary, so `top:` cannot match inside
		// `margin-top:`. At-rule wrappers are matched as written.
		let found = if declaration.starts_with('@') {
			css.contains(declaration)
		} else {
			css.contains(&format!("{{{declaration}")) || css.contains(&format!(";{declaration}"))
		};
		assert!(found, "`{class}` emits `{declaration}`; actual CSS: {css}");
	}
}

#[test]
fn sizing_and_edges() {
	assert_css("min-h-4", &["min-height:1rem;"]);
	assert_css("max-w-none", &["max-width:none;"]);
	assert_css("m-4", &["margin-top:1rem;", "margin-left:1rem;"]);
	assert_css("mt-2", &["margin-top:0.5rem;"]);
	assert_css("top-0", &["top:0rem;"]);
	assert_css("inset-x-2", &["left:0.5rem;", "right:0.5rem;"]);
	assert_css("gap-x-2", &["column-gap:0.5rem;"]);
	assert_css("gap-y-3", &["row-gap:0.75rem;"]);
}

#[test]
fn display_position_and_box_model() {
	assert_css("flex", &["display:flex;"]);
	assert_css("grid", &["display:grid;"]);
	assert_css("hidden", &["display:none;"]);
	assert_css("relative", &["position:relative;"]);
	assert_css("absolute", &["position:absolute;"]);
	assert_css("box-border", &["box-sizing:border-box;"]);
	assert_css("box-content", &["box-sizing:content-box;"]);
	assert_css("overflow-visible", &["overflow:visible;"]);
	assert_css("overflow-hidden", &["overflow:hidden;"]);
	assert_css("overflow-scroll", &["overflow:scroll;"]);
}

#[test]
fn flexbox() {
	assert_css("flex-row", &["flex-direction:row;"]);
	assert_css("flex-row-reverse", &["flex-direction:row-reverse;"]);
	assert_css("flex-col", &["flex-direction:column;"]);
	assert_css("flex-col-reverse", &["flex-direction:column-reverse;"]);
	assert_css("flex-nowrap", &["flex-wrap:nowrap;"]);
	assert_css("flex-wrap", &["flex-wrap:wrap;"]);
	assert_css("flex-wrap-reverse", &["flex-wrap:wrap-reverse;"]);
	assert_css(
		"flex-1",
		&["flex-grow:1;", "flex-shrink:1;", "flex-basis:0%;"],
	);
	assert_css("grow", &["flex-grow:1;"]);
	assert_css("shrink-0", &["flex-shrink:0;"]);
}

#[test]
fn alignment() {
	assert_css("justify-start", &["justify-content:flex-start;"]);
	assert_css("justify-end", &["justify-content:flex-end;"]);
	assert_css("justify-center", &["justify-content:center;"]);
	assert_css("justify-between", &["justify-content:space-between;"]);
	assert_css("justify-around", &["justify-content:space-around;"]);
	assert_css("justify-evenly", &["justify-content:space-evenly;"]);
	assert_css("items-center", &["align-items:center;"]);
	assert_css("self-end", &["align-self:flex-end;"]);
	assert_css("self-auto", &["align-self:auto;"]);
	assert_css("content-between", &["align-content:space-between;"]);
	assert_css("justify-items-start", &["justify-items:start;"]);
	assert_css("justify-self-end", &["justify-self:end;"]);
	assert_css("justify-items-stretch", &["justify-items:stretch;"]);
}

#[test]
fn grid() {
	assert_css(
		"grid-cols-3",
		&["grid-template-columns:minmax(0px,1fr) minmax(0px,1fr) minmax(0px,1fr);"],
	);
	assert_css(
		"grid-rows-2",
		&["grid-template-rows:minmax(0px,1fr) minmax(0px,1fr);"],
	);
	assert_css("grid-cols-none", &["grid-template-columns:none;"]);
	assert_css("auto-cols-fr", &["grid-auto-columns:minmax(0px,1fr);"]);
	assert_css("auto-rows-fr", &["grid-auto-rows:minmax(0px,1fr);"]);
	assert_css("grid-flow-row", &["grid-auto-flow:row;"]);
	assert_css("grid-flow-col", &["grid-auto-flow:column;"]);
	assert_css("grid-flow-row-dense", &["grid-auto-flow:row dense;"]);
	assert_css("grid-flow-col-dense", &["grid-auto-flow:column dense;"]);
	assert_css("col-start-2", &["grid-column-start:2;"]);
	assert_css("col-end-3", &["grid-column-end:3;"]);
	assert_css("row-start-1", &["grid-row-start:1;"]);
	assert_css("row-end-auto", &["grid-row-end:auto;"]);
	assert_css("col-span-2", &["grid-column:auto/span 2;"]);
	assert_css("row-span-3", &["grid-row:auto/span 3;"]);
}

#[test]
fn aspect_typography_and_borders() {
	assert_css("aspect-square", &["aspect-ratio:1;"]);
	assert_css("aspect-auto", &["aspect-ratio:auto;"]);
	assert_css("font-normal", &["font-weight:400;"]);
	assert_css("font-bold", &["font-weight:700;"]);
	assert_css("text-center", &["text-align:center;"]);
	assert_css("text-start", &["text-align:start;"]);
	assert_css("text-end", &["text-align:end;"]);
	assert_css("border-2", &["border-width:2px;"]);
	assert_css("border-l-2", &["border-left-width:2px;border-style:solid;"]);
	assert_css(
		"border-y",
		&["border-top-width:1px;border-bottom-width:1px;"],
	);
	assert_css("italic", &["font-style:italic;"]);
	assert_css("not-italic", &["font-style:normal;"]);
	assert_css(
		"border-red-500",
		&["border-color:#fb2c36ff;", "--lk-border-color:#fb2c36ff;"],
	);
	assert_css(
		"rounded-lg",
		&["border-radius:0.5rem;", "clip-path:inset(0 round 0.5rem);"],
	);
}

#[test]
fn important_and_conditional_rules_wrap_declarations() {
	assert_css("!p-4", &["padding-top:1rem!important;"]);
	assert_css(
		"md:flex-row",
		&["@media (width>=48rem){", "flex-direction:row;"],
	);
	assert_css("max-md:hidden", &["@media (width<48rem){", "display:none;"]);
	assert_css(
		"dark:bg-white",
		&[
			"@media (prefers-color-scheme:dark){",
			"background-color:#ffffffff;",
		],
	);
	assert!(
		Tailwind::parse("p-4")
			.unwrap()
			.css_scoped("[data-lk-style=\"x\"]")
			.contains(":where([data-lk-style=\"x\"])")
	);
}
