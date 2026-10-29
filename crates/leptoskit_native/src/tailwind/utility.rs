use leptoskit_yoga::Align;
use leptoskit_yoga::BoxSizing;
use leptoskit_yoga::Display;
use leptoskit_yoga::FlexDirection;
use leptoskit_yoga::FlexWrap;
use leptoskit_yoga::GridAutoFlow;
use leptoskit_yoga::GridLine;
use leptoskit_yoga::GridPlacement;
use leptoskit_yoga::JustifyContent;
use leptoskit_yoga::Overflow;
use leptoskit_yoga::PositionType;

use super::declaration::AlignField;
use super::declaration::Axis;
use super::declaration::ColorField;
use super::declaration::Declaration;
use super::declaration::Edge;
use super::declaration::EdgeField;
use super::declaration::SizeField;
use super::declaration::Track;
use super::value::Length;
use super::value::arbitrary;
use super::value::bounded;
use super::value::color;
use super::value::finite;
use crate::FontWeight;
use crate::TextAlign;

type Parsed = Result<(u16, Vec<Declaration>), String>;

pub(super) fn parse(utility: &str) -> Parsed {
	if let Some(declaration) = keyword(utility) {
		let rank = match declaration {
			Declaration::Flex(..) => 110,
			Declaration::Grow(_) | Declaration::Shrink(_) => 121,
			_ => 100,
		};
		return Ok((rank, vec![declaration]));
	}

	if let Some(result) = spacing(utility) {
		return result;
	}

	if let Some(result) = sizing(utility) {
		return result;
	}

	if let Some(result) = grid(utility) {
		return result;
	}

	if let Some(result) = alignment(utility) {
		return result;
	}

	if let Some(result) = paint(utility) {
		return result;
	}

	if let Some(value) = utility.strip_prefix("grow-") {
		return non_negative(value).map(|value| (121, vec![Declaration::Grow(value)]));
	}

	if let Some(value) = utility.strip_prefix("shrink-") {
		return non_negative(value).map(|value| (121, vec![Declaration::Shrink(value)]));
	}

	if let Some(value) = utility.strip_prefix("flex-") {
		let value = non_negative(value)?;
		return Ok((
			110,
			vec![Declaration::Flex(value, 1.0, Length::Percent(0.0))],
		));
	}

	if let Some(value) = utility.strip_prefix("aspect-") {
		let value = arbitrary(value).unwrap_or(value);
		let ratio = if let Some((width, height)) = value.split_once('/') {
			let width = finite(width)?;
			let height = finite(height)?;

			if width <= 0.0 || height <= 0.0 {
				return Err("aspect ratio dimensions must be positive".to_owned());
			}
			bounded(width / height)?
		} else {
			finite(value)?
		};

		if !(0.000_001..=1_000_000.0).contains(&ratio) {
			return Err("aspect ratio must be between 0.000001 and 1000000".to_owned());
		}

		return Ok((100, vec![Declaration::Aspect(Some(ratio))]));
	}

	Err(format!("unsupported utility `{utility}`"))
}

