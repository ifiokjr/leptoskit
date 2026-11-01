use leptoskit_native::Color;
use leptoskit_native::FontWeight;
use leptoskit_native::NativeProps;
use leptoskit_native::StyleContext;
use leptoskit_native::Tailwind;
use leptoskit_native::TextAlign;
use leptoskit_yoga::Align;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::BoxSizing;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::Display;
use leptoskit_yoga::FlexDirection;
use leptoskit_yoga::FlexWrap;
use leptoskit_yoga::GridAutoFlow;
use leptoskit_yoga::GridLine;
use leptoskit_yoga::GridStyle;
use leptoskit_yoga::JustifyContent;
use leptoskit_yoga::Overflow;
use leptoskit_yoga::PositionType;
use leptoskit_yoga::Rect;
use leptoskit_yoga::Size;
use leptoskit_yoga::Style;
use leptoskit_yoga::TrackSizing;
use leptoskit_yoga::Yoga;

fn resolve(classes: &str) -> (Style, NativeProps) {
	resolve_at(classes, 800.0, 600.0, false)
}

fn close(actual: f32, expected: f32) {
	assert!(
		(actual - expected).abs() < 0.001,
		"expected {expected}, got {actual}"
	);
}

fn resolve_at(classes: &str, width: f32, height: f32, dark: bool) -> (Style, NativeProps) {
	let mut style = Style::default();
	let mut props = NativeProps::default();
	Tailwind::parse(classes).unwrap().apply(
		&mut style,
		&mut props,
		&StyleContext {
			viewport: Size::new(width, height),
			dark,
			..StyleContext::default()
		},
	);
	(style, props)
}

#[test]
fn normalizes_whitespace_duplicates_and_empty_classes() {
	let classes = Tailwind::parse(" p-4\nflex  p-4\tflex-col ").unwrap();
	assert_eq!(classes.classes(), "flex flex-col p-4");
	assert_eq!(classes, Tailwind::parse("flex-col flex p-4").unwrap());
	assert_eq!(Tailwind::parse(" \n\t").unwrap(), Tailwind::default());
	assert_eq!(Tailwind::default().css(), "");
}

#[test]
fn shorthand_axis_and_edge_precedence_is_independent_of_class_order() {
	let classes = "p-8 px-4 pt-1 m-6 mx-auto -mt-2 gap-8 gap-x-2";
	let reversed = classes
		.split_whitespace()
		.rev()
		.collect::<Vec<_>>()
		.join(" ");
	let (style, props) = resolve(classes);
	assert_eq!((style.clone(), props), resolve(&reversed));
	assert_eq!(style.padding.top, Dimension::Points(4.0));
	assert_eq!(style.padding.left, Dimension::Points(16.0));
	assert_eq!(style.padding.bottom, Dimension::Points(32.0));
	assert_eq!(style.margin.left, Dimension::Auto);

	assert_eq!(style.margin.top, Dimension::Points(-8.0));
	assert_eq!(style.margin.bottom, Dimension::Points(24.0));
	assert_eq!(style.gap.left, Dimension::Points(8.0));
	assert_eq!(style.gap.top, Dimension::Points(32.0));
}

#[test]
fn relative_units_resolve_on_the_correct_axis_and_stay_relative_in_css() {
	let (style, _) = resolve("w-[50vw] h-[25vh] min-w-[2rem] max-w-[75%] basis-1/3 size-8");
	assert_eq!(style.width, Dimension::Points(400.0));
	assert_eq!(style.height, Dimension::Points(150.0));
	assert_eq!(style.min_width, Dimension::Points(32.0));
	assert_eq!(style.max_width, Dimension::Percent(0.75));
	assert_eq!(style.flex_basis, Dimension::Percent(1.0 / 3.0));
	let css = Tailwind::parse("w-[50vw] h-[25vh] min-w-[2rem] max-w-[75%]")
		.unwrap()
		.css();

	for declaration in [
		"width:50vw;",
		"height:25vh;",
		"min-width:2rem;",
		"max-width:75%;",
	] {
		assert!(css.contains(declaration), "missing {declaration} in {css}");
	}
}

