//! A desktop dashboard: a dark sidebar, a grid of stat cards, a bar chart
//! built from views, and a ranked product list.

use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_native::FontWeight;
use leptoskit_native::TextAlign;
use leptoskit_yoga::Align;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::Display;
use leptoskit_yoga::GridStyle;
use leptoskit_yoga::JustifyContent;
use leptoskit_yoga::TrackSizing;

use crate::ScreenState;
use crate::ui::Layout;
use crate::ui::alpha;
use crate::ui::circle;
use crate::ui::edges;
use crate::ui::hex;
use crate::ui::label;
use crate::ui::space;
use crate::ui::spacer;
use crate::ui::type_scale;

struct Palette {
	canvas: Color,
	card: Color,
	ink: Color,
	muted: Color,
	line: Color,
	accent: Color,
	accent_soft: Color,
	sidebar: Color,
	sidebar_text: Color,
	up: Color,
	up_soft: Color,
	down: Color,
	down_soft: Color,
}

fn palette() -> Palette {
	let accent = hex("#4f46e5");
	Palette {
		canvas: hex("#f6f7f9"),
		card: hex("#ffffff"),
		ink: hex("#0f172a"),
		muted: hex("#64748b"),
		line: hex("#e5e7eb"),
		accent,
		accent_soft: alpha(accent, 0.16),
		sidebar: hex("#0f172a"),
		sidebar_text: hex("#94a3b8"),
		up: hex("#047857"),
		up_soft: hex("#d1fae5"),
		down: hex("#b91c1c"),
		down_soft: hex("#fee2e2"),
	}
}

const SIDEBAR_WIDTH: f32 = 236.0;

/// A white card with a hairline border.
fn card(colors: &Palette) -> Element {
	Element::view()
		.column()
		.padding(space::L + space::XS, space::L + space::XS)
		.background(colors.card)
		.border(1.0, colors.line)
		.corner_radius(14.0)
}

fn nav_item(title: &str, selected: bool, colors: &Palette) -> Element {
	let white = hex("#ffffff");
	let (tint, background) = if selected {
		(white, alpha(white, 0.08))
	} else {
		(colors.sidebar_text, alpha(white, 0.0))
	};
	Element::view()
		.row()
		.gap(space::M)
		.height(36.0)
		.padding(0.0, space::M)
		.background(background)
		.corner_radius(8.0)
		.child(
			Element::view()
				.size(16.0, 16.0)
				.corner_radius(4.0)
				.border(2.0, if selected { colors.accent } else { tint }),
		)
		.child(
			Element::text(title)
				.font(
					14.0,
					if selected {
						FontWeight::Semibold
					} else {
						FontWeight::Medium
					},
				)
				.color(tint),
		)
}

fn sidebar(state: &ScreenState, colors: &Palette) -> Element {
	let white = hex("#ffffff");
	let section = |title: &str| {
		Element::view()
			.padding_edges(space::L, space::M, space::S, space::M)
			.child(label(
				title,
				type_scale::EYEBROW,
				alpha(colors.sidebar_text, 0.7),
			))
	};
	Element::view()
		.column()
		.width(SIDEBAR_WIDTH)
		.padding(space::XL, space::M)
		.gap(space::XXS)
		.background(colors.sidebar)
		.child(
			Element::view()
				.row()
				.gap(space::M)
				.padding_edges(0.0, space::M, space::M, space::M)
				.child(
					Element::view()
						.size(28.0, 28.0)
						.corner_radius(8.0)
						.background(colors.accent)
						.align(Align::Center)
						.justify(JustifyContent::Center)
						.child(Element::text("H").font(15.0, FontWeight::Bold).color(white)),
				)
				.child(
					Element::text("Harmattan")
						.font(17.0, FontWeight::Bold)
						.color(white),
				),
		)
		.child(section("STORE"))
		.child(nav_item("Overview", true, colors))
		.child(nav_item("Orders", false, colors))
		.child(nav_item("Products", false, colors))
		.child(nav_item("Customers", false, colors))
		.child(section("INSIGHTS"))
		.child(nav_item("Analytics", false, colors))
		.child(nav_item("Reports", false, colors))
		.child(spacer())
		.child(
			Element::view()
				.column()
				.gap(space::S)
				.padding(space::L, space::L)
				.background(alpha(white, 0.06))
				.corner_radius(12.0)
				.child(
					Element::text("Free plan")
						.font(14.0, FontWeight::Semibold)
						.color(white),
				)
				.child(label(
					"2,400 of 3,000 orders used this month.",
					type_scale::FOOTNOTE,
					colors.sidebar_text,
				))
				.child(
					Element::view()
						.height(6.0)
						.background(alpha(white, 0.12))
						.corner_radius(3.0)
						.child(
							Element::view()
								.background(colors.accent)
								.corner_radius(3.0)
								.with_style(|style| {
									style.width = Dimension::Percent(0.8);
									style.height = Dimension::Percent(1.0);
								}),
						),
				),
		)
		.child(
			Element::view()
				.row()
				.gap(space::M)
				.padding_edges(space::L, space::S, 0.0, space::S)
				.child(
					Element::image(state.assets.avatar(3))
						.size(34.0, 34.0)
						.corner_radius(17.0),
				)
				.child(
					Element::view()
						.column()
						.gap(space::XXS)
						.child(
							Element::text("Efua Mensah")
								.font(14.0, FontWeight::Semibold)
								.color(white),
						)
						.child(label(
							"efua@harmattan.example",
							type_scale::CAPTION,
							colors.sidebar_text,
						)),
				),
		)
}