fn keyword(utility: &str) -> Option<Declaration> {
	Some(match utility {
		"flex" => Declaration::Display(Display::Flex),
		"grid" => Declaration::Display(Display::Grid),
		"hidden" => Declaration::Display(Display::None),
		"relative" => Declaration::Position(PositionType::Relative),
		"absolute" => Declaration::Position(PositionType::Absolute),
		"box-border" => Declaration::BoxSizing(BoxSizing::BorderBox),
		"box-content" => Declaration::BoxSizing(BoxSizing::ContentBox),
		"overflow-visible" => Declaration::Overflow(Overflow::Visible),
		"overflow-hidden" => Declaration::Overflow(Overflow::Hidden),
		"overflow-scroll" => Declaration::Overflow(Overflow::Scroll),
		"flex-row" => Declaration::FlexDirection(FlexDirection::Row),
		"flex-row-reverse" => Declaration::FlexDirection(FlexDirection::RowReverse),
		"flex-col" => Declaration::FlexDirection(FlexDirection::Column),
		"flex-col-reverse" => Declaration::FlexDirection(FlexDirection::ColumnReverse),
		"flex-wrap" => Declaration::FlexWrap(FlexWrap::Wrap),
		"flex-wrap-reverse" => Declaration::FlexWrap(FlexWrap::WrapReverse),
		"flex-nowrap" => Declaration::FlexWrap(FlexWrap::NoWrap),
		"flex-auto" => Declaration::Flex(1.0, 1.0, Length::Auto),
		"flex-initial" => Declaration::Flex(0.0, 1.0, Length::Auto),
		"flex-none" => Declaration::Flex(0.0, 0.0, Length::Auto),
		"grow" => Declaration::Grow(1.0),
		"shrink" => Declaration::Shrink(1.0),
		"aspect-auto" => Declaration::Aspect(None),
		"aspect-square" => Declaration::Aspect(Some(1.0)),
		"aspect-video" => Declaration::Aspect(Some(16.0 / 9.0)),
		"font-normal" => Declaration::FontWeight(FontWeight::Regular),
		"font-medium" => Declaration::FontWeight(FontWeight::Medium),
		"font-semibold" => Declaration::FontWeight(FontWeight::Semibold),
		"font-bold" => Declaration::FontWeight(FontWeight::Bold),
		"italic" => Declaration::Italic(true),
		"not-italic" => Declaration::Italic(false),
		"text-start" => Declaration::TextAlign(TextAlign::Start),
		"text-center" => Declaration::TextAlign(TextAlign::Center),
		"text-end" => Declaration::TextAlign(TextAlign::End),
		_ => return None,
	})
}

fn spacing(utility: &str) -> Option<Parsed> {
	let negative = utility.starts_with('-');
	let utility = utility.strip_prefix('-').unwrap_or(utility);
	let (prefix, value) = utility.split_once('-')?;
	let (field, edges, rank) = match prefix {
		"p" => {
			(
				EdgeField::Padding,
				&[Edge::Top, Edge::Right, Edge::Bottom, Edge::Left][..],
				200,
			)
		}
		"px" => (EdgeField::Padding, &[Edge::Left, Edge::Right][..], 201),
		"py" => (EdgeField::Padding, &[Edge::Top, Edge::Bottom][..], 201),
		"pt" => (EdgeField::Padding, &[Edge::Top][..], 202),
		"pr" => (EdgeField::Padding, &[Edge::Right][..], 202),
		"pb" => (EdgeField::Padding, &[Edge::Bottom][..], 202),
		"pl" => (EdgeField::Padding, &[Edge::Left][..], 202),
		"m" => {
			(
				EdgeField::Margin,
				&[Edge::Top, Edge::Right, Edge::Bottom, Edge::Left][..],
				200,
			)
		}
		"mx" => (EdgeField::Margin, &[Edge::Left, Edge::Right][..], 201),
		"my" => (EdgeField::Margin, &[Edge::Top, Edge::Bottom][..], 201),
		"mt" => (EdgeField::Margin, &[Edge::Top][..], 202),
		"mr" => (EdgeField::Margin, &[Edge::Right][..], 202),
		"mb" => (EdgeField::Margin, &[Edge::Bottom][..], 202),
		"ml" => (EdgeField::Margin, &[Edge::Left][..], 202),
		"inset" => {
			(
				EdgeField::Inset,
				&[Edge::Top, Edge::Right, Edge::Bottom, Edge::Left][..],
				200,
			)
		}
		"top" => (EdgeField::Inset, &[Edge::Top][..], 202),
		"right" => (EdgeField::Inset, &[Edge::Right][..], 202),
		"bottom" => (EdgeField::Inset, &[Edge::Bottom][..], 202),
		"left" => (EdgeField::Inset, &[Edge::Left][..], 202),
		_ => {
			if let Some(value) = utility.strip_prefix("inset-x-") {
				return Some(edge_value(
					EdgeField::Inset,
					&[Edge::Left, Edge::Right],
					201,
					value,
					negative,
				));
			}

			if let Some(value) = utility.strip_prefix("inset-y-") {
				return Some(edge_value(
					EdgeField::Inset,
					&[Edge::Top, Edge::Bottom],
					201,
					value,
					negative,
				));
			}

			if let Some(value) = utility.strip_prefix("gap-") {
				return Some(gap(value, negative));
			}

			return None;
		}
	};

	// `inset-x-*` and `inset-y-*` share the first prefix with `inset-*`.
	if field == EdgeField::Inset && prefix == "inset" {
		if let Some(value) = value.strip_prefix("x-") {
			return Some(edge_value(
				field,
				&[Edge::Left, Edge::Right],
				201,
				value,
				negative,
			));
		}

		if let Some(value) = value.strip_prefix("y-") {
			return Some(edge_value(
				field,
				&[Edge::Top, Edge::Bottom],
				201,
				value,
				negative,
			));
		}
	}

	Some(edge_value(field, edges, rank, value, negative))
}

