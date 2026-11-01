use leptoskit_use::*;
use reactive_graph::owner::Owner;
use reactive_graph::prelude::*;
use reactive_graph::signal::RwSignal;

#[test]
fn toggle_allocates_or_reuses_state() {
	Owner::new().with(|| {
		let toggle = use_toggle(false);
		assert!(!toggle.value.get());
		(toggle.toggle)();
		assert!(toggle.value.get());
		toggle.set_value.set(false);
		assert!(!toggle.value.get());
		let shared = RwSignal::new(true);
		let toggle = use_toggle(shared);
		(toggle.toggle)();
		assert!(!shared.get());
		shared.set(true);
		assert!(toggle.value.get());
	});
}

#[test]
fn cycle_handles_wraparound_empty_and_changing_lists() {
	Owner::new().with(|| {
		let list = RwSignal::new(vec!["a", "b", "c"]);
		let cycle = use_cycle_list(list);
		assert_eq!(cycle.state.get(), Some("a"));
		assert_eq!(cycle.index.get(), Some(0));
		(cycle.prev)();
		assert_eq!(cycle.state.get(), Some("c"));
		(cycle.prev)();
		assert_eq!(cycle.state.get(), Some("b"));
		(cycle.next)();
		assert_eq!(cycle.state.get(), Some("c"));
		(cycle.next)();
		assert_eq!(cycle.state.get(), Some("a"));
		(cycle.set_index)(usize::MAX);
		assert_eq!(cycle.index.get(), Some(usize::MAX % 3));
		(cycle.set_index)(2);
		list.set(vec!["x", "y"]);
		assert_eq!(cycle.state.get(), Some("x"));
		list.set(vec![]);
		assert_eq!(cycle.state.get(), None);
		assert_eq!(cycle.index.get(), None);
		(cycle.next)();
		(cycle.prev)();
		assert_eq!(cycle.state.get(), None);
		list.set(vec!["only"]);
		(cycle.next)();
		(cycle.prev)();
		assert_eq!(cycle.state.get(), Some("only"));
	});
}

#[test]
fn derived_predicates_and_logic_follow_sources() {
	Owner::new().with(|| {
		let a = RwSignal::new(false);
		let b = RwSignal::new(false);
		let (not, and, or) = (use_not(a), use_and(a, b), use_or(a, b));
		for (left, right) in [(false, false), (false, true), (true, false), (true, true)] {
			a.set(left);
			b.set(right);
			assert_eq!(not.get(), !left);
			assert_eq!(and.get(), left && right);
			assert_eq!(or.get(), left || right);
		}

		let option = RwSignal::new(None::<u32>);
		let (some, none) = (is_some(option), is_none(option));
		assert!(!some.get());
		assert!(none.get());
		option.set(Some(4));
		assert!(some.get());
		assert!(!none.get());
		let result = RwSignal::new(Ok::<_, &str>(5));
		let (ok, err) = (is_ok(result), is_err(result));
		assert!(ok.get());
		assert!(!err.get());
		result.set(Err("failure"));
		assert!(!ok.get());
		assert!(err.get());
		let value = RwSignal::new(42);
		let text = use_to_string(value);
		assert_eq!(text.get(), "42");
		value.set(-3);
		assert_eq!(text.get(), "-3");
	});
}

#[test]
fn sorted_copies_are_stable_and_leave_sources_untouched() {
	Owner::new().with(|| {
		let values = RwSignal::new(vec![3, 1, 2, 1]);
		let sorted = use_sorted(values);
		let reversed = use_sorted_by(values, |a, b| b.cmp(a));
		assert_eq!(sorted.get(), vec![1, 1, 2, 3]);
		assert_eq!(reversed.get(), vec![3, 2, 1, 1]);
		assert_eq!(values.get(), vec![3, 1, 2, 1]);
		values.set(vec![]);
		assert_eq!(sorted.get(), [] as [i32; 0]);
		let records = RwSignal::new(vec![(2, "b"), (1, "a"), (2, "c")]);
		let keyed = use_sorted_by_key(records, |item| item.0);
		assert_eq!(keyed.get(), vec![(1, "a"), (2, "b"), (2, "c")]);
		records.set(vec![(0, "x")]);
		assert_eq!(keyed.get(), vec![(0, "x")]);
	});
}

#[test]
fn numeric_hooks_handle_updates_and_empty_extrema() {
	Owner::new().with(|| {
		let number = RwSignal::new(-1.5_f64);
		let (abs, ceil, floor, round) = (
			use_abs(number),
			use_ceil(number),
			use_floor(number),
			use_round(number),
		);
		assert_eq!(
			[abs.get(), ceil.get(), floor.get(), round.get()].map(f64::to_bits),
			[1.5, -1.0, -2.0, -2.0].map(f64::to_bits)
		);
		number.set(2.4);
		assert_eq!(
			[abs.get(), ceil.get(), floor.get(), round.get()].map(f64::to_bits),
			[2.4, 3.0, 2.0, 2.0].map(f64::to_bits)
		);
		number.set(f64::NAN);
		assert!(
			[abs.get(), ceil.get(), floor.get(), round.get()]
				.into_iter()
				.all(f64::is_nan)
		);
		let values = RwSignal::new(vec![3, -1, 5]);
		let (min, max) = (use_min(values), use_max(values));
		assert_eq!(min.get(), Some(-1));
		assert_eq!(max.get(), Some(5));
		values.set(vec![]);
		assert_eq!(min.get(), None);
		assert_eq!(max.get(), None);
	});
}

