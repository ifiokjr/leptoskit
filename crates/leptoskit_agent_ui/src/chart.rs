//! Charts: bars and columns, lines and areas, pies and donuts.
//!
//! Bars are views, so they lay out and label like any other content; lines
//! and pies are vector paths. Neither needs a widget an existing host lacks.
//! Colours follow the series order, never the values, and text never wears a
//! series colour: a swatch beside it carries identity.

use std::f32::consts::PI;

use leptoskit_native::Element;
use leptoskit_native::Role;
use leptoskit_native::components::Text;
use leptoskit_native::components::Vector;
use leptoskit_native::components::View;
use leptoskit_native::vector::Paint;
use leptoskit_native::vector::PathData;
use leptoskit_native::vector::Point;
use leptoskit_native::vector::Stroke;
use leptoskit_native::vector::VectorPath;
use leptoskit_native::vector::VectorScaling;
use leptoskit_native::vector::ViewBox;
use leptoskit_yoga::Dimension;

use crate::Theme;
use crate::builtins::meter;
use crate::builtins::shown;
use crate::registry::Component;
use crate::registry::Context;
use crate::registry::Prop;
use crate::registry::Props;
use crate::value::format_number;

/// The most bars before labels thin out to every few.
const MAX_LABELLED_BARS: usize = 12;
/// The most bars that carry their value at the tip.
const MAX_VALUE_LABELS: usize = 8;

/// One named run of values.
struct Series {
	name: String,
	values: Vec<f64>,
}

pub(crate) fn chart() -> Component {
	Component::new(
		"Chart",
		"A chart. `bar` compares amounts across categories, `line` and `area` show change over \
		 time, `pie` and `donut` show parts of a whole. Give one series with `values`, or several \
		 with <Series> tags inside it.",
		render,
	)
	.prop(Prop::text("type", "`bar`, `line`, `area`, `pie`, or `donut`.").default("bar"))
	.prop(Prop::list(
		"labels",
		"The category or period of each value, such as months.",
	))
	.prop(Prop::list(
		"values",
		"The numbers, for a chart with one series.",
	))
	.prop(Prop::text("name", "The name of that one series."))
	.prop(Prop::text("title", "A title above the chart."))
	.prop(
		Prop::text(
			"format",
			"How values show: `number`, `money`, or `percent` (of a fraction).",
		)
		.default("number"),
	)
	.prop(Prop::number("height", "The plot's height, from 80 to 480.").default("180"))
	.data_child("Series")
	.children("Any number of <Series> tags, up to 8.")
	.example(
		"<Chart type=\"line\" labels={[\"Jan\", \"Feb\", \"Mar\"]} format=\"money\">\n  <Series \
		 name=\"Savings\" values={cumsum([400, 450, 500])} />\n  <Series name=\"Goal\" \
		 values={[500, 1000, 1500]} />\n</Chart>",
	)
}

pub(crate) fn series() -> Component {
	Component::new(
		"Series",
		"One named run of values inside a <Chart>.",
		|_props, _cx| Err("belongs inside a <Chart>".to_owned()),
	)
	.prop(Prop::text("name", "What the values are, for the legend.").required())
	.prop(Prop::list("values", "The numbers, one per label.").required())
}

