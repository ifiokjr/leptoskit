#![cfg(all(feature = "rive", not(target_arch = "wasm32")))]

use std::time::Duration;

use leptoskit_native::Element;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::Mutation;
use leptoskit_native::NativeSurface;
use leptoskit_native::RiveEnvironment;
use leptoskit_native::RiveError;
use leptoskit_native::RiveFrame;
use leptoskit_native::RivePresenter;
use leptoskit_rive::Asset;
use leptoskit_rive::Playback;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::NodeId;

fn content() -> Playback {
	Playback::new(
		Asset::new(include_bytes!(
			"../../leptoskit_rive/tests/fixtures/scene/build/leptoskit-rive-test.riv"
		))
		.unwrap(),
	)
}

fn widget(running: bool) -> Element {
	Element::rive(content())
		.aria_label("Rotating triangle")
		.running(running)
		.with_style(|style| {
			style.width = Dimension::Points(128.0);
			style.height = Dimension::Points(128.0);
		})
}

#[derive(Default)]
struct Presenter {
	hidden: bool,
	uploads: Vec<(NodeId, Vec<u8>)>,
	fail: bool,
}

impl RivePresenter for Presenter {
	fn visible(&mut self, _id: NodeId) -> bool {
		!self.hidden
	}

	fn present(&mut self, id: NodeId, frame: &RiveFrame<'_>) -> Result<(), RiveError> {
		if self.fail {
			return Err(RiveError::Runtime("upload failed".into()));
		}
		self.uploads.push((id, frame.bytes().to_vec()));
		Ok(())
	}
}

#[test]
fn reconciliation_retains_the_pose_and_never_sends_assets_to_widgets() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(128.0, 128.0);
	surface.set_root(&widget(true));
	let mut presenter = Presenter::default();
	let environment = RiveEnvironment::default();
	assert!(
		surface
			.tick_rive_with(Duration::ZERO, environment, &mut presenter)
			.unwrap()
	);
	let initial = presenter.uploads[0].1.clone();
	surface
		.tick_rive_with(Duration::from_millis(100), environment, &mut presenter)
		.unwrap();
	assert_ne!(initial, presenter.uploads[1].1);
	surface.set_root(&widget(false).aria_label("Paused triangle"));
	assert!(
		!surface
			.tick_rive_with(Duration::ZERO, environment, &mut presenter)
			.unwrap()
	);
	assert_eq!(
		presenter.uploads.len(),
		2,
		"a label change must not reload the scene or rewind it"
	);
	for mutation in surface.backend().log() {
		if let Mutation::Create { props, .. } | Mutation::UpdateProps { props, .. } = mutation {
			assert!(
				props.rive.is_none(),
				"native UI transactions must not contain encoded assets"
			);
		}
	}
	surface.set_root(&Element::view());
	assert!(!surface.has_rive_players());
	assert!(
		!surface
			.tick_rive_with(Duration::ZERO, environment, &mut presenter)
			.unwrap()
	);
}

#[test]
fn visibility_activity_and_motion_leave_no_pending_frames() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(128.0, 128.0);
	surface.set_root(&widget(true));
	let mut presenter = Presenter {
		hidden: true,
		..Presenter::default()
	};
	let environment = RiveEnvironment::default();
	assert!(
		!surface
			.tick_rive_with(Duration::ZERO, environment, &mut presenter)
			.unwrap()
	);
	assert!(presenter.uploads.is_empty());
	presenter.hidden = false;
	assert!(
		!surface
			.tick_rive_with(
				Duration::ZERO,
				RiveEnvironment {
					active: false,
					..environment
				},
				&mut presenter
			)
			.unwrap()
	);
	assert!(presenter.uploads.is_empty());
	assert!(
		!surface
			.tick_rive_with(
				Duration::ZERO,
				RiveEnvironment {
					reduced_motion: true,
					..environment
				},
				&mut presenter
			)
			.unwrap()
	);
	assert_eq!(
		presenter.uploads.len(),
		1,
		"reduced motion still displays the initial pose"
	);
	assert!(
		surface
			.tick_rive_with(Duration::ZERO, environment, &mut presenter)
			.unwrap()
	);
}