#[test]
fn flex_and_position_utilities_match_their_typed_contract() {
	let (style, _) = resolve(
		"flex flex-row-reverse flex-wrap-reverse flex-2 grow-0 shrink-0 justify-between items-end \
		 self-center content-around absolute inset-4 inset-x-8 -top-2 box-content",
	);
	assert_eq!(style.display, Display::Flex);
	assert_eq!(style.flex_direction, FlexDirection::RowReverse);
	assert_eq!(style.flex_wrap, FlexWrap::WrapReverse);
	close(style.flex_grow, 0.0);
	close(style.flex_shrink, 0.0);
	assert_eq!(style.flex_basis, Dimension::Percent(0.0));
	assert_eq!(style.justify_content, JustifyContent::SpaceBetween);

	assert_eq!(style.align_items, Align::FlexEnd);
	assert_eq!(style.align_self, Align::Center);
	assert_eq!(style.align_content, Align::SpaceAround);
	assert_eq!(style.position, PositionType::Absolute);
	assert_eq!(style.inset.top, Dimension::Points(-8.0));
	assert_eq!(style.inset.left, Dimension::Points(32.0));
	assert_eq!(style.inset.bottom, Dimension::Points(16.0));
	assert_eq!(style.box_sizing, BoxSizing::ContentBox);
	close(resolve("flex-2 grow").0.flex_grow, 1.0);
}

#[test]
fn breakpoints_are_inclusive_at_minimum_and_exclusive_at_maximum() {
	let classes = "w-4 sm:w-8 md:w-12 lg:w-16 xl:w-20 2xl:w-24 max-md:h-8 md:max-xl:h-12 xl:h-16";

	for (width, expected_width, expected_height) in [
		(639.0, 16.0, 32.0),
		(640.0, 32.0, 32.0),
		(768.0, 48.0, 48.0),
		(1024.0, 64.0, 48.0),
		(1280.0, 80.0, 64.0),
		(1536.0, 96.0, 64.0),
	] {
		let (style, _) = resolve_at(classes, width, 600.0, false);
		assert_eq!(style.width, Dimension::Points(expected_width));
		assert_eq!(style.height, Dimension::Points(expected_height));
	}
}

#[test]
fn dark_variants_and_important_rules_share_native_and_css_precedence() {
	let classes = "p-4 dark:p-8 md:p-12 md:dark:pt-2 p-1!";
	assert_eq!(
		resolve_at(classes, 1000.0, 600.0, true).0.padding.top,
		Dimension::Points(4.0)
	);
	assert_eq!(
		resolve_at(classes, 1000.0, 600.0, false).0.padding.top,
		Dimension::Points(4.0)
	);
	assert_eq!(resolve("!p-8 pt-1").0.padding.top, Dimension::Points(32.0));
	let css = Tailwind::parse(classes).unwrap().css();
	assert!(css.contains("@media (prefers-color-scheme:dark)"));
	assert!(css.contains("@media (width>=48rem)"));
	assert!(css.contains("padding-top:0.25rem!important;"));
}

#[test]
fn active_rules_apply_only_while_held_and_outrank_other_conditions() {
	let classes = Tailwind::parse(
		"bg-blue-600 md:bg-blue-500 active:bg-blue-800 dark:active:bg-blue-900 opacity-90! \
		 active:opacity-50",
	)
	.unwrap();
	let apply = |active, dark| {
		let mut style = Style::default();
		let mut props = NativeProps::default();
		classes.apply(
			&mut style,
			&mut props,
			&StyleContext {
				viewport: Size::new(1000.0, 600.0),
				dark,
				active,
				..StyleContext::default()
			},
		);
		props
	};
	let background = |class: &str| resolve(class).1.background;
	assert_eq!(apply(false, false).background, background("bg-blue-500"));
	// Pressed rules beat wider breakpoints, as `:active`'s extra specificity
	// does in CSS, and dark pressed rules beat light ones.
	assert_eq!(apply(true, false).background, background("bg-blue-800"));
	assert_eq!(apply(true, true).background, background("bg-blue-900"));
	// Important rules still win over pressed ones.
	assert_eq!(apply(true, false).opacity, Some(0.9));

	let css = classes.css();
	assert!(css.contains(".active\\3a bg-blue-800:active:not([aria-disabled=true]){"));
	assert!(
		css.contains("@media (prefers-color-scheme:dark){.dark\\3a active\\3a bg-blue-900:active")
	);
	assert!(
		css.find(".md\\3a bg-blue-500{").unwrap() < css.find("active\\3a bg-blue-800").unwrap(),
		"pressed rules follow responsive ones: {css}"
	);
	assert_eq!(
		Tailwind::parse("active:p-4 p-2").unwrap(),
		Tailwind::parse("p-2  active:p-4").unwrap()
	);
}