fn edge_value(field: EdgeField, edges: &[Edge], rank: u16, value: &str, negative: bool) -> Parsed {
	if negative && (field == EdgeField::Padding || value == "auto") {
		return Err("this utility does not accept a negative prefix".to_owned());
	}

	let safe_offset = match value.strip_prefix("safe") {
		Some("") => Some("0"),
		Some(rest) => rest.strip_prefix("-offset-"),
		None => None,
	};

	if field == EdgeField::Padding
		&& let Some(offset) = safe_offset
	{
		let offset = Length::spacing(offset, false, false, false)?.absolute()?;
		return Ok((
			rank,
			edges
				.iter()
				.map(|edge| Declaration::SafePadding(*edge, offset))
				.collect(),
		));
	}

	let allow_negative = field != EdgeField::Padding;
	let length = Length::spacing(
		value,
		allow_negative,
		allow_negative,
		field == EdgeField::Inset,
	)?;
	let length = if negative { length.negated() } else { length };
	Ok((
		rank,
		edges
			.iter()
			.map(|edge| Declaration::Edge(field, *edge, length))
			.collect(),
	))
}

fn gap(value: &str, negative: bool) -> Parsed {
	if negative {
		return Err("gaps cannot be negative".to_owned());
	}

	let (axes, value, rank) = if let Some(value) = value.strip_prefix("x-") {
		(&[Axis::X][..], value, 201)
	} else if let Some(value) = value.strip_prefix("y-") {
		(&[Axis::Y][..], value, 201)
	} else {
		(&[Axis::X, Axis::Y][..], value, 200)
	};
	let length = Length::spacing(value, false, false, false)?;
	Ok((
		rank,
		axes.iter()
			.map(|axis| Declaration::Gap(*axis, length))
			.collect(),
	))
}

fn sizing(utility: &str) -> Option<Parsed> {
	let prefixes = [
		("min-w-", SizeField::MinWidth),
		("min-h-", SizeField::MinHeight),
		("max-w-", SizeField::MaxWidth),
		("max-h-", SizeField::MaxHeight),
		("basis-", SizeField::Basis),
		("w-", SizeField::Width),
		("h-", SizeField::Height),
	];

	for (prefix, field) in prefixes {
		if let Some(value) = utility.strip_prefix(prefix) {
			let length = size_value(value, field);
			return Some(length.map(|length| (121, vec![Declaration::Size(field, length)])));
		}
	}

	utility.strip_prefix("size-").map(|value| {
		size_value(value, SizeField::Width).map(|length| {
			(
				120,
				vec![
					Declaration::Size(SizeField::Width, length),
					Declaration::Size(SizeField::Height, length),
				],
			)
		})
	})
}

