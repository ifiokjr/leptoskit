//! Compare Yoga frames for Tailwind grid/flex trees and emit an HTML probe
//! the browser can measure against.
//!
//! ```sh
//! cargo run -p leptoskit_native --example grid_parity_probe --features web -- \
//!   /tmp/grid-parity
//! ```

#![allow(clippy::print_stdout)]

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use leptos::prelude::RenderHtml;
use leptoskit_native::Element;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeSurface;
use leptoskit_native::components::View;
use leptoskit_native::web::render;
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

fn write_frames(
	out: &mut String,
	surface: &NativeSurface<HeadlessBackend>,
	id: NodeId,
	depth: usize,
) {
	let frame = surface.frame(id).expect("frame");
	let indent = "  ".repeat(depth);
	let test_id = surface.props(id).test_id().unwrap_or("");
	let _ = writeln!(
		out,
		"{indent}{{\"test_id\":{test_id:?},\"x\":{},\"y\":{},\"width\":{},\"height\":{},\"\
		 children\":[",
		frame.x, frame.y, frame.width, frame.height
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
	let out = PathBuf::from(
		std::env::args()
			.nth(1)
			.unwrap_or_else(|| "target/grid-parity".to_owned()),
	);
	fs::create_dir_all(&out).expect("output dir");

	let mut index = String::from(
		r#"<!doctype html><html><head><meta charset="utf-8"/><title>grid parity</title>
<style>
html{font-size:16px;}
body{margin:0;background:#e2e8f0;font:14px system-ui,sans-serif;}
section{margin:24px;padding:16px;background:#fff;border-radius:12px;}
h2{margin:0 0 12px;font-size:16px;}
.probe{position:relative;outline:1px dashed #94a3b8;background:#f8fafc;}
</style></head><body>
"#,
	);

	let mut native = String::from("{\n");

	for (fixture_index, fixture) in fixtures().into_iter().enumerate() {
		let mut surface = NativeSurface::new(HeadlessBackend::new());
		surface.set_viewport(fixture.width, fixture.height);
		surface.set_root(&fixture.tree);
		let root = surface.root().expect("root");
		if fixture_index > 0 {
			native.push_str(",\n");
		}
		let _ = write!(native, "  {:?}: ", fixture.name);
		write_frames(&mut native, &surface, root, 1);

		let html = render(fixture.tree).to_html();
		let _ = writeln!(
			index,
			"<section data-fixture=\"{name}\"><h2>{name} ({w}×{h})</h2><div class=\"probe\" \
			 style=\"width:{w}px;height:{h}px\">{html}</div></section>",
			name = fixture.name,
			w = fixture.width,
			h = fixture.height,
			html = html,
		);
		let frame = surface.frame(root).unwrap();
		println!(
			"fixture {} native root {}x{} at {},{}",
			fixture.name, frame.width, frame.height, frame.x, frame.y
		);
	}

	native.push_str("\n}\n");
	index.push_str("</body></html>");
	fs::write(out.join("index.html"), index).expect("write html");
	fs::write(out.join("native-frames.json"), native).expect("write json");
	println!("wrote {}", out.join("index.html").display());
}