#[test]
fn viewport_and_appearance_changes_reuse_the_parsed_rules() {
	let classes = Tailwind::parse("w-[50vw] dark:bg-black md:rounded-lg").unwrap();
	let mut style = Style::default();
	let mut props = NativeProps::default();
	classes.apply(
		&mut style,
		&mut props,
		&StyleContext {
			viewport: Size::new(400.0, 600.0),
			dark: false,
			..StyleContext::default()
		},
	);
	assert_eq!(style.width, Dimension::Points(200.0));
	assert_eq!(props.background, None);
	let mut style = Style::default();
	let mut props = NativeProps::default();

	classes.apply(
		&mut style,
		&mut props,
		&StyleContext {
			viewport: Size::new(1000.0, 600.0),
			dark: true,
			..StyleContext::default()
		},
	);
	assert_eq!(style.width, Dimension::Points(500.0));
	assert_eq!(props.background, Some(Color::rgba(0, 0, 0, 255)));
	assert_eq!(props.corner_radius, Some(8.0));
}

#[test]
fn grid_declarations_preserve_unrelated_base_fields_and_use_minmax_zero_fr() {
	let mut style = Style {
		grid: Some(GridStyle {
			auto_rows: TrackSizing::Points(50.0),
			..GridStyle::default()
		}),
		..Style::default()
	};
	let mut props = NativeProps::default();
	Tailwind::parse(
		"grid grid-cols-3 grid-rows-2 col-span-2 row-span-2 grid-flow-col-dense \
		 justify-items-center justify-self-end",
	)
	.unwrap()
	.apply(&mut style, &mut props, &StyleContext::default());
	let grid = style.grid.unwrap();
	assert_eq!(grid.auto_rows, TrackSizing::Points(50.0));
	assert_eq!(grid.auto_flow, GridAutoFlow::ColumnDense);
	assert_eq!(grid.justify_items, Align::Center);
	assert_eq!(grid.justify_self, Align::FlexEnd);

	assert_eq!(grid.column.end, GridLine::Span(2));
	assert_eq!(grid.row.end, GridLine::Span(2));
	assert_eq!(grid.template_columns.len(), 3);

	for track in &grid.template_columns {
		let TrackSizing::MinMax(bounds) = track else {
			panic!("Tailwind tracks must use minmax(0,1fr)")
		};
		assert_eq!(bounds.min, TrackSizing::Points(0.0));
		assert_eq!(bounds.max, TrackSizing::Fraction(1.0));
	}

	assert_eq!(grid.template_rows.len(), 2);
}

#[test]
fn arbitrary_grid_tracks_repeat_and_signed_lines_are_typed() {
	let (style, _) = resolve(
		"grid grid-cols-[100px_repeat(2,minmax(0,1fr))_25%] grid-rows-[auto_2rem] auto-rows-fr \
		 col-span-full col-start-2 -col-end-1 row-start-[-2]",
	);
	let grid = style.grid.unwrap();
	assert_eq!(grid.template_columns.len(), 4);
	assert_eq!(grid.template_columns[0], TrackSizing::Points(100.0));
	assert_eq!(grid.template_columns[3], TrackSizing::Percent(0.25));
	assert_eq!(
		grid.template_rows,
		vec![TrackSizing::Auto, TrackSizing::Points(32.0)]
	);
	assert_eq!(grid.column.start, GridLine::Line(2));
	assert_eq!(grid.column.end, GridLine::Line(-1));

	assert_eq!(grid.row.start, GridLine::Line(-2));
}