fn render(props: &Props, cx: &mut Context) -> Result<Element, String> {
	let kind = props.text_or("type", "bar");
	let format = props.text_or("format", "number");
	let labels = props.texts("labels")?;
	let height = props.number_or("height", 180.0)?.clamp(80.0, 480.0) as f32;

	let mut series = Vec::new();
	if props.has("values") {
		series.push(Series {
			name: props.text_or("name", ""),
			values: props.numbers("values")?,
		});
	}
	for data in props.data("Series") {
		series.push(Series {
			name: data.text_or("name", ""),
			values: data.numbers("values")?,
		});
	}

	check(&series, &labels, cx)?;

	let theme = cx.theme().clone();
	let plot = match kind.as_str() {
		"bar" => bars(cx, &theme, &series, &labels, &format, height)?,
		// Without vector paths, change over time still reads as columns, and
		// parts of a whole as the legend's amounts and shares.
		"line" | "area" if !cx.ui.widgets.vectors() => {
			bars(cx, &theme, &series, &labels, &format, height)?
		}
		"pie" | "donut" if !cx.ui.widgets.vectors() => {
			parts(cx, &theme, &series[0], &labels, &format)?
		}
		"line" => lines(cx, &theme, &series, &labels, &format, height, false)?,
		"area" => lines(cx, &theme, &series, &labels, &format, height, true)?,
		"pie" => pie(cx, &theme, &series[0], &labels, &format, height, false)?,
		"donut" => pie(cx, &theme, &series[0], &labels, &format, height, true)?,
		other => {
			return Err(format!(
				"`type` is `bar`, `line`, `area`, `pie`, or `donut`, not `{other}`"
			));
		}
	};

	let mut chart = View()
		.class("flex-col gap-2 min-w-0")
		.role(Role::Img)
		.aria_label(summary(props, &series, &labels, &format));
	if props.has("title") {
		chart = chart.child(Text(props.text("title")?).class("text-base font-semibold"));
	}
	if series.len() > 1 && !matches!(kind.as_str(), "pie" | "donut") {
		chart = chart.child(legend(&theme, &series));
	}

	Ok(chart.child(plot))
}

fn check(series: &[Series], labels: &[String], cx: &Context) -> Result<(), String> {
	let Some(first) = series.first() else {
		return Err("needs `values`, or <Series> tags inside it".to_owned());
	};
	if series.len() > 8 {
		return Err("draws at most 8 series; fold the rest into one, such as \"Other\"".to_owned());
	}
	if series
		.iter()
		.any(|series| series.values.len() != first.values.len())
	{
		return Err("every series needs the same number of values".to_owned());
	}
	if first.values.is_empty() {
		return Err("has no values".to_owned());
	}
	if !labels.is_empty() && labels.len() != first.values.len() {
		return Err(format!(
			"has {} labels for {} values",
			labels.len(),
			first.values.len()
		));
	}
	if series
		.iter()
		.flat_map(|series| &series.values)
		.any(|value| !value.is_finite())
	{
		return Err("values must be finite numbers".to_owned());
	}

	let points: usize = series.iter().map(|series| series.values.len()).sum();
	let limit = cx.ui.limits.max_chart_points;
	if points > limit {
		return Err(format!("draws at most {limit} points; this has {points}"));
	}

	Ok(())
}

/// What the chart shows, for screen readers.
fn summary(props: &Props, series: &[Series], labels: &[String], format: &str) -> String {
	let mut text = props.text_or("title", "Chart");
	for series in series {
		text.push_str(". ");
		if !series.name.is_empty() {
			text.push_str(&series.name);
			text.push_str(": ");
		}
		let items = series
			.values
			.iter()
			.enumerate()
			.take(24)
			.map(|(index, value)| {
				match labels.get(index) {
					Some(label) => format!("{label} {}", shown(*value, format)),
					None => shown(*value, format),
				}
			})
			.collect::<Vec<_>>();
		text.push_str(&items.join(", "));
		if series.values.len() > 24 {
			text.push_str(", …");
		}
	}
	text
}

/// A short form for axis labels: `1.2K`, `$3M`.
fn compact(value: f64, format: &str) -> String {
	let magnitude = value.abs();
	let (scaled, suffix) = if magnitude >= 1e9 {
		(value / 1e9, "B")
	} else if magnitude >= 1e6 {
		(value / 1e6, "M")
	} else if magnitude >= 1e3 {
		(value / 1e3, "K")
	} else {
		return shown(value, format);
	};
	let number = format_number((scaled * 10.0).round() / 10.0);

	match format {
		"money" if value < 0.0 => format!("-${}{suffix}", number.trim_start_matches('-')),
		"money" => format!("${number}{suffix}"),
		_ => format!("{number}{suffix}"),
	}
}

