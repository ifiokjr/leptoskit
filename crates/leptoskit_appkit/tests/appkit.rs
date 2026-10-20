//! AppKit backend tests.
//!
//! AppKit must run on the main thread, so this binary uses its own harness
//! and runs every test sequentially on the main thread. On other platforms
//! there is nothing to test and the binary reports zero tests.

// Sizes are compared exactly: they are whole points from the same arithmetic.
#![allow(clippy::float_cmp)]

use libtest_mimic::Arguments;

fn main() {
	let mut args = Arguments::from_args();
	// A single test thread is the only mode in which the harness runs tests
	// on the main thread itself.
	args.test_threads = Some(1);
	#[cfg(target_os = "macos")]
	let trials = macos::trials();
	#[cfg(not(target_os = "macos"))]
	let trials = Vec::new();
	libtest_mimic::run(&args, trials).exit();
}

#[cfg(target_os = "macos")]
mod macos {
	use std::cell::RefCell;
	use std::rc::Rc;
	use std::sync::Arc;
	use std::sync::atomic::AtomicUsize;
	use std::sync::atomic::Ordering;

	use leptoskit_appkit::AppKitBackend;
	use leptoskit_appkit::AppKitTextMeasurer;
	use leptoskit_appkit::MainThreadMarker;
	use leptoskit_appkit::SnapshotError;
	use leptoskit_native::Color;
	use leptoskit_native::Element;
	use leptoskit_native::FontWeight;
	use leptoskit_native::NativeBackend;
	use leptoskit_native::NativeEvent;
	use leptoskit_native::NativeKind;
	use leptoskit_native::NativeProps;
	use leptoskit_native::NativeSurface;
	use leptoskit_native::TextAlign;
	use leptoskit_native::TextMeasurer;
	use leptoskit_native::TextStyle;
	use leptoskit_yoga::Align;
	use leptoskit_yoga::AvailableSpace;
	use leptoskit_yoga::Dimension;
	use leptoskit_yoga::NodeId;
	use leptoskit_yoga::Size;
	use leptoskit_yoga::Yoga;
	use libtest_mimic::Trial;
	use objc2_app_kit::NSScrollView;
	use objc2_app_kit::NSTextField;
	use objc2_foundation::NSPoint;
	use objc2_foundation::NSRect;
	use objc2_foundation::NSSize;

	pub fn trials() -> Vec<Trial> {
		vec![
			trial("widget_tester_renders_appkit", widget_tester_renders_appkit),
			trial(
				"views_take_their_layout_frames",
				views_take_their_layout_frames,
			),
			trial(
				"insert_child_respects_the_index",
				insert_child_respects_the_index,
			),
			trial(
				"removed_children_leave_their_parent",
				removed_children_leave_their_parent,
			),
			trial("a_press_reaches_the_listener", a_press_reaches_the_listener),
			trial(
				"hidden_views_toggle_without_hiding_siblings",
				hidden_views_toggle_without_hiding_siblings,
			),
			trial(
				"clipping_updates_preserve_rounded_corners",
				clipping_updates_preserve_rounded_corners,
			),
			trial(
				"scroll_views_cover_overflow_on_both_axes",
				scroll_views_cover_overflow_on_both_axes,
			),
			trial(
				"labels_fit_their_measured_text",
				labels_fit_their_measured_text,
			),
			trial(
				"measurement_follows_the_style",
				measurement_follows_the_style,
			),
			trial(
				"logical_text_alignment_follows_layout_direction",
				logical_text_alignment_follows_layout_direction,
			),
			trial(
				"measurement_works_off_the_main_thread",
				measurement_works_off_the_main_thread,
			),
			trial(
				"snapshots_are_pngs_at_the_scale",
				snapshots_are_pngs_at_the_scale,
			),
			trial("snapshot_errors_are_reported", snapshot_errors_are_reported),
		]
	}

	/// A trial whose failures are panics, as with the standard harness.
	fn trial(name: &str, test: fn()) -> Trial {
		Trial::test(name, move || {
			test();
			Ok(())
		})
	}