#[test]
fn compiled_utilities_drive_real_layout() {
	let mut yoga = Yoga::new();
	let root = yoga.create_node();
	let first = yoga.create_node();
	let second = yoga.create_node();
	yoga.set_style(root, resolve("flex-row w-64 h-16 p-4 gap-4").0);
	yoga.set_style(first, resolve("flex-1").0);
	yoga.set_style(second, resolve("w-8").0);
	yoga.insert_child(root, first, 0);

	yoga.insert_child(root, second, 1);
	yoga.calculate_layout(
		root,
		Size::new(
			AvailableSpace::Definite(800.0),
			AvailableSpace::Definite(600.0),
		),
	);
	close(yoga.layout(first).x, 16.0);
	close(yoga.layout(first).width, 176.0);
	close(yoga.layout(second).x, 208.0);
	close(yoga.layout(second).width, 32.0);
}

#[test]
fn typography_colors_borders_and_opacity_resolve_for_native_and_web() {
	let (style, props) = resolve(
		"text-2xl font-semibold text-center text-[#1234] bg-blue-500/50 border-2 border-[#abc] \
		 rounded-xl opacity-[0.9]",
	);
	assert_eq!(props.font_size, Some(24.0));
	assert_eq!(props.font_weight, Some(FontWeight::Semibold));
	assert_eq!(props.text_align, Some(TextAlign::Center));
	assert_eq!(props.color, Some(Color::rgba(0x11, 0x22, 0x33, 0x44)));
	assert_eq!(props.background.unwrap().alpha(), 128);
	assert_eq!(props.border_width, Some(2.0));
	assert_eq!(props.border_color, Some(Color::rgba(0xaa, 0xbb, 0xcc, 255)));

	assert_eq!(props.corner_radius, Some(12.0));
	assert_eq!(props.opacity, Some(0.9));
	close(style.border.top, 2.0);
	assert!(
		Tailwind::parse("border-2")
			.unwrap()
			.css()
			.contains("border-width:2px;border-style:solid;")
	);
}

#[test]
fn border_defaults_follow_current_text_color_without_replacing_explicit_color() {
	let (_, props) = resolve("border text-red-500");
	assert_eq!(props.border_color, props.color);
	assert_eq!(
		resolve("border").1.border_color,
		Some(Color::rgba(0, 0, 0, 255))
	);
	let mut style = Style::default();
	let color = Color::rgba(1, 2, 3, 255);
	let mut props = NativeProps {
		border_color: Some(color),
		..NativeProps::default()
	};
	Tailwind::parse("border-2")
		.unwrap()
		.apply(&mut style, &mut props, &StyleContext::default());
	assert_eq!(props.border_color, Some(color));
}

#[test]
fn color_opacity_modifiers_preserve_transparent_and_existing_alpha() {
	assert_eq!(
		resolve("bg-transparent/50").1.background,
		Some(Color::rgba(0, 0, 0, 0))
	);
	assert_eq!(
		resolve("bg-[#1234]/50").1.background,
		Some(Color::rgba(0x11, 0x22, 0x33, 0x22))
	);
}

#[test]
fn full_default_palette_is_available_including_v43_neutrals() {
	for family in [
		"red", "orange", "amber", "yellow", "lime", "green", "emerald", "teal", "cyan", "sky",
		"blue", "indigo", "violet", "purple", "fuchsia", "pink", "rose", "slate", "gray", "zinc",
		"neutral", "stone", "mauve", "olive", "mist", "taupe",
	] {
		for shade in [50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 950] {
			assert!(
				Tailwind::parse(&format!("bg-{family}-{shade}")).is_ok(),
				"missing {family}-{shade}"
			);
		}
	}
	assert_eq!(resolve("bg-red-500").1.background, Color::hex("#fb2c36"));
}

#[test]
fn css_escapes_leading_digits_variants_fractions_and_arbitrary_values() {
	let css = Tailwind::parse("2xl:w-1/2 md:bg-[#123] p-4!")
		.unwrap()
		.css_scoped("[data-lk-style=\"abc\"]");
	assert!(css.contains(":where([data-lk-style=\"abc\"]).\\32 xl\\3a w-1\\2f 2"));
	assert!(css.contains("md\\3a bg-\\5b \\23 123\\5d "));
	assert!(css.contains("p-4\\21 "));
	assert!(css.contains("@media (width>=96rem)"));
	assert!(css.contains("width:50%;"));
}