/// A delta pill: green for growth, red for decline.
fn delta(value: &str, up: bool, colors: &Palette) -> Element {
	let (tint, background) = if up {
		(colors.up, colors.up_soft)
	} else {
		(colors.down, colors.down_soft)
	};
	Element::view()
		.padding(2.0, space::S)
		.background(background)
		.corner_radius(10.0)
		.child(
			Element::text(value)
				.font(12.0, FontWeight::Semibold)
				.color(tint),
		)
}

fn stat_card(title: &str, value: &str, change: &str, up: bool, colors: &Palette) -> Element {
	card(colors)
		.gap(space::S)
		.child(label(title, type_scale::FOOTNOTE, colors.muted))
		.child(
			Element::text(value)
				.font(28.0, FontWeight::Bold)
				.color(colors.ink),
		)
		.child(
			Element::view()
				.row()
				.gap(space::S)
				.child(delta(change, up, colors))
				.child(label("vs last week", type_scale::CAPTION, colors.muted)),
		)
}

/// Monthly revenue, in thousands, for the bar chart.
const REVENUE: [(&str, f32); 12] = [
	("Nov", 42.0),
	("Dec", 58.0),
	("Jan", 39.0),
	("Feb", 47.0),
	("Mar", 55.0),
	("Apr", 61.0),
	("May", 52.0),
	("Jun", 68.0),
	("Jul", 74.0),
	("Aug", 66.0),
	("Sep", 81.0),
	("Oct", 92.0),
];

/// The value at the top of the chart's scale, in thousands.
const CHART_MAX: f32 = 100.0;

fn revenue_chart(colors: &Palette) -> Element {
	let last = REVENUE.len() - 1;
	// The plot fills whatever height the card has; each bar is a fraction of
	// its column's plot area.
	let bars = Element::view()
		.row()
		.align(Align::Stretch)
		.gap(space::S)
		.grow()
		.children(REVENUE.iter().enumerate().map(|(index, (month, value))| {
			let current = index == last;
			Element::view()
				.column()
				.align(Align::Center)
				.gap(space::S)
				.grow()
				.child(
					Element::view()
						.column()
						.justify(JustifyContent::FlexEnd)
						.grow()
						.with_style(|style| style.width = Dimension::Percent(1.0))
						.child(
							Element::view()
								.with_style(|style| {
									style.height = Dimension::Percent(value / CHART_MAX);
								})
								.corner_radius(6.0)
								.background(if current {
									colors.accent
								} else {
									colors.accent_soft
								}),
						),
				)
				.child(label(
					*month,
					type_scale::CAPTION,
					if current { colors.ink } else { colors.muted },
				))
		}));
	let legend = |color: Color, text: &str| {
		Element::view()
			.row()
			.gap(space::XS + 2.0)
			.child(circle(8.0, color))
			.child(label(text, type_scale::CAPTION, colors.muted))
	};
	card(colors)
		.gap(space::XL)
		.grow()
		.child(
			Element::view()
				.row()
				.align(Align::FlexStart)
				.child(
					Element::view()
						.column()
						.gap(space::XS)
						.grow()
						.child(label("Revenue", type_scale::HEADLINE, colors.ink))
						.child(
							Element::view()
								.row()
								.gap(space::S)
								.child(
									Element::text("$735,210")
										.font(22.0, FontWeight::Bold)
										.color(colors.ink),
								)
								.child(delta("+18.2%", true, colors)),
						),
				)
				.child(
					Element::view()
						.row()
						.gap(space::L)
						.child(legend(colors.accent, "This month"))
						.child(legend(colors.accent_soft, "Previous")),
				),
		)
		.child(bars)
}