/// The class that paints series `index`, counting from 0.
fn series_role(index: usize) -> String {
	format!("series-{}", index + 1)
}

fn swatch(theme: &Theme, index: usize) -> Element {
	View()
		.class("w-2.5 h-2.5 rounded-full")
		.class(theme.class("bg", &series_role(index)))
		.aria_hidden(true)
}

fn legend(theme: &Theme, series: &[Series]) -> Element {
	View()
		.class("flex-row flex-wrap gap-x-4 gap-y-1 min-w-0")
		.aria_hidden(true)
		.children(series.iter().enumerate().map(|(index, series)| {
			View()
				.class("flex-row gap-1.5 items-center")
				.child(swatch(theme, index))
				.child(
					Text(series.name.clone())
						.class("text-xs")
						.class(theme.class("text", "muted")),
				)
		}))
}

/// A drawing that fills its box and paints in its text colour, so a colour
/// role class on it follows the theme and dark appearance. Stack several in
/// one box to paint each in its own role.
fn layer(view_box: ViewBox, theme: &Theme, role: &str, opacity: Option<u8>) -> Element {
	let class = match opacity {
		Some(opacity) => theme.expand(&format!("text-{role}/{opacity}")),
		None => theme.class("text", role),
	};
	Vector(view_box)
		.scaling(VectorScaling::Stretch)
		.class("absolute inset-0")
		.class(class)
		.aria_hidden(true)
}

fn fixed_height(element: Element, height: f32) -> Element {
	element.with_style(|style| style.height = Dimension::Points(height))
}

fn percent_height(element: Element, fraction: f64) -> Element {
	let fraction = fraction.clamp(0.0, 1.0) as f32;
	element.with_style(|style| style.height = Dimension::Percent(fraction))
}

/// The value range a plot spans, always including zero.
fn domain(series: &[Series]) -> (f64, f64) {
	let values = series
		.iter()
		.flat_map(|series| series.values.iter().copied());
	let (low, high) = values.fold((0.0_f64, 0.0_f64), |(low, high), value| {
		(low.min(value), high.max(value))
	});
	if high - low <= 0.0 {
		(0.0, 1.0)
	} else {
		(low, high)
	}
}

/// The labels under a plot, thinned to every few when there are many.
fn axis_labels(theme: &Theme, labels: &[String], count: usize, align: &'static str) -> Element {
	let every = count.div_ceil(MAX_LABELLED_BARS).max(1);
	View()
		.class("flex-row gap-1 min-w-0")
		.aria_hidden(true)
		.children((0..count).map(|index| {
			let text = if index % every == 0 {
				labels.get(index).cloned().unwrap_or_default()
			} else {
				String::new()
			};
			View().class("flex-1 min-w-0").child(
				Text(text)
					.class("text-xs")
					.class(theme.class("text", "muted"))
					.class(align),
			)
		}))
}

/// The value axis beside a plot: its top and bottom values.
fn value_axis(theme: &Theme, high: f64, low: f64, format: &str, height: f32) -> Element {
	let muted = theme.class("text", "muted");
	View()
		.class("flex-col")
		.aria_hidden(true)
		.child(fixed_height(
			View()
				.class("flex-col justify-between items-end")
				.child(Text(compact(high, format)).class("text-xs").class(&muted))
				.child(Text(compact(low, format)).class("text-xs").class(&muted)),
			height,
		))
		// Keeps the axis level with the plot above the category labels.
		.child(Text(" ").class("text-xs"))
}

