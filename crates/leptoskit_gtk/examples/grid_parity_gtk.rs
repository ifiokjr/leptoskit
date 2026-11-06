//! Mount the `grid_parity_probe` fixtures on GTK and compare widget
//! allocations to Yoga frames. Writes JSON + PNGs under the given directory.
//!
//! ```sh
//! dbus-run-session -- xvfb-run -a env GDK_BACKEND=x11 GSK_RENDERER=cairo \
//!   cargo run -p leptoskit_gtk --example grid_parity_gtk -- /tmp/grid-parity-gtk
//! ```

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use gtk::prelude::*;
use leptoskit_gtk::GtkBackend;
use leptoskit_gtk::gtk;
use leptoskit_native::Element;
use leptoskit_native::NativeSurface;
use leptoskit_native::components::View;
use leptoskit_yoga::NodeId;

struct Fixture {
	name: &'static str,
	width: f32,
	height: f32,
	tree: Element,
}

fn cell(id: &str, class: &str) -> Element {
	View().class(class).test_id(id)
}

/// Same trees as `leptoskit_native`'s `grid_parity_probe` example.
#[allow(
	clippy::too_many_lines,
	reason = "parity fixtures are intentionally exhaustive class trees"
)]
fn fixtures() -> Vec<Fixture> {
	vec![
		Fixture {
			name: "grid-cols-3-gap",
			width: 400.0,
			height: 200.0,
			tree: View()
				.class("grid grid-cols-3 grid-rows-2 w-[400px] h-[200px] p-4 gap-4")
				.children([
					cell("cell-0", "bg-slate-200"),
					cell("cell-1", "bg-slate-200"),
					cell("cell-2", "bg-slate-200"),
					cell("cell-3", "bg-slate-200"),
					cell("cell-4", "bg-slate-200"),
					cell("cell-5", "bg-slate-200"),
				]),
		},
		Fixture {
			name: "grid-span-and-start",
			width: 400.0,
			height: 240.0,
			tree: View()
				.class("grid grid-cols-4 grid-rows-3 w-[400px] h-[240px] gap-2")
				.children([
					cell("span", "col-span-2 row-span-2 bg-indigo-200"),
					cell("start", "col-start-3 bg-rose-200"),
					cell("corner", "col-start-4 row-start-2 bg-emerald-200"),
					cell("wide", "col-span-2 bg-amber-200"),
				]),
		},
		Fixture {
			name: "grid-align-justify-items",
			width: 300.0,
			height: 200.0,
			tree: View()
				.class(
					"grid grid-cols-2 grid-rows-2 w-[300px] h-[200px] gap-4 place-items-center \
					 bg-slate-50",
				)
				.children([
					cell("dot-0", "w-10 h-10 bg-blue-500"),
					cell("dot-1", "w-10 h-10 bg-blue-500"),
					cell("dot-2", "w-10 h-10 bg-blue-500"),
					cell("dot-3", "w-10 h-10 bg-blue-500"),
				]),
		},
		Fixture {
			name: "flex-row-gap-grow",
			width: 400.0,
			height: 80.0,
			tree: View()
				.class("flex-row w-[400px] h-20 p-2 gap-3 items-stretch")
				.children([
					cell("fixed", "w-16 bg-slate-300"),
					cell("grow", "flex-1 bg-slate-400"),
					cell("tail", "w-20 bg-slate-500"),
				]),
		},
		Fixture {
			name: "responsive-lg-grid",
			width: 1024.0,
			height: 320.0,
			tree: View()
				.class("grid grid-cols-1 lg:grid-cols-3 gap-5 w-[1024px] h-[320px] p-6")
				.children([
					cell("card-0", "bg-white border border-slate-200 rounded-xl"),
					cell("card-1", "bg-white border border-slate-200 rounded-xl"),
					cell("card-2", "bg-white border border-slate-200 rounded-xl"),
				]),
		},
		Fixture {
			name: "nested-grid-in-flex",
			width: 480.0,
			height: 240.0,
			tree: View()
				.class("flex-row w-[480px] h-[240px] gap-4 p-4")
				.children([
					cell("side", "w-40 bg-slate-100"),
					View()
						.class("flex-1 grid grid-cols-2 grid-rows-2 gap-2")
						.test_id("nest")
						.children([
							cell("nest-0", "bg-indigo-100"),
							cell("nest-1", "bg-indigo-100"),
							cell("nest-2", "bg-indigo-100"),
							cell("nest-3", "bg-indigo-100"),
						]),
				]),
		},
		Fixture {
			name: "nested-grids-deep",
			width: 480.0,
			height: 320.0,
			tree: View()
				.class("grid grid-cols-2 gap-3 w-[480px] h-[320px] p-3 bg-slate-50")
				.children([
					View()
						.class("grid grid-cols-2 grid-rows-2 gap-2 bg-white")
						.test_id("left")
						.children([
							cell("l0", "bg-indigo-200"),
							cell("l1", "bg-indigo-300"),
							View()
								.class("col-span-2 grid grid-cols-3 gap-1")
								.test_id("l-inner")
								.children([
									cell("li0", "bg-sky-200"),
									cell("li1", "bg-sky-300"),
									cell("li2", "bg-sky-400"),
								]),
						]),
					View()
						.class("grid grid-rows-3 gap-2")
						.test_id("right")
						.children([
							cell("r0", "bg-amber-200"),
							cell("r1", "bg-amber-300"),
							cell("r2", "bg-amber-400"),
						]),
				]),
		},
		Fixture {
			name: "grid-flow-dense",
			width: 360.0,
			height: 240.0,
			tree: View()
				.class(
					"grid grid-cols-3 grid-rows-3 grid-flow-dense gap-2 w-[360px] h-[240px] p-2 \
					 bg-slate-100",
				)
				.children([
					cell("big", "col-span-2 row-span-2 bg-indigo-300"),
					cell("a", "bg-rose-200"),
					cell("b", "bg-rose-300"),
					cell("c", "bg-rose-400"),
					cell("d", "bg-emerald-200"),
				]),
		},
		Fixture {
			name: "grid-flow-col",
			width: 300.0,
			height: 240.0,
			tree: View()
				.class("grid grid-rows-2 grid-flow-col auto-cols-fr gap-3 w-[300px] h-[240px] p-3")
				.children([
					cell("c0", "bg-violet-200"),
					cell("c1", "bg-violet-300"),
					cell("c2", "bg-violet-400"),
					cell("c3", "bg-violet-500"),
					cell("c4", "bg-violet-600"),
				]),
		},
		Fixture {
			name: "grid-minmax-tracks",
			width: 480.0,
			height: 160.0,
			tree: View()
				.class(
					"grid grid-cols-[minmax(80px,1fr)_minmax(40px,2fr)_100px] grid-rows-1 gap-x-4 \
					 gap-y-2 w-[480px] h-[160px] p-4 bg-slate-50",
				)
				.children([
					cell("min", "bg-teal-200"),
					cell("flexy", "bg-teal-300"),
					cell("fixed", "bg-teal-400"),
				]),
		},
		Fixture {
			name: "flex-shrink-basis-edges",
			width: 400.0,
			height: 96.0,
			tree: View()
				.class("flex-row w-[400px] h-24 p-2 gap-2 items-center bg-slate-100")
				.children([
					cell("basis", "basis-32 shrink grow-0 bg-orange-200 h-16"),
					cell("noshrink", "w-40 shrink-0 bg-orange-300 h-12"),
					cell("grow2", "grow-[2] shrink basis-0 bg-orange-400 h-20"),
					cell("grow1", "grow shrink basis-0 bg-orange-500 h-10"),
				]),
		},
		Fixture {
			name: "flex-wrap-justify-between",
			width: 320.0,
			height: 200.0,
			tree: View()
				.class(
					"flex-row flex-wrap justify-between content-start gap-x-3 gap-y-4 w-[320px] \
					 h-[200px] p-3 bg-slate-50",
				)
				.children([
					cell("a", "w-28 h-12 bg-fuchsia-200"),
					cell("b", "w-28 h-12 bg-fuchsia-300"),
					cell("c", "w-28 h-12 bg-fuchsia-400"),
					cell("d", "w-28 h-12 bg-fuchsia-500"),
					cell("e", "w-28 h-12 bg-fuchsia-600"),
				]),
		},
		Fixture {
			name: "absolute-inset-in-relative",
			width: 320.0,
			height: 200.0,
			tree: View()
				.class("relative w-[320px] h-[200px] p-6 bg-slate-200")
				.children([
					cell("bg", "w-full h-full bg-slate-100"),
					cell(
						"abs",
						"absolute top-4 right-4 bottom-8 left-8 bg-indigo-400",
					),
					cell("badge", "absolute top-2 left-2 w-10 h-10 bg-rose-500"),
				]),
		},
		Fixture {
			name: "percent-sizes-and-margins",
			width: 400.0,
			height: 240.0,
			tree: View()
				.class("flex-col w-[400px] h-[240px] p-[10%] gap-2 bg-slate-50")
				.children([
					cell("half", "w-1/2 h-1/4 bg-lime-300"),
					cell("third", "w-1/3 h-[30%] ml-[10%] bg-lime-400"),
					cell("full", "w-full h-8 bg-lime-500"),
				]),
		},
		Fixture {
			name: "padding-margin-gap-mix",
			width: 360.0,
			height: 200.0,
			tree: View()
				.class(
					"flex-row items-stretch w-[360px] h-[200px] pt-3 pb-5 px-4 gap-x-5 \
					 bg-slate-100",
				)
				.children([
					cell("a", "w-16 m-2 bg-cyan-200"),
					cell("b", "flex-1 mx-3 my-1 bg-cyan-300"),
					cell("c", "w-20 mt-4 mb-2 bg-cyan-400"),
				]),
		},
		Fixture {
			name: "overflow-hidden-child",
			width: 280.0,
			height: 160.0,
			tree: View()
				.class("relative w-[280px] h-[160px] p-4 overflow-hidden bg-slate-200")
				.children([
					cell("inner", "w-[400px] h-[200px] bg-yellow-300"),
					cell("abs", "absolute -left-6 top-8 w-24 h-24 bg-yellow-500"),
				]),
		},
		Fixture {
			name: "percent-in-grid",
			width: 420.0,
			height: 210.0,
			tree: View()
				.class("grid grid-cols-2 grid-rows-2 gap-[5%] w-[420px] h-[210px] p-[5%]")
				.children([
					cell("a", "w-full h-full bg-pink-200"),
					cell("b", "w-3/4 h-1/2 self-center justify-self-end bg-pink-300"),
					cell("c", "w-1/2 h-full bg-pink-400"),
					cell("d", "w-full h-3/4 self-end bg-pink-500"),
				]),
		},
	]
}

