//! The cost of vector drawings in the parts of a frame that scale with their
//! size: parsing, recommitting an unchanged screen, and resolving styles.
//!
//! - `parse`: SVG path data with `points` cubic segments.
//! - `recommit_shared`: a screen of 64 icons that reuse one parsed drawing,
//!   committed again unchanged; diffing compares drawings by pointer.
//! - `recommit_rebuilt`: the same screen with every drawing rebuilt from its
//!   parsed path each render, so diffing compares paths by value.
//! - `resolve`: resolving inherited styles for a tree holding one large
//!   drawing, which clones its props.
//!
//! Run with `cargo bench -p leptoskit_native --bench vector`.

use std::fmt::Write;

use divan::Bencher;
use leptoskit_native::Color;
use leptoskit_native::Drawing;
use leptoskit_native::Element;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeSurface;
use leptoskit_native::PathData;
use leptoskit_native::Stroke;
use leptoskit_native::StyleContext;
use leptoskit_native::VectorPath;
use leptoskit_native::ViewBox;
use leptoskit_native::components::Vector;
use leptoskit_native::components::View;

fn main() {
	divan::main();
}

const POINTS: &[usize] = &[100, 1_000, 10_000];

/// A wavy line of `points` cubic segments, like generated rough ink.
fn wave(points: usize) -> String {
	let mut data = String::from("M0 50");

	for index in 0..points {
		let x = index as f32;
		write!(data, " C{} 0 {} 100 {} 50", x + 0.25, x + 0.75, x + 1.0).expect("write to String");
	}

	data
}

fn drawing(points: usize) -> Drawing {
	let ink = Color::rgba(17, 17, 17, 255);
	Drawing::new(ViewBox::new(0.0, 0.0, points as f32, 100.0).expect("a valid view box")).path(
		VectorPath::new(PathData::parse(&wave(points)).expect("valid data"))
			.stroke(Stroke::new(ink, 2.0).expect("a positive width")),
	)
}

fn screen(icons: impl Fn() -> Drawing) -> Element {
	View()
		.class("flex-row flex-wrap")
		.children((0..64).map(|_| Vector(ViewBox::UNIT).drawing(icons()).class("w-6 h-6")))
}

#[divan::bench(args = POINTS)]
fn parse(bencher: Bencher<'_, '_>, points: usize) {
	let data = wave(points);
	bencher.bench(|| PathData::parse(divan::black_box(&data)));
}

#[divan::bench(args = POINTS)]
fn recommit_shared(bencher: Bencher<'_, '_>, points: usize) {
	let icon = drawing(points);
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(400.0, 800.0);
	surface.set_root(&screen(|| icon.clone()));

	bencher.bench_local(|| {
		surface.set_root(&screen(|| icon.clone()));
		surface.backend_mut().clear();
	});
}

#[divan::bench(args = POINTS)]
fn recommit_rebuilt(bencher: Bencher<'_, '_>, points: usize) {
	let icon = drawing(points);
	let rebuild = || {
		icon.paths()
			.iter()
			.fold(Drawing::new(icon.view_box()), |drawing, path| {
				drawing.path(VectorPath::new(
					PathData::from_commands(path.data().commands().iter().copied())
						.expect("valid commands"),
				))
			})
	};
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(400.0, 800.0);
	surface.set_root(&screen(rebuild));

	bencher.bench_local(|| {
		surface.set_root(&screen(rebuild));
		surface.backend_mut().clear();
	});
}

#[divan::bench(args = POINTS)]
fn resolve(bencher: Bencher<'_, '_>, points: usize) {
	let tree = View().child(Vector(ViewBox::UNIT).drawing(drawing(points)));
	let context = StyleContext::default();
	bencher.bench(|| divan::black_box(&tree).resolved(&context));
}
