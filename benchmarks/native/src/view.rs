//! The view layer: `leptoskit_native`'s surface and `leptoskit_yoga`'s layout,
//! measured directly on the host or device CPU with the headless backend.
//!
//! The fixture is the same 1,000-row list the browser benchmark renders —
//! a column of rows, each a `View` holding one `Text` — so the scenarios line
//! up with the keyed-table set: build, commit, update every tenth label,
//! remove a row, relayout, and the input paths (dispatch, hit test).

use leptoskit_native::Element;
use leptoskit_native::EventOutcome;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeEventSlot;
use leptoskit_native::NativeSurface;
use leptoskit_native::tailwind::StyleContext;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::Display;
use leptoskit_yoga::FlexDirection;
use leptoskit_yoga::GridStyle;
use leptoskit_yoga::Rect;
use leptoskit_yoga::Size;
use leptoskit_yoga::Style;
use leptoskit_yoga::TrackSizing;
use leptoskit_yoga::Yoga;

use crate::report::Config;
use crate::report::Report;
use crate::report::sample;

pub const ROWS: usize = 1000;
pub const VIEWPORT: (f32, f32) = (390.0, 844.0);
const DISPATCHES: usize = 1000;
const HIT_TESTS: usize = 1000;

fn row_style(style: &mut Style) {
	style.flex_direction = FlexDirection::Row;
	style.padding = Rect {
		top: Dimension::Points(4.0),
		right: Dimension::Points(8.0),
		bottom: Dimension::Points(4.0),
		left: Dimension::Points(8.0),
	};
}

/// The list fixture. `generation` suffixes every tenth label so two
/// generations differ exactly as the browser's "update every 10th" does;
/// `skip` leaves one row out so a removal can be diffed.
pub fn rows_tree(rows: usize, generation: u32, skip: Option<usize>) -> Element {
	let mut root = Element::view().with_style(|style| {
		style.flex_direction = FlexDirection::Column;
	});
	for row in 0..rows {
		if skip == Some(row) {
			continue;
		}
		let label = if generation > 0 && row % 10 == 0 {
			format!("Row {row} · gen {generation}")
		} else {
			format!("Row {row}")
		};
		root = root.child(
			Element::view()
				.with_style(row_style)
				.child(Element::text(label).font(14.0, leptoskit_native::FontWeight::Regular)),
		);
	}
	root
}

/// The same list with Tailwind classes instead of inline styles, so the
/// resolver has real work per node.
fn classed_rows_tree(rows: usize) -> Element {
	let mut root = Element::view().class("flex-col");
	for row in 0..rows {
		root = root.child(
			Element::view()
				.class("flex-row gap-2 rounded bg-white")
				.child(Element::text(format!("Row {row}")).class("text-sm font-medium")),
		);
	}
	root
}

fn committed_surface(tree: &Element) -> NativeSurface<HeadlessBackend> {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(VIEWPORT.0, VIEWPORT.1);
	surface.set_root(tree);
	surface
}

