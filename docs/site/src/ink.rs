//! Seeded artwork from skribble's Rust engine, served with the book.
//! Like kit.pina.rs, frames use non-scaling strokes in SVG masks so the
//! same ink follows the page palette at every screen size.
use skribble_rough::DrawConfig;
use skribble_rough::DrawOptions;
use skribble_rough::FillOptions;
use skribble_rough::FillStyle;
use skribble_rough::Filler;
use skribble_rough::Generator;
use skribble_rough::PathElementOptions;
use skribble_rough::Point;
use skribble_rough::RoughSvgPaths;
use skribble_rough::Shape;
use skribble_rough::SketchOptions;

fn generator(seed: i64, fill: FillStyle) -> Generator {
	let options = DrawOptions {
		seed,
		roughness: 1.2,
		..DrawOptions::default()
	};
	Generator::new(
		DrawConfig::new(options),
		Filler::new(
			fill,
			FillOptions {
				hachure_gap: 10.0,
				..FillOptions::default()
			},
			DrawConfig::new(options),
		),
	)
}

fn svg(paths: &RoughSvgPaths, width: f64, height: f64, color: &str) -> String {
	let elements = paths.to_path_elements(&PathElementOptions {
		stroke: color,
		fill: color,
		stroke_width: 2.0,
		sketch_width: 1.5,
		non_scaling_stroke: true,
	});
	format!(
		r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" preserveAspectRatio="none">{elements}</svg>"#
	)
}

/// Generated assets shared by the public landing page and the entire book.
/// Names and seeds are stable, and the source tree stays read-only.
pub fn files() -> Vec<(String, Vec<u8>)> {
	let mut files = Vec::new();
	for variant in 1..=4 {
		for (name, width, height) in [
			("box", 240.0, 120.0),
			("card", 320.0, 200.0),
			("circle", 120.0, 120.0),
			("rule", 600.0, 12.0),
			("edge", 12.0, 600.0),
			("underline", 200.0, 40.0),
		] {
			let mut pen = generator(variant * 101, FillStyle::None);
			let drawing = match name {
				"circle" => pen.ellipse(60.0, 60.0, 112.0, 112.0),
				"rule" => pen.line(4.0, 6.0, 596.0, 6.0),
				"edge" => pen.line(6.0, 4.0, 6.0, 596.0),
				"underline" => {
					pen.curve(&[
						Point::new(3.0, 28.0),
						Point::new(70.0, 25.0),
						Point::new(140.0, 28.0),
						Point::new(197.0, 23.0),
					])
				}
				_ => pen.rectangle(4.0, 4.0, width - 8.0, height - 8.0),
			};
			let paths = RoughSvgPaths::from_drawable(&drawing);
			files.push((
				format!("{name}-{variant}.svg"),
				svg(&paths, width, height, "#000").into_bytes(),
			));
		}
		let mut hatch = RoughSvgPaths::from_drawable(
			&generator(variant * 101, FillStyle::Hachure).rectangle(4.0, 4.0, 232.0, 112.0),
		);
		hatch.outline.clear();
		files.push((
			format!("hatch-{variant}.svg"),
			svg(&hatch, 240.0, 120.0, "#000").into_bytes(),
		));
		let frame = RoughSvgPaths::from_drawable(
			&generator(variant * 101 + 7, FillStyle::None).rectangle(4.0, 4.0, 312.0, 192.0),
		);
		for (theme, color) in [("light", "#34283f"), ("dark", "#f3ebdf")] {
			files.push((
				format!("frame-{variant}-{theme}.svg"),
				svg(&frame, 320.0, 200.0, color).into_bytes(),
			));
		}
	}
	for (name, width, height, color, seed) in [
		("brand-tall", 125.0, 290.0, "#e2674d", 21),
		("brand-square", 125.0, 125.0, "#e2674d", 22),
		("brand-mint", 125.0, 125.0, "#3fb3a4", 23),
	] {
		let options = SketchOptions {
			draw: DrawOptions {
				seed,
				roughness: 0.9,
				..DrawOptions::default()
			},
			fill: Some(color.into()),
			fill_style: Some(FillStyle::Solid),
			stroke: "#34283f".into(),
			stroke_width: 2.0,
			..SketchOptions::default()
		};
		let mark = skribble_rough::sketch(
			&[Shape::rounded_rectangle(
				6.0,
				6.0,
				width - 12.0,
				height - 12.0,
				12.0,
			)],
			&options,
		)
		.expect("fixed finite brand geometry");
		files.push((
			format!("{name}.svg"),
			mark.to_svg_string(width, height, None).into_bytes(),
		));
	}
	files
}
