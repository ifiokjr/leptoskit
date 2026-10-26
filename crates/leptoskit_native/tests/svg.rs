//! SVG documents imported into drawings.
#![cfg(feature = "svg")]
// Imported lengths are exact in binary floating point.
#![allow(clippy::float_cmp)]

use leptoskit_native::Color;
use leptoskit_native::Drawing;
use leptoskit_native::FillRule;
use leptoskit_native::LineCap;
use leptoskit_native::LineJoin;
use leptoskit_native::Paint;
use leptoskit_native::StrokeScaling;
use leptoskit_native::SvgError;
use leptoskit_native::VectorAlign;
use leptoskit_native::VectorAlignment;
use leptoskit_native::VectorScaling;

fn parse(document: &str) -> Drawing {
	Drawing::parse_svg(document).unwrap_or_else(|error| panic!("the document imports: {error}"))
}

#[test]
fn icon_sets_import_with_current_color_strokes() {
	// The shape of a Lucide icon.
	let drawing = parse(
		r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"
			fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"
			stroke-linejoin="round">
			<circle cx="12" cy="12" r="10"/>
			<path d="m9 12 2 2 4-4"/>
		</svg>"#,
	);

	assert_eq!(drawing.view_box().width(), 24.0);
	assert_eq!(drawing.paths().len(), 2);

	for path in drawing.paths() {
		let stroke = path.stroke_style().expect("every path is stroked");
		assert_eq!(path.fill_paint(), None);
		assert_eq!(stroke.paint(), Paint::CurrentColor);
		assert_eq!(stroke.width(), 2.0);
		assert_eq!(stroke.line_cap(), LineCap::Round);
		assert_eq!(stroke.line_join(), LineJoin::Round);
		// SVG strokes scale with the drawing unless they opt out.
		assert_eq!(stroke.stroke_scaling(), StrokeScaling::Drawing);
	}

	assert_eq!(
		drawing.paths()[1].data().to_string(),
		"M 9 12 L 11 14 L 15 10"
	);
}

#[test]
fn groups_inherit_styles_and_compose_transforms() {
	let drawing = parse(
		r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"
			preserveAspectRatio="xMinYMax slice">
			<title>Test</title>
			<defs><linearGradient id="unused"/></defs>
			<g fill="#ff0000" transform="translate(10 0)" opacity="0.5">
				<g transform="scale(2)" style="fill: rgb(0 0 255 / 50%); fill-rule: evenodd">
					<rect width="10" height="5" stroke="hsl(120, 100%, 25%)" stroke-width="3"
						stroke-dasharray="2 1 2" stroke-miterlimit="8"/>
				</g>
				<line x1="0" y1="0" x2="10" y2="0" stroke="navy" vector-effect="non-scaling-stroke"/>
				<rect width="0" height="10"/>
				<circle r="5" display="none"/>
				<circle r="5" visibility="hidden"/>
			</g>
		</svg>"##,
	);

	assert_eq!(drawing.vector_scaling(), VectorScaling::Fill);
	assert_eq!(
		drawing.alignment(),
		VectorAlignment::new(VectorAlign::Start, VectorAlign::End)
	);
	// The zero-width rect, the undisplayed circle, and the hidden one draw
	// nothing.
	assert_eq!(drawing.paths().len(), 2);

	let rect = &drawing.paths()[0];
	assert_eq!(rect.data().to_string(), "M 10 0 L 30 0 L 30 10 L 10 10 Z");
	assert_eq!(rect.rule(), FillRule::Evenodd);
	// 50% from the color, times the group's 0.5 opacity.
	assert_eq!(
		rect.fill_paint(),
		Some(Paint::Color(Color::rgba(0, 0, 255, 64)))
	);
	let stroke = rect.stroke_style().expect("the rect is stroked");
	assert_eq!(stroke.color(), Color::rgba(0, 128, 0, 128));
	// User-space lengths grow with the 2× scale.
	assert_eq!(stroke.width(), 6.0);
	assert_eq!(stroke.dash_lengths(), &[4.0, 2.0, 4.0, 4.0, 2.0, 4.0]);
	assert_eq!(stroke.miter(), 8.0);

	let line = &drawing.paths()[1];
	assert_eq!(line.data().to_string(), "M 10 0 L 20 0");
	let stroke = line.stroke_style().expect("the line is stroked");
	assert_eq!(stroke.stroke_scaling(), StrokeScaling::Fixed);
	assert_eq!(stroke.width(), 1.0);
	assert_eq!(stroke.color(), Color::rgba(0, 0, 0x80, 128));
}