fn bars(
	cx: &mut Context,
	theme: &Theme,
	series: &[Series],
	labels: &[String],
	format: &str,
	height: f32,
) -> Result<Element, String> {
	let count = series[0].values.len();
	cx.reserve(count * (series.len() * 2 + 3) + 12)?;

	let (low, high) = domain(series);
	let range = high - low;
	let tips = series.len() == 1 && count <= MAX_VALUE_LABELS;
	// Leaves room above the tallest bar for its value.
	let headroom = if tips { 0.82 } else { 1.0 };

	let side = |negative: bool| {
		let groups = (0..count).map(|index| {
			let bars = series.iter().enumerate().map(|(position, series)| {
				let value = series.values[index];
				let fraction = match (negative, value < 0.0) {
					(false, false) if high > 0.0 => value / high,
					(true, true) if low < 0.0 => value / low,
					_ => 0.0,
				};
				let bar = View()
					.class("flex-1 max-w-[24px] min-w-[3px]")
					.class("rounded-[3px]")
					.class(theme.class("bg", &series_role(position)));
				percent_height(bar, fraction * headroom)
			});

			let mut group = View()
				.class("flex-1 flex-row justify-center gap-[2px] min-w-0")
				.class(if negative { "items-start" } else { "items-end" });
			if tips && !negative {
				let value = series[0].values[index];
				group = View()
					.class("flex-1 flex-col justify-end items-center gap-0.5 min-w-0")
					.child(
						Text(if value >= 0.0 {
							compact(value, format)
						} else {
							String::new()
						})
						.class("text-xs"),
					)
					.child(
						View()
							.class("w-full flex-row justify-center")
							.with_style(|style| {
								style.height = Dimension::Percent(
									((value.max(0.0) / high.max(f64::MIN_POSITIVE)) * headroom)
										as f32,
								);
							})
							.children(bars.map(|bar| {
								bar.with_style(|style| style.height = Dimension::Percent(1.0))
							})),
					);
				return group;
			}
			group = group.children(bars);
			group
		});
		View().class("flex-row gap-1 min-w-0").children(groups)
	};

	let mut plot = View()
		.class("flex-col min-w-0")
		.child(percent_height(side(false), high / range))
		.child(View().class("h-px w-full").class(theme.class("bg", "line")));
	if low < 0.0 {
		plot = plot.child(percent_height(side(true), -low / range));
	}

	Ok(View()
		.class("flex-row gap-2 min-w-0")
		.child(value_axis(theme, high, low, format, height))
		.child(
			View()
				.class("flex-col flex-1 gap-1 min-w-0")
				.child(fixed_height(plot, height))
				.child(axis_labels(theme, labels, count, "text-center")),
		))
}