fn size_value(value: &str, field: SizeField) -> Result<Length, String> {
	let keyword = arbitrary(value).unwrap_or(value);
	let maximum = matches!(field, SizeField::MaxWidth | SizeField::MaxHeight);
	let minimum = matches!(field, SizeField::MinWidth | SizeField::MinHeight);

	if maximum && keyword == "none" {
		return Ok(Length::Auto);
	}

	if (maximum || minimum) && keyword == "auto" {
		return Err(
			"automatic minimum and maximum sizing is unsupported; use max-*-none to remove a \
			 maximum"
				.to_owned(),
		);
	}

	if value == "screen" {
		return Ok(match field {
			SizeField::Height | SizeField::MinHeight | SizeField::MaxHeight => Length::Vh(100.0),
			_ => Length::Vw(100.0),
		});
	}

	if matches!(field, SizeField::MaxWidth | SizeField::Width) {
		let rem = match value {
			"3xs" => Some(16.0),
			"2xs" => Some(18.0),
			"xs" => Some(20.0),
			"sm" => Some(24.0),
			"md" => Some(28.0),
			"lg" => Some(32.0),
			"xl" => Some(36.0),
			"2xl" => Some(42.0),
			"3xl" => Some(48.0),
			"4xl" => Some(56.0),
			"5xl" => Some(64.0),
			"6xl" => Some(72.0),
			"7xl" => Some(80.0),
			_ => None,
		};

		if let Some(rem) = rem {
			return Ok(Length::Rem(rem));
		}
	}

	Length::spacing(value, false, true, true)
}

fn alignment(utility: &str) -> Option<Parsed> {
	if let Some(value) = utility.strip_prefix("justify-") {
		if !value.starts_with("items-") && !value.starts_with("self-") {
			let justify = match value {
				"start" => JustifyContent::FlexStart,
				"end" => JustifyContent::FlexEnd,
				"center" => JustifyContent::Center,
				"between" => JustifyContent::SpaceBetween,
				"around" => JustifyContent::SpaceAround,
				"evenly" => JustifyContent::SpaceEvenly,
				_ => return Some(Err(format!("unsupported justification `{value}`"))),
			};
			return Some(Ok((100, vec![Declaration::Justify(justify)])));
		}
	}

	for (prefix, field) in [
		("items-", AlignField::Items),
		("self-", AlignField::SelfItem),
		("content-", AlignField::Content),
		("justify-items-", AlignField::JustifyItems),
		("justify-self-", AlignField::JustifySelf),
	] {
		if let Some(value) = utility.strip_prefix(prefix) {
			return Some(
				align_value(value, field)
					.map(|align| (100, vec![Declaration::Align(field, align)])),
			);
		}
	}

	for (prefix, fields) in [
		(
			"place-items-",
			[AlignField::Items, AlignField::JustifyItems],
		),
		(
			"place-self-",
			[AlignField::SelfItem, AlignField::JustifySelf],
		),
	] {
		if let Some(value) = utility.strip_prefix(prefix) {
			return Some(align_value(value, fields[0]).and_then(|align| {
				align_value(value, fields[1])?;
				Ok((
					99,
					fields
						.iter()
						.map(|field| Declaration::Align(*field, align))
						.collect(),
				))
			}));
		}
	}

	None
}

fn align_value(value: &str, field: AlignField) -> Result<Align, String> {
	let align = match value {
		"start" => Align::FlexStart,
		"end" => Align::FlexEnd,
		"center" => Align::Center,
		"stretch" => Align::Stretch,
		"auto" if matches!(field, AlignField::SelfItem | AlignField::JustifySelf) => Align::Auto,
		"between" if field == AlignField::Content => Align::SpaceBetween,
		"around" if field == AlignField::Content => Align::SpaceAround,
		"evenly" if field == AlignField::Content => Align::SpaceEvenly,
		_ => return Err(format!("unsupported alignment `{value}`")),
	};
	Ok(align)
}