#[test]
fn parent_visibility_and_paused_resizing_are_observed_on_activation() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(128.0, 128.0);
	let mut parent = Element::view().child(widget(false));
	parent.props.hidden = Some(true);
	surface.set_root(&parent);
	let mut presenter = Presenter::default();
	let environment = RiveEnvironment::default();
	assert!(
		!surface
			.tick_rive_with(Duration::ZERO, environment, &mut presenter)
			.unwrap()
	);
	assert!(presenter.uploads.is_empty());
	parent.props.hidden = Some(false);
	surface.set_root(&parent);
	surface
		.tick_rive_with(Duration::ZERO, environment, &mut presenter)
		.unwrap();
	assert_eq!(presenter.uploads[0].1.len(), 128 * 128 * 4);
	surface
		.tick_rive_with(
			Duration::ZERO,
			RiveEnvironment {
				scale: 2.0,
				..environment
			},
			&mut presenter,
		)
		.unwrap();
	assert_eq!(presenter.uploads[1].1.len(), 256 * 256 * 4);
	assert!(!surface.rive_needs_frame());
}

#[test]
fn presentation_failure_stops_retries_until_content_is_replaced() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(128.0, 128.0);
	surface.set_root(&widget(true));
	let mut presenter = Presenter {
		fail: true,
		..Presenter::default()
	};
	assert!(
		surface
			.tick_rive_with(Duration::ZERO, RiveEnvironment::default(), &mut presenter)
			.is_err()
	);
	assert!(!surface.rive_needs_frame());
	assert!(
		!surface
			.tick_rive_with(Duration::ZERO, RiveEnvironment::default(), &mut presenter)
			.unwrap()
	);
	assert!(presenter.uploads.is_empty());
}

#[test]
fn surfaces_bound_player_count_and_total_backing_pixels() {
	#[derive(Default)]
	struct Budget {
		pixels: u64,
		uploads: usize,
	}
	impl RivePresenter for Budget {
		fn visible(&mut self, _: NodeId) -> bool {
			true
		}

		fn present(&mut self, _: NodeId, frame: &RiveFrame<'_>) -> Result<(), RiveError> {
			assert!(frame.width() <= 4096 && frame.height() <= 4096);
			self.pixels += u64::from(frame.width()) * u64::from(frame.height());
			self.uploads += 1;
			Ok(())
		}
	}
	let content = content();
	let tree = |count| {
		Element::view().children((0..count).map(|_| {
			Element::rive(content.clone())
				.aria_hidden(true)
				.with_style(|style| {
					style.position = leptoskit_yoga::PositionType::Absolute;
					style.width = Dimension::Points(5000.0);
					style.height = Dimension::Points(5000.0);
				})
		}))
	};
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(5000.0, 5000.0);
	surface.set_root(&tree(16));
	let mut budget = Budget::default();
	let environment = RiveEnvironment {
		scale: 10.0,
		..RiveEnvironment::default()
	};
	assert!(
		!surface
			.tick_rive_with(Duration::ZERO, environment, &mut budget)
			.unwrap()
	);
	assert_eq!(budget.uploads, 16);
	assert!(budget.pixels <= 8_388_608);
	assert!(
		!surface
			.tick_rive_with(Duration::ZERO, environment, &mut budget)
			.unwrap()
	);
	assert_eq!(
		budget.uploads, 16,
		"paused scenes do no further raster or upload work"
	);
	surface.set_root(&tree(17));
	assert!(
		surface
			.tick_rive_with(Duration::ZERO, environment, &mut budget)
			.unwrap_err()
			.to_string()
			.contains("16 Rive players")
	);
}

#[test]
fn invalid_rive_trees_fail_before_backend_mutation() {
	assert!(
		Element::view()
			.rive_playback(content())
			.aria_label("Illustration")
			.validate()
			.is_err()
	);
	assert!(Element::rive(content()).validate().is_err());
	assert!(
		widget(false)
			.child(Element::text("child"))
			.validate()
			.is_err()
	);
	assert!(
		widget(false)
			.source("https://example.invalid/file.riv")
			.validate()
			.is_err()
	);
	assert!(
		Element::rive(content())
			.aria_hidden(true)
			.validate()
			.is_ok()
	);
}

#[test]
fn removing_a_rive_widget_releases_its_encoded_asset_even_though_node_ids_stay_allocated() {
	let bytes: std::sync::Arc<[u8]> =
		include_bytes!("../../leptoskit_rive/tests/fixtures/scene/build/leptoskit-rive-test.riv")
			.as_slice()
			.into();
	let weak = std::sync::Arc::downgrade(&bytes);
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(128.0, 128.0);
	surface.set_root(
		&Element::rive(Playback::new(Asset::from_shared(bytes).unwrap())).aria_hidden(true),
	);
	assert!(weak.upgrade().is_some());
	surface.set_root(&Element::view());
	assert!(
		weak.upgrade().is_none(),
		"deleted shadow slots, backend logs, and player caches must release the asset"
	);
}