	fn logical_text_alignment_follows_layout_direction() {
		use leptoskit_yoga::Direction;
		use objc2_app_kit::NSTextAlignment;
		use objc2_app_kit::NSTextField;
		use objc2_app_kit::NSWritingDirection;

		let mut surface = NativeSurface::new(AppKitBackend::new(mtm()));
		surface.set_viewport(240.0, 100.0);
		for (direction, align, expected) in [
			(Direction::Rtl, TextAlign::Start, NSTextAlignment::Right),
			(Direction::Rtl, TextAlign::End, NSTextAlignment::Left),
			(Direction::Ltr, TextAlign::End, NSTextAlignment::Right),
			(Direction::Ltr, TextAlign::Start, NSTextAlignment::Left),
			(Direction::Rtl, TextAlign::Center, NSTextAlignment::Center),
		] {
			surface.set_root(
				&Element::text_input()
					.value("hello שלום")
					.text_align(align)
					.with_style(|style| style.direction = direction),
			);
			let field = surface
				.backend()
				.view(surface.root().unwrap())
				.unwrap()
				.downcast_ref::<NSTextField>()
				.unwrap();
			assert_eq!(field.alignment(), expected);
			assert_eq!(
				field.baseWritingDirection(),
				match direction {
					Direction::Ltr => NSWritingDirection::LeftToRight,
					Direction::Rtl => NSWritingDirection::RightToLeft,
				}
			);
		}
	}

	fn mtm() -> MainThreadMarker {
		MainThreadMarker::new().expect("the harness runs tests on the main thread")
	}

	fn widget_tester_renders_appkit() {
		use leptoskit_testing::Finder;
		use leptoskit_testing::WidgetTester;

		let count = Arc::new(AtomicUsize::new(0));
		let mut tester = WidgetTester::with_backend(
			AppKitBackend::new(mtm()),
			move || {
				let action = Arc::clone(&count);
				Element::view()
					.child(Element::text(format!(
						"Count: {}",
						count.load(Ordering::SeqCst)
					)))
					.child(Element::button("Add").on_press(move || {
						action.fetch_add(1, Ordering::SeqCst);
					}))
			},
			390.0,
			844.0,
		)
		.expect("mount the real backend");
		tester.tap(&Finder::text("Add")).expect("tap and commit");
		tester
			.find(&Finder::text("Count: 1"))
			.expect("updated label");
		let surface = tester.surface();
		let root = surface.root().expect("mounted root");
		let label = surface.children(root)[0];
		let field = surface
			.backend()
			.view(label)
			.unwrap()
			.downcast_ref::<NSTextField>()
			.unwrap();
		assert_eq!(field.stringValue().to_string(), "Count: 1");
		let png = surface
			.backend()
			.snapshot_png(root, 1.0)
			.expect("render native views");
		assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
		if let Some(directory) = std::env::var_os("LEPTOSKIT_TEST_ARTIFACTS") {
			std::fs::create_dir_all(&directory).unwrap();
			std::fs::write(
				std::path::PathBuf::from(directory).join("widget-counter.png"),
				png,
			)
			.unwrap();
		}
	}

	fn surface(width: f32, height: f32, root: &Element) -> NativeSurface<AppKitBackend> {
		let mut surface = NativeSurface::new(AppKitBackend::new(mtm()));
		surface.set_viewport(width, height);
		surface.set_root(root);
		surface
	}

	fn padded(value: f32) -> leptoskit_yoga::Rect<Dimension> {
		leptoskit_yoga::Rect::all_edges(
			Dimension::Points(value),
			Dimension::Points(value),
			Dimension::Points(value),
			Dimension::Points(value),
		)
	}