fn paint(utility: &str) -> Option<Parsed> {
	if let Some(value) = utility.strip_prefix("bg-") {
		return Some(
			color(value)
				.map(|color| (300, vec![Declaration::Color(ColorField::Background, color)])),
		);
	}

	if let Some(value) = utility.strip_prefix("text-") {
		let rem = match value {
			"xs" => Some(0.75),
			"sm" => Some(0.875),
			"base" => Some(1.0),
			"lg" => Some(1.125),
			"xl" => Some(1.25),
			"2xl" => Some(1.5),
			"3xl" => Some(1.875),
			"4xl" => Some(2.25),
			"5xl" => Some(3.0),
			"6xl" => Some(3.75),
			"7xl" => Some(4.5),
			"8xl" => Some(6.0),
			"9xl" => Some(8.0),
			_ => None,
		};

		if let Some(rem) = rem {
			return Some(Ok((300, vec![Declaration::FontSize(Length::Rem(rem))])));
		}

		if let Some(inner) = arbitrary(value) {
			let inner = inner.strip_prefix("length:").unwrap_or(inner);

			if !inner.starts_with('#') && !inner.starts_with("color:") {
				return Some(
					Length::parse(inner, false, false)
						.and_then(Length::absolute)
						.map(|size| (300, vec![Declaration::FontSize(size)])),
				);
			}
		}

		return Some(
			color(value).map(|color| (300, vec![Declaration::Color(ColorField::Text, color)])),
		);
	}

	if utility == "border" {
		return Some(Ok((300, vec![Declaration::BorderWidth(Length::Px(1.0))])));
	}

	if let Some(parsed) = edge_border(utility) {
		return Some(parsed);
	}

	if let Some(value) = utility.strip_prefix("border-") {
		if value
			.chars()
			.next()
			.is_some_and(|character| character.is_ascii_digit())
		{
			return Some(
				non_negative(value)
					.map(|width| (300, vec![Declaration::BorderWidth(Length::Px(width))])),
			);
		}

		if let Some(inner) = arbitrary(value) {
			if !inner.starts_with('#') && !inner.starts_with("color:") {
				return Some(
					Length::parse(inner.strip_prefix("length:").unwrap_or(inner), false, false)
						.and_then(Length::absolute)
						.map(|width| (300, vec![Declaration::BorderWidth(width)])),
				);
			}
		}

		return Some(
			color(value).map(|color| (301, vec![Declaration::Color(ColorField::Border, color)])),
		);
	}

	if utility == "rounded" || utility.starts_with("rounded-") {
		let value = utility.strip_prefix("rounded-").unwrap_or("sm");
		let length = match value {
			"none" => Ok(Length::Px(0.0)),
			"xs" => Ok(Length::Rem(0.125)),
			"sm" => Ok(Length::Rem(0.25)),
			"md" => Ok(Length::Rem(0.375)),
			"lg" => Ok(Length::Rem(0.5)),
			"xl" => Ok(Length::Rem(0.75)),
			"2xl" => Ok(Length::Rem(1.0)),
			"3xl" => Ok(Length::Rem(1.5)),
			"4xl" => Ok(Length::Rem(2.0)),
			"full" => Ok(Length::Px(9999.0)),
			_ => Length::parse(value, false, false).and_then(Length::absolute),
		};
		return Some(length.map(|length| (300, vec![Declaration::Radius(length)])));
	}

	if let Some(value) = utility.strip_prefix("opacity-") {
		let value = if let Some(value) = arbitrary(value) {
			value.strip_suffix('%').map_or_else(
				|| finite(value),
				|value| finite(value).map(|value| value / 100.0),
			)
		} else {
			finite(value).map(|value| value / 100.0)
		};
		return Some(value.and_then(|value| {
			if !(0.0..=1.0).contains(&value) {
				return Err("opacity must be between 0 and 100%".to_owned());
			}
			Ok((300, vec![Declaration::Opacity(value)]))
		}));
	}

	None
}

