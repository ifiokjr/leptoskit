//! A store dashboard: a dark sidebar, a grid of stat cards, a bar chart built
//! from views, and a ranked product list. Below the `lg` breakpoint the sidebar
//! hides and the page stacks into one scrolling column.

use leptoskit::prelude::*;
use leptoskit_native::Color;
use leptoskit_native::FontWeight;
use leptoskit_yoga::Dimension;

use crate::ScreenState;
use crate::ui::Circle;
use crate::ui::Label;
use crate::ui::alpha;
use crate::ui::hex;
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
#[component]
fn Panel(colors: &Palette, #[prop(into)] class: String, children: Children) -> Element {
	view! {
		<View
			class="flex-col p-5"
			background=colors.card
			border=(1.0, colors.line)
			class="rounded-[14px]"
			class=class
		>
			{children}
		</View>
	}
}

#[component]
fn NavItem(title: &'static str, #[prop(optional)] selected: bool, colors: &Palette) -> Element {
	let white = hex("#ffffff");
	let (tint, background) = if selected {
		(white, alpha(white, 0.08))
	} else {
		(colors.sidebar_text, alpha(white, 0.0))
	};
	let weight = if selected {
		FontWeight::Semibold
	} else {
		FontWeight::Medium
	};

	view! {
		<View
			class="flex-row gap-3 items-center py-0 px-3 h-9"
			background=background
			class="rounded-[8px]"
		>
			<View
				class="w-4 h-4 rounded-[4px]"
				border=(2.0, if selected { colors.accent } else { tint })
			/>
			<Text font=(14.0, weight) color=tint>{title}</Text>
		</View>
	}
}

#[component]
fn Section(colors: &Palette, children: Children) -> Element {
	view! {
		<View class="px-3 pt-4 pb-2">
			<Label style=type_scale::EYEBROW color=alpha(colors.sidebar_text, 0.7)>
				{children}
			</Label>
		</View>
	}
}

#[component]
fn Sidebar(state: &ScreenState, colors: &Palette) -> Element {
	let white = hex("#ffffff");
	view! {
		<View class="hidden flex-col gap-0.5 py-6 px-3 lg:flex w-[236px]" background=colors.sidebar>
			<View class="flex-row gap-3 items-center px-3 pb-3">
				<View
					class="w-7 h-7 rounded-[8px]"
					background=colors.accent
					class="justify-center items-center"
				>
					<Text class="font-bold text-[15px]" color=white>"H"</Text>
				</View>
				<Text class="font-bold text-[17px]" color=white>"Harmattan"</Text>
			</View>
			<Section colors=colors>"STORE"</Section>
			<NavItem title="Overview" selected colors=colors />
			<NavItem title="Orders" colors=colors />
			<NavItem title="Products" colors=colors />
			<NavItem title="Customers" colors=colors />
			<Section colors=colors>"INSIGHTS"</Section>
			<NavItem title="Analytics" colors=colors />
			<NavItem title="Reports" colors=colors />
			<View class="flex-1" />
			<View class="flex-col gap-2 p-4" background=alpha(white, 0.06) class="rounded-[12px]">
				<Text font=(14.0, FontWeight::Semibold) color=white>"Free plan"</Text>
				<Label style=type_scale::FOOTNOTE color=colors.sidebar_text>
					"2,400 of 3,000 orders used this month."
				</Label>
				<View class="h-1.5" background=alpha(white, 0.12) class="rounded-[3px]">
					<View background=colors.accent class="h-full rounded-[3px] w-[80%]" />
				</View>
			</View>
			<View class="flex-row gap-3 items-center px-2 pt-4">
				<Image source=state.assets.avatar(3) class="w-[34px] h-[34px] rounded-[17px]" />
				<View class="flex-col gap-0.5">
					<Text font=(14.0, FontWeight::Semibold) color=white>"Efua Mensah"</Text>
					<Label style=type_scale::CAPTION color=colors.sidebar_text>
						"efua@harmattan.example"
					</Label>
				</View>
			</View>
		</View>
	}
}