#[test]
fn unsupported_syntax_and_inactive_utilities_are_errors() {
	for utility in [
		"block",
		"contents",
		"inline-flex",
		"fixed",
		"sticky",
		"items-baseline",
		"font-black",
		"border-x-red-500",
		"overflow-x-hidden",
		"hover:p-4",
		"focus:bg-black",
		"active:hover:bg-black",
		"active:",
		"dark:md:made-up",
		"2xl:shadow-lg",
		"w-[calc(100%-1px)]",
		"bg-[url(https://example.com/a.png)]",
		"bg-[var(--brand)]",
		"[color:red]",
		"md:max-sm:p-4",
		"p-[10px",
		"p-[[1px]]",
		"p-1]",
		"p-[</style>]",
		"dark:",
	] {
		let error = Tailwind::parse(utility).expect_err(utility);
		assert_eq!(error.class, utility);
		assert_ne!(error.reason, "");
		assert!(error.to_string().contains(utility));
	}
}

#[test]
fn invalid_numbers_ranges_units_and_grid_arguments_are_rejected() {
	for utility in [
		"p-[NaNpx]",
		"w-[infpx]",
		"h-[1e40px]",
		"p-[-1px]",
		"w-[-1%]",
		"-p-4",
		"gap-[-2rem]",
		"-gap-4",
		"-m-auto",
		"w-1/0",
		"w-[-1rem]",
		"opacity-101",
		"opacity-[-0.1]",
		"opacity-[NaN]",
		"bg-red-500/101",
		"bg-[#zzzzzz]",
		"rounded-[-1px]",
		"border-[-1px]",
		"text-[-1rem]",
		"grow-[-2]",
		"shrink-[NaN]",
		"grid-cols-0",
		"grid-cols-257",
		"grid-cols-[repeat(0,1fr)]",
		"grid-cols-[repeat(99999999,1fr)]",
		"grid-cols-[minmax(1fr,2fr)]",
		"grid-cols-[minmax(0,,1fr)]",
		"grid-cols-[minmax(0,)]",
		"grid-cols-[-1px]",
		"col-span-0",
		"col-start-0",
		"-col-start-10001",
		"aspect-0",
		"aspect-1/0",
	] {
		assert!(
			Tailwind::parse(utility).is_err(),
			"unexpectedly accepted {utility}"
		);
	}
}

#[test]
fn overflow_and_hidden_are_explicit_layout_declarations() {
	assert_eq!(resolve("overflow-hidden").0.overflow, Overflow::Hidden);
	assert_eq!(resolve("overflow-scroll").0.overflow, Overflow::Scroll);
	assert_eq!(resolve("overflow-visible").0.overflow, Overflow::Visible);
	assert_eq!(resolve("hidden").0.display, Display::None);
}

#[test]
fn active_display_and_overflow_classes_override_typed_visibility_flags() {
	for display in ["flex", "grid"] {
		let mut style = Style {
			display: Display::None,
			overflow: Overflow::Hidden,
			..Style::default()
		};
		let mut props = NativeProps {
			hidden: Some(true),
			clip: Some(true),
			..NativeProps::default()
		};
		Tailwind::parse(&format!("{display} overflow-visible"))
			.unwrap()
			.apply(&mut style, &mut props, &StyleContext::default());
		assert_eq!(props.hidden, Some(false));
		assert_eq!(props.clip, Some(false));
		assert_ne!(style.display, Display::None);
		assert_eq!(style.overflow, Overflow::Visible);
	}

	let (_, props) = resolve("hidden overflow-hidden");
	assert_eq!(props.hidden, Some(true));
	assert_eq!(props.clip, Some(true));
	assert_eq!(resolve("overflow-scroll").1.clip, Some(true));
}