/// Runs every view-layer scenario into `report`.
pub fn run(report: &mut Report, config: Config) {
	// Tree construction alone: builders, allocation, no layout.
	let build = sample(
		config,
		|| (),
		|()| {
			let tree = rows_tree(ROWS, 0, None);
			std::hint::black_box(tree);
		},
	);
	report.duration("native.build-1000", "build a 1,000-row element tree", build);

	// First commit: resolve, create every node, lay out, collect frames.
	let tree = rows_tree(ROWS, 0, None);
	let commit = sample(
		config,
		|| {
			let mut surface = NativeSurface::new(HeadlessBackend::new());
			surface.set_viewport(VIEWPORT.0, VIEWPORT.1);
			surface
		},
		|surface| surface.set_root(&tree),
	);
	report.duration("native.commit-1000", "first commit of 1,000 rows", commit);

	// Relayout: a viewport change re-lays out every node, nothing is rebuilt.
	let relayout = sample(
		config,
		|| committed_surface(&tree),
		|surface| surface.set_viewport(VIEWPORT.0 + 10.0, VIEWPORT.1 + 20.0),
	);
	report.duration(
		"native.relayout-1000",
		"relayout 1,000 rows after a viewport change",
		relayout,
	);

	// Partial update: every tenth label changes; the diff must find them.
	let updated = rows_tree(ROWS, 1, None);
	let update = sample(
		config,
		|| committed_surface(&tree),
		|surface| {
			surface.set_root(&updated);
		},
	);
	report.duration(
		"native.update-every-10th-1000",
		"diff and patch every tenth label of 1,000 rows",
		update,
	);

	// Removal from the middle: the reconciler matches children by index, so
	// everything after the gap shifts — this is the cost of that choice.
	let shorter = rows_tree(ROWS, 0, Some(ROWS / 2));
	let remove = sample(
		config,
		|| committed_surface(&tree),
		|surface| {
			surface.set_root(&shorter);
		},
	);
	report.duration(
		"native.remove-row-1000",
		"remove the middle row of 1,000",
		remove,
	);

	// Tailwind resolution: the per-commit pass that turns classes into styles.
	let classed = classed_rows_tree(ROWS);
	let context = StyleContext {
		viewport: Size {
			width: VIEWPORT.0,
			height: VIEWPORT.1,
		},
		dark: false,
	};
	let resolve = sample(
		config,
		|| (),
		|()| {
			std::hint::black_box(classed.resolved(&context));
		},
	);
	report.duration(
		"native.tailwind-resolve-1000",
		"resolve Tailwind classes on 1,000 rows",
		resolve,
	);

	// Event dispatch: a press on a deep leaf bubbles row → root, where a
	// handler claims it.
	let handled = rows_tree(ROWS, 0, None).on(NativeEventSlot::Press, |_| EventOutcome::Handled);
	let dispatch = sample(
		config,
		|| {
			let surface = committed_surface(&handled);
			let root = surface.root().expect("committed root");
			let row = surface.children(root)[ROWS / 2];
			let leaf = surface.children(row)[0];
			(surface, leaf)
		},
		|(surface, leaf)| {
			for _ in 0..DISPATCHES {
				std::hint::black_box(surface.dispatch_event(*leaf, &NativeEvent::Press));
			}
		},
	);
	report.throughput(
		"native.dispatch-throughput",
		"press events dispatched per second through a 3-level bubble",
		DISPATCHES as f64,
		&dispatch,
	);
	report.duration(
		"native.dispatch-1000",
		"dispatch 1,000 press events",
		dispatch,
	);

	// Hit testing across the viewport: the deepest frame under each point.
	let hit = sample(
		config,
		|| committed_surface(&tree),
		|surface| {
			for index in 0..HIT_TESTS {
				let y = (index as f32 / HIT_TESTS as f32) * VIEWPORT.1;
				std::hint::black_box(surface.hit_test(VIEWPORT.0 / 2.0, y));
			}
		},
	);
	report.duration(
		"native.hit-test-1000",
		"1,000 hit tests across the viewport",
		hit,
	);

	run_yoga(report, config);
}

fn flat_tree(rows: usize) -> (Yoga, leptoskit_yoga::NodeId) {
	let mut yoga = Yoga::new();
	let root = yoga.create_node();
	yoga.set_style(
		root,
		Style {
			flex_direction: FlexDirection::Column,
			..Style::default()
		},
	);
	for index in 0..rows {
		let row = yoga.create_node();
		yoga.set_style(
			row,
			Style {
				height: Dimension::Points(24.0),
				width: Dimension::Percent(100.0),
				..Style::default()
			},
		);
		yoga.insert_child(root, row, index);
	}
	(yoga, root)
}

fn available() -> Size<AvailableSpace> {
	Size {
		width: AvailableSpace::Definite(VIEWPORT.0),
		height: AvailableSpace::Definite(VIEWPORT.1),
	}
}

fn run_yoga(report: &mut Report, config: Config) {
	// A fresh tree per sample: identical constraints on a clean tree are a
	// cached no-op by design, which is not what this measures.
	let flat = sample(
		config,
		|| flat_tree(ROWS),
		|(yoga, root)| yoga.calculate_layout(*root, available()),
	);
	report.duration("yoga.layout-flat-1000", "lay out a 1,000-row column", flat);

	let grid = sample(
		config,
		|| {
			let mut yoga = Yoga::new();
			let root = yoga.create_node();
			yoga.set_style(
				root,
				Style {
					display: Display::Grid,
					grid: Some(GridStyle {
						template_columns: vec![TrackSizing::Fraction(1.0); 20],
						..GridStyle::default()
					}),
					..Style::default()
				},
			);
			for index in 0..ROWS {
				let cell = yoga.create_node();
				yoga.set_style(
					cell,
					Style {
						height: Dimension::Points(24.0),
						..Style::default()
					},
				);
				yoga.insert_child(root, cell, index);
			}
			(yoga, root)
		},
		|(yoga, root)| yoga.calculate_layout(*root, available()),
	);
	report.duration(
		"yoga.layout-grid-1000",
		"lay out 1,000 cells in a 20-column grid",
		grid,
	);

	// Incremental: one dirty node in an otherwise clean, cached tree.
	let incremental = sample(
		config,
		|| {
			let (mut yoga, root) = flat_tree(ROWS);
			yoga.calculate_layout(root, available());
			let middle = yoga.children(root)[ROWS / 2];
			yoga.set_style(
				middle,
				Style {
					height: Dimension::Points(48.0),
					width: Dimension::Percent(100.0),
					..Style::default()
				},
			);
			(yoga, root)
		},
		|(yoga, root)| yoga.calculate_layout(*root, available()),
	);
	report.duration(
		"yoga.relayout-dirty-1",
		"relayout 1,000 rows after one row changed height",
		incremental,
	);
}