	fn views_take_their_layout_frames() {
		let tree = Element::view()
			.with_style(|style| {
				style.width = Dimension::Percent(1.0);
				style.height = Dimension::Percent(1.0);
				style.padding = padded(20.0);
			})
			.child(Element::view().with_style(|style| style.height = Dimension::Points(40.0)))
			.child(
				Element::button("Go").with_style(|style| style.height = Dimension::Points(30.0)),
			);
		let surface = surface(200.0, 100.0, &tree);
		let root = surface.root().expect("root");
		let mut nodes = vec![root];
		nodes.extend_from_slice(surface.children(root));
		for id in nodes {
			let frame = surface.frame(id).expect("frame");
			let view = surface.backend().view(id).expect("view").frame();
			assert_eq!(
				(
					view.origin.x,
					view.origin.y,
					view.size.width,
					view.size.height
				),
				(
					f64::from(frame.x),
					f64::from(frame.y),
					f64::from(frame.width),
					f64::from(frame.height)
				),
				"{id:?}"
			);
		}
	}

	fn insert_child_respects_the_index() {
		let mut ids = Yoga::new();
		let mut backend = AppKitBackend::new(mtm());
		let [parent, first, second, third] = [(); 4].map(|()| ids.create_node());
		for id in [parent, first, second, third] {
			backend.create(id, NativeKind::View, &NativeProps::default());
		}
		backend.insert_child(parent, first, 0);
		backend.insert_child(parent, third, 1);
		backend.insert_child(parent, second, 1);

		let subviews = backend.view(parent).expect("parent").subviews();
		let order: Vec<_> = subviews.iter().collect();
		let expected: Vec<_> = [first, second, third]
			.iter()
			.map(|&id| backend.view(id).expect("child"))
			.collect();
		assert_eq!(order.len(), 3);
		for (actual, expected) in order.iter().zip(expected) {
			assert!(**actual == *expected, "subviews are out of order");
		}
	}

	fn removed_children_leave_their_parent() {
		let tree =
			|count: usize| Element::view().children((0..count).map(|_| Element::text("row")));
		let mut surface = surface(100.0, 100.0, &tree(3));
		let root = surface.root().expect("root");
		let removed = surface.children(root)[2];
		surface.set_root(&tree(1));

		let subviews = surface.backend().view(root).expect("root").subviews();
		assert_eq!(subviews.len(), 1);
		assert!(
			surface.backend().view(removed).is_none(),
			"deleted views are released"
		);
	}

	fn a_press_reaches_the_listener() {
		let handled = Arc::new(AtomicUsize::new(0));
		let tree = Element::view().child(Element::button("Go").on_press({
			let handled = Arc::clone(&handled);
			move || {
				handled.fetch_add(1, Ordering::SeqCst);
			}
		}));
		let surface = Rc::new(RefCell::new(surface(100.0, 100.0, &tree)));
		let button = {
			let surface = surface.borrow();
			surface.children(surface.root().expect("root"))[0]
		};
		let delivered = Rc::new(RefCell::new(Vec::new()));
		surface.borrow().backend().set_event_listener({
			let surface = Rc::downgrade(&surface);
			let delivered = Rc::clone(&delivered);
			move |node, event| {
				delivered.borrow_mut().push((node, event.clone()));
				if let Some(surface) = surface.upgrade() {
					surface.borrow().dispatch_event(node, event);
				}
			}
		});

		assert!(surface.borrow().backend().perform_press(button));

		assert_eq!(*delivered.borrow(), vec![(button, NativeEvent::Press)]);
		assert_eq!(handled.load(Ordering::SeqCst), 1);
		let root = surface.borrow().root().expect("root");
		assert!(
			!surface.borrow().backend().perform_press(root),
			"a view is not a button"
		);
	}

	fn hidden_views_toggle_without_hiding_siblings() {
		let mut ids = Yoga::new();
		let [root, parent, child, sibling] = [(); 4].map(|()| ids.create_node());
		let mut backend = AppKitBackend::new(mtm());
		for id in [root, sibling] {
			backend.create(id, NativeKind::View, &NativeProps::default());
		}
		backend.create(
			parent,
			NativeKind::View,
			&NativeProps {
				hidden: Some(true),
				..NativeProps::default()
			},
		);
		backend.create(child, NativeKind::Button, &NativeProps::text("Go"));
		backend.insert_child(root, parent, 0);
		backend.insert_child(parent, child, 0);
		backend.insert_child(root, sibling, 1);
		assert!(backend.view(parent).expect("parent").isHidden());
		assert!(
			backend
				.view(child)
				.expect("child")
				.isHiddenOrHasHiddenAncestor()
		);

		for hidden in [Some(true), Some(false), Some(true), None] {
			backend.update_props(
				parent,
				&NativeProps {
					hidden,
					..NativeProps::default()
				},
			);

			let expected = hidden.unwrap_or(false);
			assert_eq!(backend.view(parent).expect("parent").isHidden(), expected);
			let child_view = backend.view(child).expect("child");
			assert!(!child_view.isHidden(), "a child keeps its own visibility");
			assert_eq!(child_view.isHiddenOrHasHiddenAncestor(), expected);
			assert!(
				!backend
					.view(sibling)
					.expect("sibling")
					.isHiddenOrHasHiddenAncestor(),
				"a sibling is outside the hidden subtree",
			);
		}
	}

