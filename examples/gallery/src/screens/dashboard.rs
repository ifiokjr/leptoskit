//! A store dashboard: a dark sidebar, a grid of stat cards, a bar chart built
//! from views, and a ranked product list. Below the `lg` breakpoint the sidebar
//! hides and the page stacks into one scrolling column.

use leptoskit::prelude::Button;
use leptoskit::prelude::Image;
use leptoskit::prelude::Text;
use leptoskit::prelude::View;
use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_native::FontWeight;
use leptoskit_yoga::Dimension;

use crate::ScreenState;
use crate::ui::alpha;
use crate::ui::circle;
use crate::ui::hex;
use crate::ui::label;
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

/// A white card with a hairline border.
fn card(colors: &Palette) -> Element {
	View()
		.class("flex-col p-5")
		.background(colors.card)
		.border(1.0, colors.line)
		.class("rounded-[14px]")
}

fn nav_item(title: &str, selected: bool, colors: &Palette) -> Element {
	let white = hex("#ffffff");
	let (tint, background) = if selected {
		(white, alpha(white, 0.08))
	} else {
		(colors.sidebar_text, alpha(white, 0.0))
	};
	View()
		.class("flex-row items-center gap-3 h-9 py-0 px-3")
		.background(background)
		.class("rounded-[8px]")
		.child(
			View()
				.class("w-4 h-4 rounded-[4px]")
				.border(2.0, if selected { colors.accent } else { tint }),
		)
		.child(
			Text(title)
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
		View().class("pt-4 px-3 pb-2").child(label(
			title,
			type_scale::EYEBROW,
			alpha(colors.sidebar_text, 0.7),
		))
	};
	View()
		.class("hidden lg:flex flex-col w-[236px] py-6 px-3 gap-0.5")
		.background(colors.sidebar)
		.child(
			View()
				.class("flex-row items-center gap-3 px-3 pb-3")
				.child(
					View()
						.class("w-7 h-7 rounded-[8px]")
						.background(colors.accent)
						.class("items-center justify-center")
						.child(Text("H").class("text-[15px] font-bold").color(white)),
				)
				.child(
					Text("Harmattan")
						.class("text-[17px] font-bold")
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
			View()
				.class("flex-col gap-2 p-4")
				.background(alpha(white, 0.06))
				.class("rounded-[12px]")
				.child(
					Text("Free plan")
						.font(14.0, FontWeight::Semibold)
						.color(white),
				)
				.child(label(
					"2,400 of 3,000 orders used this month.",
					type_scale::FOOTNOTE,
					colors.sidebar_text,
				))
				.child(
					View()
						.class("h-1.5")
						.background(alpha(white, 0.12))
						.class("rounded-[3px]")
						.child(
							View()
								.background(colors.accent)
								.class("rounded-[3px] w-[80%] h-full"),
						),
				),
		)
		.child(
			View()
				.class("flex-row items-center gap-3 pt-4 px-2")
				.child(Image(state.assets.avatar(3)).class("w-[34px] h-[34px] rounded-[17px]"))
				.child(
					View()
						.class("flex-col gap-0.5")
						.child(
							Text("Efua Mensah")
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
	View()
		.class("py-0.5 px-2")
		.background(background)
		.class("rounded-[10px]")
		.child(Text(value).class("text-[12px] font-semibold").color(tint))
}

fn stat_card(title: &str, value: &str, change: &str, up: bool, colors: &Palette) -> Element {
	card(colors)
		.class("gap-2")
		.child(label(title, type_scale::FOOTNOTE, colors.muted))
		.child(Text(value).class("text-[28px] font-bold").color(colors.ink))
		.child(
			View()
				.class("flex-row flex-wrap items-center gap-2")
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
	let bars = View()
		.class("flex-row items-stretch gap-1 lg:gap-2 flex-1")
		.children(REVENUE.iter().enumerate().map(|(index, (month, value))| {
			let current = index == last;
			let tint = if current { colors.ink } else { colors.muted };
			View()
				.class("flex-col items-center gap-2 flex-1")
				.child(
					View().class("flex-col justify-end flex-1 w-full").child(
						View()
							.with_style(|style| {
								style.height = Dimension::Percent(value / CHART_MAX);
							})
							.class("rounded-[6px]")
							.background(if current {
								colors.accent
							} else {
								colors.accent_soft
							}),
					),
				)
				// Phone-width bars fit only an initial.
				.child(label(&month[..1], type_scale::CAPTION, tint).class("lg:hidden"))
				.child(label(*month, type_scale::CAPTION, tint).class("hidden lg:flex"))
		}));
	let legend = |color: Color, text: &str| {
		View()
			.class("flex-row items-center gap-1.5")
			.child(circle(8.0, color))
			.child(label(text, type_scale::CAPTION, colors.muted))
	};
	card(colors)
		.class("gap-6 h-80 lg:h-auto lg:flex-1")
		.child(
			View()
				.class("flex-row flex-wrap items-start gap-3")
				.child(
					View()
						.class("flex-col gap-1 grow shrink basis-48 min-w-0")
						.child(label("Revenue", type_scale::HEADLINE, colors.ink))
						.child(
							View()
								.class("flex-row flex-wrap items-center gap-2")
								.child(
									Text("$735,210")
										.class("text-[22px] font-bold")
										.color(colors.ink),
								)
								.child(delta("+18.2%", true, colors)),
						),
				)
				.child(
					View()
						.class("flex-row items-center gap-4")
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
	View()
		.class("flex-row items-center gap-3")
		.child(Image(state.assets.photo(photo)).class("w-10 h-10 rounded-[8px]"))
		.child(
			View()
				.class("flex-col gap-0.5 flex-1")
				.child(
					Text(name)
						.font(14.0, FontWeight::Semibold)
						.color(colors.ink),
				)
				.child(label(category, type_scale::CAPTION, colors.muted)),
		)
		.child(
			Text(amount)
				.font(14.0, FontWeight::Semibold)
				.color(colors.ink)
				.class("text-end"),
		)
}

fn top_products(state: &ScreenState, colors: &Palette) -> Element {
	card(colors)
		.class("w-full lg:w-80 gap-4")
		.child(
			View()
				.class("flex-row items-center justify-between")
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
	View()
		.class("flex-row items-center p-0.5")
		.background(colors.card)
		.border(1.0, colors.line)
		.class("rounded-[9px]")
		.children(["7 days", "30 days", "12 months"].map(|range| {
			let selected = range == "12 months";
			let button = Button(range)
				.font(
					13.0,
					if selected {
						FontWeight::Semibold
					} else {
						FontWeight::Medium
					},
				)
				.color(if selected { colors.ink } else { colors.muted })
				.class("rounded-[7px] h-[30px] py-0 px-3");
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

	let header = View()
		.class("flex-row flex-wrap items-center gap-3")
		.child(
			View()
				.class("flex-col gap-1 grow shrink basis-64 min-w-0")
				.child(
					Text("Overview")
						.class("text-[28px] font-bold")
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
			Button("Export")
				.font(13.0, FontWeight::Semibold)
				.color(colors.card)
				.background(colors.ink)
				.class("rounded-[9px] h-9 py-0 px-4"),
		);

	let stat_cards = View()
		.class("grid grid-cols-2 lg:grid-cols-4 gap-4")
		.child(stat_card("Revenue", "$92,418", "+12.4%", true, &colors))
		.child(stat_card("Orders", "1,864", "+8.1%", true, &colors))
		.child(stat_card("Average order", "$49.58", "+3.9%", true, &colors))
		.child(stat_card("Refund rate", "2.1%", "−0.6%", false, &colors));

	let main = View()
		.class("flex-col lg:flex-1 pt-[60px] px-4 pb-10 lg:py-8 lg:px-10 gap-6")
		.child(header)
		.child(stat_cards)
		.child(
			View()
				.class("flex-col lg:flex-row items-stretch gap-4 lg:flex-1")
				.child(revenue_chart(&colors))
				.child(top_products(state, &colors)),
		);

	View()
		.class("w-full h-full overflow-scroll lg:flex-row lg:items-stretch lg:overflow-hidden")
		.background(colors.canvas)
		.child(sidebar(state, &colors))
		.child(main)
}