/// `border-t`, `border-l-2`, `border-x-[3px]`: a border on some edges only,
/// one pixel wide unless a width follows.
fn edge_border(utility: &str) -> Option<Parsed> {
	let rest = utility.strip_prefix("border-")?;
	let (side, width) = rest.split_once('-').unwrap_or((rest, ""));
	let edges: &'static [Edge] = match side {
		"t" => &[Edge::Top],
		"r" => &[Edge::Right],
		"b" => &[Edge::Bottom],
		"l" => &[Edge::Left],
		"x" => &[Edge::Left, Edge::Right],
		"y" => &[Edge::Top, Edge::Bottom],
		_ => return None,
	};

	let width = if width.is_empty() {
		Ok(Length::Px(1.0))
	} else if let Some(inner) = arbitrary(width) {
		Length::parse(inner.strip_prefix("length:").unwrap_or(inner), false, false)
			.and_then(Length::absolute)
	} else {
		non_negative(width).map(Length::Px)
	};

	// Single edges outrank axis pairs, as `border-l` beats `border-x`.
	let rank = if edges.len() == 1 { 303 } else { 302 };
	Some(width.map(|width| (rank, vec![Declaration::EdgeBorderWidth(edges, width)])))
}

fn grid(utility: &str) -> Option<Parsed> {
	for (prefix, axis) in [("grid-cols-", Axis::X), ("grid-rows-", Axis::Y)] {
		if let Some(value) = utility.strip_prefix(prefix) {
			return Some(
				tracks(value).map(|tracks| (100, vec![Declaration::GridTracks(axis, tracks)])),
			);
		}
	}

	for (prefix, axis) in [("auto-cols-", Axis::X), ("auto-rows-", Axis::Y)] {
		if let Some(value) = utility.strip_prefix(prefix) {
			let track = if value == "fr" {
				Ok(fraction_track())
			} else {
				parse_track(arbitrary(value).unwrap_or(value))
			};
			return Some(track.map(|track| (100, vec![Declaration::GridAutoTrack(axis, track)])));
		}
	}

	if let Some(flow) = utility.strip_prefix("grid-flow-") {
		let flow = match flow {
			"row" => GridAutoFlow::Row,
			"col" => GridAutoFlow::Column,
			"dense" | "row-dense" => GridAutoFlow::RowDense,
			"col-dense" => GridAutoFlow::ColumnDense,
			_ => return Some(Err(format!("unsupported grid auto-flow `{flow}`"))),
		};
		return Some(Ok((100, vec![Declaration::GridAutoFlow(flow)])));
	}

	let negative = utility.starts_with('-');
	let utility = utility.strip_prefix('-').unwrap_or(utility);

	for (prefix, axis) in [("col-", Axis::X), ("row-", Axis::Y)] {
		if let Some(value) = utility.strip_prefix(prefix) {
			return Some(placement(value, axis, negative));
		}
	}

	None
}

fn placement(value: &str, axis: Axis, negative: bool) -> Parsed {
	if !negative && value == "auto" {
		return Ok((
			100,
			vec![Declaration::GridPlacement(axis, GridPlacement::AUTO)],
		));
	}

	if !negative && value == "span-full" {
		return Ok((
			100,
			vec![Declaration::GridPlacement(axis, GridPlacement::line(1, -1))],
		));
	}

	if !negative && let Some(value) = value.strip_prefix("span-") {
		let span = arbitrary(value)
			.unwrap_or(value)
			.parse::<u16>()
			.map_err(|_| "invalid grid span")?;

		if span == 0 || span > 10_000 {
			return Err("grid span must be between 1 and 10000".to_owned());
		}
		return Ok((
			100,
			vec![Declaration::GridPlacement(axis, GridPlacement::span(span))],
		));
	}

	for (prefix, start) in [("start-", true), ("end-", false)] {
		if let Some(value) = value.strip_prefix(prefix) {
			let line = if value == "auto" && !negative {
				GridLine::Auto
			} else {
				let line = arbitrary(value)
					.unwrap_or(value)
					.parse::<i32>()
					.map_err(|_| "invalid grid line")?;

				if line == 0 || !(-10_000..=10_000).contains(&line) {
					return Err("grid line must be nonzero and between -10000 and 10000".to_owned());
				}
				GridLine::Line(if negative { -line } else { line })
			};
			return Ok((101, vec![Declaration::GridLine(axis, start, line)]));
		}
	}

	Err(format!("unsupported grid placement `{value}`"))
}