fn product_row(
	state: &ScreenState,
	photo: usize,
	name: &str,
	category: &str,
	amount: &str,
	colors: &Palette,
) -> Element {
	Element::view()
		.row()
		.gap(space::M)
		.child(
			Element::image(state.assets.photo(photo))
				.size(40.0, 40.0)
				.corner_radius(8.0),
		)
		.child(
			Element::view()
				.column()
				.gap(space::XXS)
				.grow()
				.child(
					Element::text(name)
						.font(14.0, FontWeight::Semibold)
						.color(colors.ink),
				)
				.child(label(category, type_scale::CAPTION, colors.muted)),
		)
		.child(
			Element::text(amount)
				.font(14.0, FontWeight::Semibold)
				.color(colors.ink)
				.text_align(TextAlign::End),
		)
}

fn top_products(state: &ScreenState, colors: &Palette) -> Element {
	card(colors)
		.width(320.0)
		.gap(space::L)
		.child(
			Element::view()
				.row()
				.justify(JustifyContent::SpaceBetween)
				.child(label("Top products", type_scale::HEADLINE, colors.ink))
				.child(label("View all", type_scale::FOOTNOTE, colors.accent)),
		)
		.child(product_row(
			state,
			1,
			"Linen field jacket",
			"Outerwear",
			"$48,210",
			colors,
		))
		.child(product_row(
			state,
			3,
			"Clay pour-over set",
			"Kitchen",
			"$36,940",
			colors,
		))
		.child(product_row(
			state,
			6,
			"Kente weekender",
			"Bags",
			"$29,115",
			colors,
		))
		.child(product_row(
			state,
			4,
			"Shea wax candle",
			"Home",
			"$21,780",
			colors,
		))
		.child(product_row(
			state,
			10,
			"Indigo throw",
			"Home",
			"$18,402",
			colors,
		))
		.child(product_row(
			state,
			8,
			"Brass desk lamp",
			"Lighting",
			"$15,960",
			colors,
		))
		.child(product_row(
			state,
			12,
			"Raffia market tote",
			"Bags",
			"$12,337",
			colors,
		))
}

/// A segmented range picker.
fn range_picker(colors: &Palette) -> Element {
	Element::view()
		.row()
		.padding(2.0, 2.0)
		.background(colors.card)
		.border(1.0, colors.line)
		.corner_radius(9.0)
		.children(["7 days", "30 days", "12 months"].map(|range| {
			let selected = range == "12 months";
			let button = Element::button(range)
				.font(
					13.0,
					if selected {
						FontWeight::Semibold
					} else {
						FontWeight::Medium
					},
				)
				.color(if selected { colors.ink } else { colors.muted })
				.corner_radius(7.0)
				.height(30.0)
				.padding(0.0, space::M);
			if selected {
				button.background(colors.canvas)
			} else {
				button.background(alpha(colors.card, 0.0))
			}
		}))
}

/// The dashboard screen.
pub fn screen(state: &ScreenState) -> Element {
	let colors = palette();

	let header = Element::view()
		.row()
		.gap(space::M)
		.child(
			Element::view()
				.column()
				.gap(space::XS)
				.grow()
				.child(
					Element::text("Overview")
						.font(28.0, FontWeight::Bold)
						.color(colors.ink),
				)
				.child(label(
					"Tuesday, 20 October · Store performance across every channel",
					type_scale::BODY,
					colors.muted,
				)),
		)
		.child(range_picker(&colors))
		.child(
			Element::button("Export")
				.font(13.0, FontWeight::Semibold)
				.color(colors.card)
				.background(colors.ink)
				.corner_radius(9.0)
				.height(36.0)
				.padding(0.0, space::L),
		);

	let stat_cards = Element::view()
		.with_style(|style| {
			style.display = Display::Grid;
			style.gap = edges(space::L);
			style.grid = Some(GridStyle {
				template_columns: vec![TrackSizing::Fraction(1.0); 4],
				..GridStyle::default()
			});
		})
		.child(stat_card("Revenue", "$92,418", "+12.4%", true, &colors))
		.child(stat_card("Orders", "1,864", "+8.1%", true, &colors))
		.child(stat_card("Average order", "$49.58", "+3.9%", true, &colors))
		.child(stat_card("Refund rate", "2.1%", "−0.6%", false, &colors));

	let main = Element::view()
		.column()
		.grow()
		.padding(space::XXL, space::XXL + space::S)
		.gap(space::XL)
		.child(header)
		.child(stat_cards)
		.child(
			Element::view()
				.row()
				.align(Align::Stretch)
				.gap(space::L)
				.grow()
				.child(revenue_chart(&colors))
				.child(top_products(state, &colors)),
		);

	Element::view()
		.row()
		.align(Align::Stretch)
		.fill()
		.background(colors.canvas)
		.child(sidebar(state, &colors))
		.child(main)
}