fn pump() {
	while gtk::glib::MainContext::default().iteration(false) {}
}

fn pump_until(condition: impl Fn() -> bool) {
	let deadline = std::time::Instant::now() + Duration::from_secs(5);
	while !condition() {
		assert!(
			std::time::Instant::now() < deadline,
			"GTK window did not settle within five seconds"
		);
		pump();
	}
}

struct Mismatch {
	fixture: String,
	test_id: String,
	kind: String,
	yoga: String,
	gtk: String,
}

fn collect_mismatches(
	surface: &NativeSurface<GtkBackend>,
	id: NodeId,
	parent_view: &gtk::Widget,
	fixture: &str,
	out: &mut Vec<Mismatch>,
) {
	let layout = surface.layout(id);
	let view = surface.backend().view(id).expect("gtk view");
	let test_id = surface.props(id).test_id().unwrap_or("").to_owned();
	let width = view.width();
	let height = view.height();
	let expect_w = layout.width.round() as i32;
	let expect_h = layout.height.round() as i32;
	if width != expect_w || height != expect_h {
		out.push(Mismatch {
			fixture: fixture.to_owned(),
			test_id: test_id.clone(),
			kind: "allocation".into(),
			yoga: format!("{expect_w}x{expect_h}"),
			gtk: format!("{width}x{height}"),
		});
	}
	// Yoga frames are parent-relative; compare GTK bounds against the parent.
	if let Some(bounds) = view.compute_bounds(parent_view)
		&& ((bounds.x() - layout.x).abs() > 0.5 || (bounds.y() - layout.y).abs() > 0.5)
	{
		out.push(Mismatch {
			fixture: fixture.to_owned(),
			test_id: test_id.clone(),
			kind: "origin".into(),
			yoga: format!("{},{}", layout.x, layout.y),
			gtk: format!("{},{}", bounds.x(), bounds.y()),
		});
	}
	for child in surface.children(id) {
		collect_mismatches(surface, *child, &view, fixture, out);
	}
}