#[test]
fn sizes_without_a_view_box_and_absolute_units() {
	let drawing = parse(
		r#"<svg xmlns="http://www.w3.org/2000/svg" width="1in" height="48px">
			<rect x="10%" y="50%" width="1pt" height="1pt" fill="black" rx="0.5pt"/>
			<polygon points="0,0 4,0 2,3 9"/>
		</svg>"#,
	);

	assert_eq!(drawing.view_box().width(), 96.0);
	assert_eq!(drawing.view_box().height(), 48.0);
	let rect = drawing.paths()[0].data();
	assert_eq!(
		rect.commands()[0],
		leptoskit_native::vector::PathCommand::MoveTo(leptoskit_native::vector::Point::new(
			9.6 + 2.0 / 3.0,
			24.0
		))
	);
	assert_eq!(drawing.paths()[1].data().to_string(), "M 0 0 L 4 0 L 2 3 Z");
}

#[test]
fn unsupported_features_are_errors_that_name_them() {
	let svg = |body: &str| {
		Drawing::parse_svg(&format!(
			r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">{body}</svg>"#
		))
	};

	assert_eq!(
		svg("<text>Hi</text>"),
		Err(SvgError::UnsupportedElement("text".to_owned()))
	);
	assert_eq!(
		svg(r#"<rect width="1" height="1" fill="url(#g)"/>"#),
		Err(SvgError::Unsupported {
			name: "fill".to_owned(),
			value: "url(#g)".to_owned()
		})
	);
	assert_eq!(
		svg(r#"<g clip-path="url(#c)"/>"#),
		Err(SvgError::Unsupported {
			name: "clip-path".to_owned(),
			value: "url(#c)".to_owned()
		})
	);
	assert_eq!(
		svg(r#"<rect width="1em" height="1"/>"#),
		Err(SvgError::Invalid {
			name: "width".to_owned(),
			value: "1em".to_owned()
		})
	);
	assert_eq!(
		svg(r#"<path d="M0 0 X"/>"#),
		Err(SvgError::Vector(leptoskit_native::VectorError::PathData {
			offset: 5
		}))
	);
	assert_eq!(
		Drawing::parse_svg(r#"<svg xmlns="http://www.w3.org/2000/svg"/>"#),
		Err(SvgError::MissingViewBox)
	);
	assert_eq!(Drawing::parse_svg("<html/>"), Err(SvgError::NotSvg));
	assert!(matches!(Drawing::parse_svg("<svg"), Err(SvgError::Xml(_))));
}

#[test]
fn css_colors_cover_the_common_syntaxes() {
	let fill = |value: &str| {
		parse(&format!(
			r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"><rect width="1" height="1" fill="{value}"/></svg>"#
		))
		.paths()[0]
			.fill_color()
	};

	assert_eq!(fill("#abc"), Some(Color::rgba(0xaa, 0xbb, 0xcc, 0xff)));
	assert_eq!(fill("#11223380"), Some(Color::rgba(0x11, 0x22, 0x33, 0x80)));
	assert_eq!(
		fill("rebeccapurple"),
		Some(Color::rgba(0x66, 0x33, 0x99, 0xff))
	);
	assert_eq!(
		fill("RGBA(255, 0, 0, 0.5)"),
		Some(Color::rgba(255, 0, 0, 128))
	);
	assert_eq!(
		fill("hsl(0deg 100% 50%)"),
		Some(Color::rgba(255, 0, 0, 255))
	);
	assert_eq!(fill("transparent"), Some(Color::rgba(0, 0, 0, 0)));
}