#[test]
fn inactive_display_and_overflow_classes_retain_typed_visibility_flags() {
	let mut style = Style {
		display: Display::None,
		overflow: Overflow::Hidden,
		..Style::default()
	};
	let mut props = NativeProps {
		hidden: Some(true),
		clip: Some(true),
		..NativeProps::default()
	};
	Tailwind::parse("md:flex md:overflow-visible")
		.unwrap()
		.apply(
			&mut style,
			&mut props,
			&StyleContext {
				viewport: Size::new(400.0, 600.0),
				dark: false,
				..StyleContext::default()
			},
		);
	assert_eq!(props.hidden, Some(true));
	assert_eq!(props.clip, Some(true));
	assert_eq!(style.display, Display::None);
	assert_eq!(style.overflow, Overflow::Hidden);
}

#[test]
fn maximum_none_clears_typed_constraints_and_emits_valid_css() {
	let classes = Tailwind::parse("max-w-none max-h-none w-auto h-[auto] basis-auto").unwrap();
	let mut style = Style {
		max_width: Dimension::Points(300.0),
		max_height: Dimension::Points(200.0),
		..Style::default()
	};
	classes.apply(
		&mut style,
		&mut NativeProps::default(),
		&StyleContext::default(),
	);
	assert_eq!(style.max_width, Dimension::Auto);
	assert_eq!(style.max_height, Dimension::Auto);
	assert_eq!(style.width, Dimension::Auto);
	assert_eq!(style.height, Dimension::Auto);
	assert_eq!(style.flex_basis, Dimension::Auto);

	let css = classes.css();
	assert!(css.contains("max-width:none;"));
	assert!(css.contains("max-height:none;"));
	assert!(css.contains("width:auto;"));
	assert!(css.contains("height:auto;"));
	assert!(!css.contains("max-width:auto;"));
	assert!(!css.contains("max-height:auto;"));

	for utility in [
		"max-w-auto",
		"max-h-auto",
		"min-w-auto",
		"min-h-auto",
		"max-w-[auto]",
		"min-h-[auto]",
	] {
		assert!(
			Tailwind::parse(utility).is_err(),
			"unexpectedly accepted {utility}"
		);
	}
}

#[test]
fn numeric_bounds_cover_conversions_and_computed_fractions() {
	for utility in [
		"w-3e38/1",
		"w-[3e38vw]",
		"w-[3e38%]",
		"h-[3e38vh]",
		"w-1000000/0.00001",
		"p-1000001",
		"w-[1000001rem]",
		"aspect-[0.0000000001]",
		"aspect-[-1/-2]",
	] {
		assert!(
			Tailwind::parse(utility).is_err(),
			"unexpectedly accepted {utility}"
		);
	}

	let classes = Tailwind::parse("w-1000000/1 h-[1000000%] p-[1000000rem]").unwrap();
	let css = classes.css();
	assert!(css.contains("width:100000000%;"));
	assert!(css.contains("height:1000000%;"));
	assert!(!css.contains("inf"));
	assert!(!css.contains("NaN"));
}

#[test]
fn viewport_unit_resolution_stays_finite_at_extreme_runtime_inputs() {
	let classes = Tailwind::parse("w-[1000000vw] h-[1000000vh] -left-[1000000vw]").unwrap();
	let mut style = Style::default();
	classes.apply(
		&mut style,
		&mut NativeProps::default(),
		&StyleContext {
			viewport: Size::new(f32::MAX, f32::MAX),
			dark: false,
			..StyleContext::default()
		},
	);

	for dimension in [style.width, style.height, style.inset.left] {
		assert!(
			matches!(dimension, Dimension::Points(value) if value.is_finite() && value.abs() <= 1_000_000_000.0)
		);
	}

	for viewport in [f32::INFINITY, f32::NAN, -100.0] {
		let mut style = Style::default();
		classes.apply(
			&mut style,
			&mut NativeProps::default(),
			&StyleContext {
				viewport: Size::new(viewport, viewport),
				dark: false,
				..StyleContext::default()
			},
		);
		assert_eq!(style.width, Dimension::Points(0.0));
		assert_eq!(style.height, Dimension::Points(0.0));
		assert_eq!(style.inset.left, Dimension::Points(0.0));
	}
}