fn lines(
	cx: &mut Context,
	theme: &Theme,
	series: &[Series],
	labels: &[String],
	format: &str,
	height: f32,
	area: bool,
) -> Result<Element, String> {
	cx.reserve(series.len() * 3 + 16)?;
	let count = series[0].values.len();
	let (low, high) = domain(series);
	let range = high - low;

	let x = |index: usize| {
		if count == 1 {
			50.0
		} else {
			index as f32 / (count - 1) as f32 * 100.0
		}
	};
	let y = |value: f64| (100.0 - (value - low) / range * 100.0) as f32;
	let error = |error: leptoskit_native::vector::VectorError| format!("cannot draw it: {error:?}");
	// A little room above and below, so a line at the top or bottom keeps its
	// full width.
	let view_box = ViewBox::new(0.0, -3.0, 100.0, 106.0).map_err(error)?;

	let mut layers = Vec::new();
	if low < 0.0 {
		let zero = y(0.0);
		layers.push(
			layer(view_box, theme, "line", None).path(
				VectorPath::new(
					PathData::line(Point::new(0.0, zero), Point::new(100.0, zero))
						.map_err(error)?,
				)
				.stroke(Stroke::new(Paint::CurrentColor, 1.0).map_err(error)?),
			),
		);
	}

	for (position, series) in series.iter().enumerate() {
		let role = series_role(position);
		let points: Vec<Point> = series
			.values
			.iter()
			.enumerate()
			.map(|(index, value)| Point::new(x(index), y(*value)))
			.collect();

		if area {
			let baseline = y(0.0_f64.max(low));
			let mut outline = points.clone();
			outline.push(Point::new(x(count - 1), baseline));
			outline.push(Point::new(x(0), baseline));
			layers.push(
				layer(view_box, theme, &role, Some(15)).path(
					VectorPath::new(PathData::polygon(outline).map_err(error)?)
						.fill(Paint::CurrentColor),
				),
			);
		}

		let line = if points.len() == 1 {
			PathData::circle(points[0].x, points[0].y, 1.5).map_err(error)?
		} else {
			PathData::polyline(points).map_err(error)?
		};
		layers.push(
			layer(view_box, theme, &role, None).path(
				VectorPath::new(line).stroke(
					Stroke::new(Paint::CurrentColor, 2.0)
						.map_err(error)?
						.round(),
				),
			),
		);
	}

	let first = labels.first().cloned().unwrap_or_default();
	let last = if count > 1 {
		labels.last().cloned().unwrap_or_default()
	} else {
		String::new()
	};
	let muted = theme.class("text", "muted");
	let ends = View()
		.class("flex-row justify-between gap-2 min-w-0")
		.aria_hidden(true)
		.child(Text(first).class("text-xs").class(&muted))
		.child(Text(last).class("text-xs").class(&muted));

	Ok(View()
		.class("flex-row gap-2 min-w-0")
		.child(value_axis(theme, high, low, format, height))
		.child(
			View()
				.class("flex-col flex-1 gap-1 min-w-0")
				.child(fixed_height(
					View().class("relative w-full").children(layers),
					height,
				))
				.child(ends),
		))
}

/// A point on a circle around the drawing's centre, at `degrees` clockwise
/// from twelve o'clock.
fn polar(radius: f32, degrees: f32) -> (f32, f32) {
	let radians = (degrees - 90.0) * PI / 180.0;
	(50.0 + radius * radians.cos(), 50.0 + radius * radians.sin())
}

/// A slice from `start` to `end` degrees: a wedge, or a ring segment when
/// `inner` is more than zero.
fn slice(start: f32, end: f32, outer: f32, inner: f32) -> String {
	// An arc cannot start and end at the same point, so a whole circle is two
	// halves.
	if end - start >= 359.99 {
		let middle = start + 180.0;
		let first = slice(start, middle, outer, inner);
		let second = slice(middle, end, outer, inner);
		return format!("{first} {second}");
	}

	let large = i32::from(end - start > 180.0);
	let (x0, y0) = polar(outer, start);
	let (x1, y1) = polar(outer, end);

	if inner <= 0.0 {
		return format!("M 50 50 L {x0} {y0} A {outer} {outer} 0 {large} 1 {x1} {y1} Z");
	}

	let (x2, y2) = polar(inner, end);
	let (x3, y3) = polar(inner, start);
	format!(
		"M {x0} {y0} A {outer} {outer} 0 {large} 1 {x1} {y1} L {x2} {y2} A {inner} {inner} 0 \
		 {large} 0 {x3} {y3} Z"
	)
}