	fn clipping_updates_preserve_rounded_corners() {
		let mut ids = Yoga::new();
		let node = ids.create_node();
		let mut backend = AppKitBackend::new(mtm());
		backend.create(
			node,
			NativeKind::View,
			&NativeProps {
				clip: Some(true),
				..NativeProps::default()
			},
		);
		assert!(
			backend
				.view(node)
				.expect("view")
				.layer()
				.expect("layer")
				.masksToBounds()
		);

		for (clip, corner_radius, expected) in [
			(Some(false), Some(8.0), true),
			(None, Some(8.0), true),
			(Some(true), None, true),
			(Some(false), None, false),
			(None, None, false),
		] {
			backend.update_props(
				node,
				&NativeProps {
					clip,
					corner_radius,
					..NativeProps::default()
				},
			);

			let layer = backend.view(node).expect("view").layer().expect("layer");
			assert_eq!(
				layer.masksToBounds(),
				expected,
				"clip={clip:?}, radius={corner_radius:?}"
			);
			assert_eq!(
				layer.cornerRadius(),
				f64::from(corner_radius.unwrap_or(0.0))
			);
		}
	}

	fn scroll_views_cover_overflow_on_both_axes() {
		let tree = Element::scroll_view()
			.with_style(|style| {
				style.width = Dimension::Points(100.0);
				style.height = Dimension::Points(80.0);
			})
			.child(Element::view().with_style(|style| {
				style.width = Dimension::Points(280.0);
				style.height = Dimension::Points(220.0);
			}));
		let surface = surface(100.0, 80.0, &tree);
		let root = surface.root().expect("root");
		let scroll = surface
			.backend()
			.view(root)
			.expect("scroll view")
			.downcast_ref::<NSScrollView>()
			.expect("a ScrollView uses an NSScrollView");

		assert!(scroll.hasVerticalScroller());
		assert!(scroll.hasHorizontalScroller());
		assert!(scroll.autohidesScrollers());
		let document = scroll.documentView().expect("scroll content");
		assert_eq!(document.frame().size, NSSize::new(280.0, 220.0));
		let visible = scroll.contentSize();
		assert!(document.frame().size.width > visible.width);
		assert!(document.frame().size.height > visible.height);
	}

	/// A label laid out at its measured size draws every line: the cell needs
	/// no more height than the frame at the frame's width.
	fn labels_fit_their_measured_text() {
		let paragraph = "Designing calm software for loud cities, one specimen at a time.";
		let cases = [
			("Home", 12.0, FontWeight::Regular, TextAlign::Center),
			("Followers", 12.0, FontWeight::Regular, TextAlign::Start),
			(paragraph, 15.0, FontWeight::Regular, TextAlign::Start),
			(paragraph, 16.0, FontWeight::Medium, TextAlign::Center),
			(paragraph, 17.0, FontWeight::Semibold, TextAlign::End),
			("$92,418", 28.0, FontWeight::Bold, TextAlign::Start),
		];
		let tree = Element::view()
			.with_style(|style| style.align_items = Align::FlexStart)
			.children(cases.iter().map(|&(text, size, weight, align)| {
				Element::text(text)
					.font(size, weight)
					.text_align(align)
					.with_style(|style| style.max_width = Dimension::Points(180.0))
			}));
		let surface = surface(400.0, 800.0, &tree);
		let root = surface.root().expect("root");
		for (&id, (text, size, ..)) in surface.children(root).iter().zip(cases) {
			let view = surface.backend().view(id).expect("label");
			let label = view
				.downcast_ref::<NSTextField>()
				.expect("text is an NSTextField");
			let frame = label.frame();
			let cell = label.cell().expect("labels have a cell");
			let needed = cell.cellSizeForBounds(NSRect::new(
				NSPoint::ZERO,
				NSSize::new(frame.size.width, 10_000.0),
			));
			assert!(
				needed.height <= frame.size.height && needed.width <= frame.size.width,
				"{text:?} at {size}pt needs {needed:?} but has {:?}",
				frame.size
			);
		}
	}