#[test]
fn safe_area_padding_resolves_host_insets_per_edge_and_emits_env_css() {
	let classes = Tailwind::parse("px-safe pt-safe-offset-6 pb-safe-offset-[30px] p-4").unwrap();
	let resolve_with = |safe_area| {
		let mut style = Style::default();
		classes.apply(
			&mut style,
			&mut NativeProps::default(),
			&StyleContext {
				safe_area,
				..StyleContext::default()
			},
		);
		style.padding
	};

	let padding = resolve_with(Rect::all_edges(47.0, 8.0, 34.0, 12.0));
	assert_eq!(padding.top, Dimension::Points(71.0));
	assert_eq!(padding.right, Dimension::Points(8.0));
	assert_eq!(padding.bottom, Dimension::Points(64.0));
	assert_eq!(padding.left, Dimension::Points(12.0));

	// A window without system UI keeps only the offsets.
	let padding = resolve_with(Rect::default());
	assert_eq!(padding.top, Dimension::Points(24.0));
	assert_eq!(padding.left, Dimension::Points(0.0));

	let padding = resolve_with(Rect::all_edges(f32::NAN, -5.0, f32::INFINITY, f32::MAX));
	assert_eq!(padding.top, Dimension::Points(24.0));
	assert_eq!(padding.right, Dimension::Points(0.0));
	assert_eq!(padding.bottom, Dimension::Points(30.0));
	assert_eq!(padding.left, Dimension::Points(1_000_000_000.0));

	let css = classes.css();
	assert!(css.contains("padding-left:env(safe-area-inset-left);"));
	assert!(css.contains("padding-right:env(safe-area-inset-right);"));
	assert!(css.contains("padding-top:calc(env(safe-area-inset-top) + 1.5rem);"));
	assert!(css.contains("padding-bottom:calc(env(safe-area-inset-bottom) + 30px);"));

	for utility in [
		"-pt-safe",
		"pt-safe-offset-[10%]",
		"pt-safe-offset-auto",
		"pt-safe-offset--1",
		"pt-safe-4",
		"mt-safe",
		"top-safe",
	] {
		assert!(
			Tailwind::parse(utility).is_err(),
			"`{utility}` should be rejected"
		);
	}
}

#[test]
fn edge_borders_and_italics_resolve_for_native() {
	let (style, props) = resolve("border-l-2 border-t italic");
	assert_eq!(props.border_width, None);
	let edges = props.border_widths.unwrap();
	assert_eq!(
		(edges.top, edges.right, edges.bottom, edges.left),
		(1.0, 0.0, 0.0, 2.0)
	);
	close(style.border.left, 2.0);
	close(style.border.right, 0.0);
	assert_eq!(props.italic, Some(true));
	assert_eq!(props.border_edges(), edges);

	assert!(props.is_custom_drawn());

	let (style, props) = resolve("border-x-[3px] border-y-4");
	let edges = props.border_widths.unwrap();
	assert_eq!(
		(edges.top, edges.right, edges.bottom, edges.left),
		(4.0, 3.0, 4.0, 3.0)
	);
	close(style.border.bottom, 4.0);

	// Edge utilities outrank the uniform width whatever the class order, as
	// in Tailwind's stylesheet.
	let edges = resolve("border-l-4 border-2").1.border_edges();
	assert_eq!((edges.top, edges.left), (2.0, 4.0));
	let edges = resolve("border border-l-4").1.border_edges();
	assert_eq!((edges.top, edges.left), (1.0, 4.0));

	assert_eq!(resolve("italic not-italic").1.italic, Some(false));
	assert!(Tailwind::parse("border-q-2").is_err());
	assert!(Tailwind::parse("border-l-[nope]").is_err());
}

#[test]
fn arbitrary_breakpoints_follow_the_viewport_natively() {
	assert_eq!(
		resolve_at("hidden min-[968px]:flex", 960.0, 600.0, false)
			.0
			.display,
		Display::None
	);
	assert_eq!(
		resolve_at("hidden min-[968px]:flex", 968.0, 600.0, false)
			.0
			.display,
		Display::Flex
	);
	assert_eq!(
		resolve_at("max-[968px]:hidden", 967.0, 600.0, false)
			.0
			.display,
		Display::None
	);
}