fn tracks(value: &str) -> Result<Vec<Track>, String> {
	if value == "none" {
		return Ok(Vec::new());
	}

	if let Some(value) = arbitrary(value) {
		let value = value.replace('_', " ");
		let mut tracks = Vec::new();

		for value in split_top_level(&value, ' ')? {
			if let Some(repeat) = value
				.strip_prefix("repeat(")
				.and_then(|value| value.strip_suffix(')'))
			{
				let (count, repeated) = repeat
					.split_once(',')
					.ok_or("repeat requires a count and tracks")?;
				let count = count
					.trim()
					.parse::<usize>()
					.map_err(|_| "repeat count must be an integer")?;
				let repeated = split_top_level(repeated.trim(), ' ')?
					.into_iter()
					.map(parse_track)
					.collect::<Result<Vec<_>, _>>()?;

				if count == 0 || repeated.is_empty() || count.saturating_mul(repeated.len()) > 256 {
					return Err("grid templates support 1 to 256 tracks".to_owned());
				}

				for _ in 0..count {
					tracks.extend(repeated.iter().cloned());
				}
			} else {
				tracks.push(parse_track(value)?);
			}

			if tracks.len() > 256 {
				return Err("grid templates support at most 256 tracks".to_owned());
			}
		}

		if tracks.is_empty() {
			return Err("grid template cannot be empty".to_owned());
		}
		return Ok(tracks);
	}

	let count = value
		.parse::<usize>()
		.map_err(|_| "grid template requires a track count or arbitrary track list")?;

	if !(1..=256).contains(&count) {
		return Err("grid templates support 1 to 256 tracks".to_owned());
	}
	Ok(vec![fraction_track(); count])
}

fn fraction_track() -> Track {
	Track::MinMax(
		Box::new(Track::Length(Length::Px(0.0))),
		Box::new(Track::Fraction(1.0)),
	)
}

fn parse_track(value: &str) -> Result<Track, String> {
	let value = value.trim();

	if value == "auto" {
		return Ok(Track::Auto);
	}

	if let Some(value) = value.strip_suffix("fr") {
		return non_negative(value).map(Track::Fraction);
	}

	if let Some(value) = value
		.strip_prefix("minmax(")
		.and_then(|value| value.strip_suffix(')'))
	{
		let values = split_top_level(value, ',')?;

		if values.len() != 2 {
			return Err("minmax requires two track sizes".to_owned());
		}
		let min = parse_track(values[0])?;
		let max = parse_track(values[1])?;

		if matches!(min, Track::Fraction(_) | Track::MinMax(..)) || matches!(max, Track::MinMax(..))
		{
			return Err(
				"minmax minimum cannot be a fraction and bounds cannot nest minmax".to_owned(),
			);
		}

		return Ok(Track::MinMax(Box::new(min), Box::new(max)));
	}

	Length::parse(value, false, false).map(Track::Length)
}

fn split_top_level(value: &str, separator: char) -> Result<Vec<&str>, String> {
	let mut depth = 0u8;
	let mut start = 0;
	let mut parts = Vec::new();

	for (index, character) in value.char_indices() {
		match character {
			'(' => {
				depth = depth
					.checked_add(1)
					.filter(|depth| *depth <= 4)
					.ok_or("grid functions are nested too deeply")?;
			}
			')' => depth = depth.checked_sub(1).ok_or("unbalanced grid parentheses")?,
			character if character == separator && depth == 0 => {
				if index > start {
					parts.push(value[start..index].trim());
				} else if separator == ',' {
					return Err("grid function has an empty argument".to_owned());
				}
				start = index + character.len_utf8();
			}
			_ => {}
		}
	}

	if depth != 0 {
		return Err("unbalanced grid parentheses".to_owned());
	}

	if start < value.len() {
		parts.push(value[start..].trim());
	} else if separator == ',' {
		return Err("grid function has an empty argument".to_owned());
	}
	Ok(parts)
}

fn non_negative(value: &str) -> Result<f32, String> {
	let value = finite(arbitrary(value).unwrap_or(value))?;

	if value < 0.0 {
		return Err("value must be non-negative".to_owned());
	}
	Ok(value)
}