/// A delta pill: green for growth, red for decline.
#[component]
fn Delta(#[prop(optional)] up: bool, colors: &Palette, children: Children) -> Element {
	let (tint, background) = if up {
		(colors.up, colors.up_soft)
	} else {
		(colors.down, colors.down_soft)
	};
	view! {
		<View class="py-0.5 px-2" background=background class="rounded-[10px]">
			<Text class="font-semibold text-[12px]" color=tint>{children}</Text>
		</View>
	}
}

#[component]
fn StatCard(
	title: &'static str,
	value: &'static str,
	change: &'static str,
	#[prop(optional)] up: bool,
	colors: &Palette,
) -> Element {
	view! {
		<Panel colors=colors class="gap-2">
			<Label style=type_scale::FOOTNOTE color=colors.muted>{title}</Label>
			<Text class="font-bold text-[28px]" color=colors.ink>{value}</Text>
			<View class="flex-row flex-wrap gap-2 items-center">
				<Delta up=up colors=colors>{change}</Delta>
				<Label style=type_scale::CAPTION color=colors.muted>"vs last week"</Label>
			</View>
		</Panel>
	}
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

/// The first character of a label, for axes too narrow for the whole label.
fn initial(label: &str) -> String {
	label.chars().take(1).collect()
}

#[component]
fn Legend(color: Color, colors: &Palette, children: Children) -> Element {
	view! {
		<View class="flex-row gap-1.5 items-center">
			<Circle diameter=8.0 color=color />
			<Label style=type_scale::CAPTION color=colors.muted>{children}</Label>
		</View>
	}
}

#[component]
fn RevenueChart(colors: &Palette) -> Element {
	let last = REVENUE.len() - 1;
	// The plot fills whatever height the card has; each bar is a fraction of
	// its column's plot area.
	let bars = REVENUE
		.iter()
		.enumerate()
		.map(|(index, (month, value))| {
			let current = index == last;
			let tint = if current { colors.ink } else { colors.muted };
			view! {
				<View class="flex-col flex-1 gap-2 items-center">
					<View class="flex-col flex-1 justify-end w-full">
						<View
							with_style=|style| style.height = Dimension::Percent(value / CHART_MAX)
							class="rounded-[6px]"
							background=if current {
								colors.accent
							} else {
								colors.accent_soft
							}
						/>
					</View>
					// Phone-width bars fit only an initial.
					<Label style=type_scale::CAPTION color=tint class="lg:hidden">
						{initial(month)}
					</Label>
					<Label style=type_scale::CAPTION color=tint class="hidden lg:flex">
						{*month}
					</Label>
				</View>
			}
		})
		.collect::<Fragment>();

	view! {
		<Panel colors=colors class="gap-6 h-80 lg:flex-1 lg:h-auto">
			<View class="flex-row flex-wrap gap-3 items-start">
				<View class="flex-col gap-1 min-w-0 grow shrink basis-48">
					<Label style=type_scale::HEADLINE color=colors.ink>"Revenue"</Label>
					<View class="flex-row flex-wrap gap-2 items-center">
						<Text class="font-bold text-[22px]" color=colors.ink>"$735,210"</Text>
						<Delta up colors=colors>"+18.2%"</Delta>
					</View>
				</View>
				<View class="flex-row gap-4 items-center">
					<Legend color=colors.accent colors=colors>"This month"</Legend>
					<Legend color=colors.accent_soft colors=colors>"Previous"</Legend>
				</View>
			</View>
			<View class="flex-row flex-1 gap-1 items-stretch lg:gap-2">{bars}</View>
		</Panel>
	}
}

#[component]
fn ProductRow(
	state: &ScreenState,
	photo: usize,
	name: &'static str,
	category: &'static str,
	amount: &'static str,
	colors: &Palette,
) -> Element {
	view! {
		<View class="flex-row gap-3 items-center">
			<Image source=state.assets.photo(photo) class="w-10 h-10 rounded-[8px]" />
			<View class="flex-col flex-1 gap-0.5">
				<Text font=(14.0, FontWeight::Semibold) color=colors.ink>{name}</Text>
				<Label style=type_scale::CAPTION color=colors.muted>{category}</Label>
			</View>
			<Text font=(14.0, FontWeight::Semibold) color=colors.ink class="text-end">
				{amount}
			</Text>
		</View>
	}
}

#[component]
fn TopProducts(state: &ScreenState, colors: &Palette) -> Element {
	view! {
		<Panel colors=colors class="gap-4 w-full lg:w-80">
			<View class="flex-row justify-between items-center">
				<Label style=type_scale::HEADLINE color=colors.ink>"Top products"</Label>
				<Label style=type_scale::FOOTNOTE color=colors.accent>"View all"</Label>
			</View>
			<ProductRow
				state=state
				photo=1
				name="Linen field jacket"
				category="Outerwear"
				amount="$48,210"
				colors=colors
			/>
			<ProductRow
				state=state
				photo=3
				name="Clay pour-over set"
				category="Kitchen"
				amount="$36,940"
				colors=colors
			/>
			<ProductRow
				state=state
				photo=6
				name="Kente weekender"
				category="Bags"
				amount="$29,115"
				colors=colors
			/>
			<ProductRow
				state=state
				photo=4
				name="Shea wax candle"
				category="Home"
				amount="$21,780"
				colors=colors
			/>
			<ProductRow
				state=state
				photo=10
				name="Indigo throw"
				category="Home"
				amount="$18,402"
				colors=colors
			/>
			<ProductRow
				state=state
				photo=8
				name="Brass desk lamp"
				category="Lighting"
				amount="$15,960"
				colors=colors
			/>
			<ProductRow
				state=state
				photo=12
				name="Raffia market tote"
				category="Bags"
				amount="$12,337"
				colors=colors
			/>
		</Panel>
	}
}

/// A segmented range picker.
#[component]
fn RangePicker(colors: &Palette) -> Element {
	let ranges = ["7 days", "30 days", "12 months"].map(|range| {
		let selected = range == "12 months";
		let weight = if selected {
			FontWeight::Semibold
		} else {
			FontWeight::Medium
		};
		let button = view! {
			<Button
				font=(13.0, weight)
				color=if selected { colors.ink } else { colors.muted }
				class="py-0 px-3 rounded-[7px] h-[30px]"
			>
				{range}
			</Button>
		};
		if selected {
			button.background(colors.canvas)
		} else {
			button.background(alpha(colors.card, 0.0))
		}
	});

	view! {
		<View
			class="flex-row items-center p-0.5"
			background=colors.card
			border=(1.0, colors.line)
			class="rounded-[9px]"
		>
			{ranges}
		</View>
	}
}

/// The dashboard screen.
pub fn screen(state: &ScreenState) -> Element {
	let colors = &palette();

	view! {
		<View
			class="overflow-scroll w-full h-full lg:overflow-hidden lg:flex-row lg:items-stretch"
			background=colors.canvas
		>
			<Sidebar state=state colors=colors />
			<View class="flex-col gap-6 px-4 pb-10 lg:flex-1 lg:py-8 lg:px-10 pt-[60px]">
				<View class="flex-row flex-wrap gap-3 items-center">
					<View class="flex-col gap-1 min-w-0 grow shrink basis-64">
						<Text class="font-bold text-[28px]" color=colors.ink>"Overview"</Text>
						<Label style=type_scale::BODY color=colors.muted>
							"Tuesday, 20 October · Store performance across every channel"
						</Label>
					</View>
					<RangePicker colors=colors />
					<Button
						font=(13.0, FontWeight::Semibold)
						color=colors.card
						background=colors.ink
						class="py-0 px-4 h-9 rounded-[9px]"
					>
						"Export"
					</Button>
				</View>
				<View class="grid grid-cols-2 gap-4 lg:grid-cols-4">
					<StatCard title="Revenue" value="$92,418" change="+12.4%" up colors=colors />
					<StatCard title="Orders" value="1,864" change="+8.1%" up colors=colors />
					<StatCard
						title="Average order"
						value="$49.58"
						change="+3.9%"
						up
						colors=colors
					/>
					<StatCard title="Refund rate" value="2.1%" change="−0.6%" colors=colors />
				</View>
				<View class="flex-col gap-4 items-stretch lg:flex-row lg:flex-1">
					<RevenueChart colors=colors />
					<TopProducts state=state colors=colors />
				</View>
			</View>
		</View>
	}
}