#[test]
fn viewport_queries_track_exact_boundaries_and_unknown_keys() {
	Owner::new().with(|| {
		let viewport = RwSignal::new(Viewport {
			width: 640,
			height: 480,
		});
		let size = use_window_size(viewport);
		let breaks = use_breakpoints([("small", 640), ("large", 1024)].into(), size.width);
		let large = breaks.greater_or_equal(&"large").unwrap();
		let small = breaks.smaller(&"small").unwrap();
		let middle = breaks.between(&"small", &"large").unwrap();
		assert!(!large.get());
		assert!(!small.get());
		assert!(middle.get());
		assert_eq!(size.height.get(), 480);
		viewport.set(Viewport {
			width: 1024,
			height: 768,
		});
		assert!(large.get());
		assert!(!middle.get());
		assert_eq!(size.width.get(), 1024);
		assert_eq!(size.height.get(), 768);
		viewport.set(Viewport::default());
		assert!(small.get());
		assert!(!large.get());
		assert!(!middle.get());
		assert!(!breaks.between(&"large", &"small").unwrap().get());
		assert!(breaks.greater_or_equal(&"missing").is_none());
		assert!(breaks.smaller(&"missing").is_none());
		assert!(breaks.between(&"missing", &"large").is_none());
		assert!(breaks.between(&"small", &"missing").is_none());
	});
}

#[test]
fn fractional_native_viewports_do_not_round_across_tailwind_breakpoints() {
	Owner::new().with(|| {
		let viewport = RwSignal::new(Viewport {
			width: 767.75_f32,
			height: 600.25,
		});
		let size = use_window_size(viewport);
		let points = use_breakpoints([("md", 768.0), ("lg", 1024.0)].into(), size.width);
		let md = points.greater_or_equal(&"md").unwrap();
		let below = points.smaller(&"md").unwrap();
		let range = points.between(&"md", &"lg").unwrap();
		assert!(below.get());
		assert!(!md.get());
		assert_eq!(size.height.get().to_bits(), 600.25_f32.to_bits());
		viewport.set(Viewport {
			width: 768.25,
			height: 600.5,
		});
		assert!(md.get());
		assert!(!below.get());
		assert!(range.get());
		viewport.set(Viewport {
			width: f32::NAN,
			height: 600.5,
		});
		assert!(!md.get());
		assert!(!below.get());
		assert!(!range.get());
	});
}

#[test]
fn posture_queries_follow_fold_transitions() {
	use std::sync::Arc;
	use std::sync::atomic::AtomicUsize;
	use std::sync::atomic::Ordering;

	use reactive_graph::computed::Memo;

	fn book(fold: Fold, angle: u16) -> Posture {
		Posture {
			foldable: Some(true),
			fold,
			hinges: vec![Hinge {
				fold,
				axis: Axis::Vertical,
				region: Some(Region {
					x: 0.5,
					y: 0.0,
					width: 0.0,
					height: 1.0,
				}),
				angle: Some(angle),
				occluding: Some(false),
			}],
		}
	}

	Owner::new().with(|| {
		let posture = RwSignal::new(Posture {
			foldable: Some(true),
			fold: Fold::Closed,
			hinges: Vec::new(),
		});
		let fold = use_posture(posture);
		assert!(fold.is_closed.get());
		assert_eq!(fold.panes.get(), vec![Region::FULL]);
		assert_eq!(fold.angle.get(), None);

		let runs = Arc::new(AtomicUsize::new(0));
		let half_open_reads = {
			let runs = Arc::clone(&runs);
			Memo::new(move |_| {
				runs.fetch_add(1, Ordering::SeqCst);
				fold.is_half_open.get()
			})
		};
		assert!(!half_open_reads.get());

		posture.set(book(Fold::HalfOpen, 90));
		assert!(half_open_reads.get());
		assert!(fold.is_book.get());
		assert!(!fold.is_tabletop.get());
		assert_eq!(fold.angle.get(), Some(90));
		assert_eq!(fold.panes.get().len(), 2);
		let after_opening = runs.load(Ordering::SeqCst);

		// Moving the hinge without leaving half open does not notify readers.
		posture.set(book(Fold::HalfOpen, 120));
		assert!(half_open_reads.get());
		assert_eq!(fold.angle.get(), Some(120));
		assert_eq!(runs.load(Ordering::SeqCst), after_opening);

		posture.set(book(Fold::Open, 180));
		assert!(fold.is_open.get());
		assert!(!fold.is_book.get());
		assert_eq!(fold.fold.get(), Fold::Open);

		posture.set(Posture::rigid());
		assert_eq!(fold.fold.get(), Fold::Unknown);
		assert!(!fold.is_open.get() && !fold.is_closed.get() && !fold.is_half_open.get());
	});
}

#[test]
fn supported_mirrors_capability_probes() {
	Owner::new().with(|| {
		let clipboard = RwSignal::new(true);
		let supported = use_supported(move || clipboard.get());
		assert!(supported.get());
		clipboard.set(false);
		assert!(!supported.get());
	});
}