fn write_frames(out: &mut String, surface: &NativeSurface<GtkBackend>, id: NodeId, depth: usize) {
	use std::fmt::Write as _;
	let layout = surface.layout(id);
	let indent = "  ".repeat(depth);
	let test_id = surface.props(id).test_id().unwrap_or("");
	let _ = writeln!(
		out,
		"{indent}{{\"test_id\":{test_id:?},\"x\":{},\"y\":{},\"width\":{},\"height\":{},\"\
		 children\":[",
		layout.x, layout.y, layout.width, layout.height
	);
	let children = surface.children(id);
	for (index, child) in children.iter().enumerate() {
		write_frames(out, surface, *child, depth + 1);
		if index + 1 != children.len() {
			out.push(',');
		}
		out.push('\n');
	}
	let _ = write!(out, "{indent}]}}");
}

fn main() {
	gtk::init().expect("GTK needs a display; run under dbus-run-session and xvfb-run");
	let out = PathBuf::from(
		std::env::args()
			.nth(1)
			.unwrap_or_else(|| "target/grid-parity-gtk".to_owned()),
	);
	fs::create_dir_all(&out).expect("output dir");
	fs::create_dir_all(out.join("png")).expect("png dir");

	let mut frames = String::from("{\n");
	let mut mismatches = Vec::new();
	let fixtures = fixtures();

	for (index, fixture) in fixtures.iter().enumerate() {
		let mut surface = NativeSurface::new(GtkBackend::new());
		surface.set_dark_mode(false);
		surface.set_viewport(fixture.width, fixture.height);
		surface.set_root(&fixture.tree);
		let root = surface.root().expect("root");
		let window = surface
			.backend()
			.open_window(
				root,
				&format!("grid-parity-{}", fixture.name),
				fixture.width.ceil() as i32,
				fixture.height.ceil() as i32,
			)
			.expect("window");
		let root_view = surface.backend().view(root).expect("root view");
		pump_until(|| window.is_mapped() && root_view.width() > 0);

		// Root is parented by the window content; check size only, then walk
		// children with parent-relative origins.
		let layout = surface.layout(root);
		if root_view.width() != layout.width.round() as i32
			|| root_view.height() != layout.height.round() as i32
		{
			mismatches.push(Mismatch {
				fixture: fixture.name.to_owned(),
				test_id: String::new(),
				kind: "allocation".into(),
				yoga: format!(
					"{}x{}",
					layout.width.round() as i32,
					layout.height.round() as i32
				),
				gtk: format!("{}x{}", root_view.width(), root_view.height()),
			});
		}
		for child in surface.children(root) {
			collect_mismatches(&surface, *child, &root_view, fixture.name, &mut mismatches);
		}

		if index > 0 {
			frames.push_str(",\n");
		}
		use std::fmt::Write as _;
		let _ = write!(frames, "  {:?}: ", fixture.name);
		write_frames(&mut frames, &surface, root, 1);

		let png = surface.backend().snapshot_png(root).expect("snapshot");
		fs::write(out.join("png").join(format!("{}.png", fixture.name)), png).expect("write png");

		println!(
			"fixture {} gtk root {}x{}",
			fixture.name,
			root_view.width(),
			root_view.height()
		);
		surface.backend().clear_event_listener();
		window.destroy();
		pump();
		drop(surface);
	}

	frames.push_str("\n}\n");
	fs::write(out.join("gtk-yoga-frames.json"), frames).expect("write frames");

	let report = serde_json_lite(&mismatches);
	fs::write(out.join("gtk-allocation-diff.json"), report).expect("write diff");
	println!(
		"wrote {} ({} allocation mismatches)",
		out.display(),
		mismatches.len()
	);
	if !mismatches.is_empty() {
		for item in &mismatches {
			eprintln!(
				"mismatch {} {} {}: yoga={} gtk={}",
				item.fixture, item.test_id, item.kind, item.yoga, item.gtk
			);
		}
		std::process::exit(1);
	}
}

fn serde_json_lite(mismatches: &[Mismatch]) -> String {
	use std::fmt::Write as _;
	let mut out = String::from("{\n  \"count\": ");
	let _ = write!(out, "{},\n  \"mismatches\": [\n", mismatches.len());
	for (index, item) in mismatches.iter().enumerate() {
		let _ = writeln!(
			out,
			"    {{\"fixture\":{:?},\"test_id\":{:?},\"kind\":{:?},\"yoga\":{:?},\"gtk\":{:?}}}{}",
			item.fixture,
			item.test_id,
			item.kind,
			item.yoga,
			item.gtk,
			if index + 1 == mismatches.len() {
				""
			} else {
				","
			}
		);
	}
	out.push_str("  ]\n}\n");
	out
}