	fn measure(text: &str, style: TextStyle, width: Option<f32>) -> Size<f32> {
		let available = Size::new(
			width.map_or(AvailableSpace::MaxContent, AvailableSpace::Definite),
			AvailableSpace::MaxContent,
		);
		AppKitTextMeasurer::new().measure(text, style, available)
	}

	fn measurement_follows_the_style() {
		let text = "the quick brown fox jumps over the lazy dog";
		let body = TextStyle::new(15.0, FontWeight::Regular);
		let line = measure(text, body, None);
		let wrapped = measure(text, body, Some(line.width / 2.0));
		assert!(wrapped.height >= 2.0 * line.height, "narrow text wraps");
		assert!(
			wrapped.width <= line.width / 2.0,
			"wrapped text fits its width"
		);

		let large = measure(text, TextStyle::new(30.0, FontWeight::Regular), None);
		assert!(large.width > 1.8 * line.width && large.height > 1.8 * line.height);
		let bold = measure(text, TextStyle::new(15.0, FontWeight::Bold), None);
		assert!(bold.width > line.width, "bold is wider");

		let centered = measure(
			text,
			TextStyle {
				text_align: TextAlign::Center,
				..body
			},
			None,
		);
		assert_eq!(
			centered.width,
			line.width + 4.0,
			"centered labels reserve more"
		);
		let min = AppKitTextMeasurer::new().measure(
			text,
			body,
			Size::new(AvailableSpace::MinContent, AvailableSpace::MaxContent),
		);
		assert!(
			min.width < line.width / 4.0,
			"min-content is the widest word"
		);
	}

	fn measurement_works_off_the_main_thread() {
		let style = TextStyle::new(15.0, FontWeight::Regular);
		let here = measure("measured anywhere", style, None);
		let there = std::thread::spawn(move || measure("measured anywhere", style, None))
			.join()
			.expect("the measuring thread finishes");
		assert_eq!(here, there);
	}

	fn snapshots_are_pngs_at_the_scale() {
		let tree = Element::view()
			.with_style(|style| {
				style.width = Dimension::Points(120.0);
				style.height = Dimension::Points(80.0);
			})
			.background(Color::hex("#e5304f").expect("color"))
			.corner_radius(12.0)
			.child(Element::text("snap"));
		let surface = surface(300.0, 300.0, &tree);
		let root = surface.root().expect("root");
		let png = surface
			.backend()
			.snapshot_png(root, 2.0)
			.expect("a snapshot");

		assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
		let width = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
		let height = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
		assert_eq!((width, height), (240, 160));
	}

	fn snapshot_errors_are_reported() {
		let tree = Element::view().child(Element::view());
		let surface = surface(100.0, 100.0, &tree);
		let root = surface.root().expect("root");
		let empty = surface.children(root)[0];

		assert_eq!(
			surface.backend().snapshot_png(root, 0.0),
			Err(SnapshotError::InvalidScale(0.0))
		);
		assert!(matches!(
			surface.backend().snapshot_png(empty, 2.0),
			Err(SnapshotError::EmptyView { .. })
		));
		let mut ids = Yoga::new();
		let unknown: NodeId = (0..10).map(|_| ids.create_node()).last().expect("an id");
		assert_eq!(
			surface.backend().snapshot_png(unknown, 2.0),
			Err(SnapshotError::UnknownNode(unknown))
		);
	}
}