fn pie(
	cx: &mut Context,
	theme: &Theme,
	series: &Series,
	labels: &[String],
	format: &str,
	height: f32,
	donut: bool,
) -> Result<Element, String> {
	let total = whole(series)?;
	cx.reserve(series.values.len() * 5 + 6)?;

	let size = height.min(200.0);
	let error = |error: leptoskit_native::vector::VectorError| format!("cannot draw it: {error:?}");
	let view_box = ViewBox::new(0.0, 0.0, 100.0, 100.0).map_err(error)?;

	// A sliver of space between slices separates neighbours of similar hue.
	let gap = if series.values.len() > 1 { 1.2 } else { 0.0 };
	let mut start = 0.0_f32;
	let mut slices = Vec::new();
	for (index, value) in series.values.iter().enumerate() {
		let sweep = (*value / total * 360.0) as f32;
		if sweep > gap {
			let path = slice(
				start + gap / 2.0,
				start + sweep - gap / 2.0,
				48.0,
				if donut { 30.0 } else { 0.0 },
			);
			slices.push(layer(view_box, theme, &series_role(index), None).path(
				VectorPath::new(PathData::parse(&path).map_err(error)?).fill(Paint::CurrentColor),
			));
		}
		start += sweep;
	}
	let drawing = View()
		.class("relative")
		.with_style(|style| {
			style.width = Dimension::Points(size);
			style.height = Dimension::Points(size);
		})
		.children(slices);

	let muted = theme.class("text", "muted");
	let rows = series.values.iter().enumerate().map(|(index, value)| {
		let label = labels
			.get(index)
			.cloned()
			.unwrap_or_else(|| format!("Slice {}", index + 1));
		View()
			.class("flex-row gap-2 items-center min-w-0")
			.child(swatch(theme, index))
			.child(Text(label).class("text-sm flex-1 min-w-0"))
			.child(Text(shown(*value, format)).class("text-sm font-semibold"))
			.child(
				Text(format!("{:.0}%", value / total * 100.0))
					.class("text-xs min-w-[36px] text-end")
					.class(&muted),
			)
	});

	Ok(View()
		.class("flex-row flex-wrap gap-4 items-center min-w-0")
		.child(drawing)
		.child(
			View()
				.class("flex-col gap-1.5 grow shrink basis-40 min-w-0")
				.children(rows),
		))
}

/// The total of a series a pie shows, or why a pie cannot show it.
fn whole(series: &Series) -> Result<f64, String> {
	if series.values.len() > 8 {
		return Err(
			"a pie shows at most 8 slices; fold the smallest into one, such as \"Other\""
				.to_owned(),
		);
	}
	if series.values.iter().any(|value| *value < 0.0) {
		return Err("a pie cannot show negative values; use a bar chart".to_owned());
	}
	let total: f64 = series.values.iter().sum();
	if total <= 0.0 {
		return Err("a pie needs a total above zero".to_owned());
	}
	Ok(total)
}

/// Parts of a whole without a drawing: each part's amount and share.
fn parts(
	cx: &mut Context,
	theme: &Theme,
	series: &Series,
	labels: &[String],
	format: &str,
) -> Result<Element, String> {
	let total = whole(series)?;
	cx.reserve(series.values.len() * 6 + 2)?;

	let muted = theme.class("text", "muted");
	let rows = series.values.iter().enumerate().map(|(index, value)| {
		let label = labels
			.get(index)
			.cloned()
			.unwrap_or_else(|| format!("Slice {}", index + 1));
		let share = (value / total) as f32;
		View()
			.class("flex-col gap-1 min-w-0")
			.child(
				View()
					.class("flex-row gap-2 items-center min-w-0")
					.child(swatch(theme, index))
					.child(Text(label).class("text-sm flex-1 min-w-0"))
					.child(Text(shown(*value, format)).class("text-sm font-semibold"))
					.child(
						Text(format!("{:.0}%", share * 100.0))
							.class("text-xs min-w-[36px] text-end")
							.class(&muted),
					),
			)
			.child(meter(theme, share, &series_role(index), "h-1.5"))
	});

	Ok(View().class("flex-col gap-2 min-w-0").children(rows))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn compacts_axis_values() {
		assert_eq!(compact(950.0, "number"), "950");
		assert_eq!(compact(1200.0, "number"), "1.2K");
		assert_eq!(compact(2_500_000.0, "money"), "$2.5M");
		assert_eq!(compact(-1500.0, "money"), "-$1.5K");
		assert_eq!(compact(0.25, "percent"), "25%");
	}

	#[test]
	fn slices_parse_as_paths() {
		for (start, end, inner) in [(0.0, 90.0, 0.0), (10.0, 300.0, 30.0), (0.0, 360.0, 30.0)] {
			assert!(PathData::parse(&slice(start, end, 48.0, inner)).is_ok());
		}
	}
}
