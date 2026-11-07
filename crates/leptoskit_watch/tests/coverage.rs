//! Branch coverage for the shared watch API surface.

use leptoskit_native::Element;
use leptoskit_native::components::Button;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_watch::ComplicationFamily;
use leptoskit_watch::RecordingHost;
use leptoskit_watch::RefreshPolicy;
use leptoskit_watch::WatchAmbient;
use leptoskit_watch::WatchApp;
use leptoskit_watch::WatchBudget;
use leptoskit_watch::WatchComplication;
use leptoskit_watch::WatchError;
use leptoskit_watch::WatchGlance;
use leptoskit_watch::WatchHost;
use leptoskit_watch::WatchKind;
use leptoskit_watch::WatchNavigation;
use leptoskit_watch::WatchPage;
use leptoskit_watch::WatchPresentation;
use leptoskit_watch::WatchShape;
use leptoskit_watch::WatchSizeClass;
use leptoskit_watch::WatchTile;
use leptoskit_watch::WatchViewport;
use leptoskit_watch::measure;

fn tiny() -> Element {
	Text("ok")
}

#[test]
fn kind_aliases_display_and_unknown() {
	assert_eq!("wear-os".parse::<WatchKind>().unwrap(), WatchKind::WearOs);
	assert_eq!("wear_os".parse::<WatchKind>().unwrap(), WatchKind::WearOs);
	assert_eq!("watch-os".parse::<WatchKind>().unwrap(), WatchKind::WatchOs);
	assert_eq!("watch_os".parse::<WatchKind>().unwrap(), WatchKind::WatchOs);
	assert_eq!(
		"nope".parse::<WatchKind>().unwrap_err(),
		WatchError::UnknownKind("nope".into())
	);
	assert_eq!(WatchKind::WearOs.to_string(), "wearos");
	assert_eq!(WatchKind::WatchOs.to_string(), "watchos");
	assert_eq!(WatchKind::WearOs.display_name(), "Wear OS");
	assert_eq!(WatchKind::WatchOs.display_name(), "watchOS");
}

#[test]
fn viewport_content_box_and_constructors() {
	let wear = WatchViewport::wear_os_medium();
	assert_eq!(wear.kind, WatchKind::WearOs);
	assert_eq!(wear.shape, WatchShape::Round);
	assert_eq!(wear.size_class, WatchSizeClass::Medium);
	assert_eq!(wear.content_width(), 384.0 - 48.0);
	assert_eq!(wear.content_height(), 384.0 - 48.0);

	let watch = WatchViewport::watch_os_medium();
	assert_eq!(watch.kind, WatchKind::WatchOs);
	assert_eq!(watch.shape, WatchShape::RoundedRect);
	assert!(watch.content_width() > 0.0);

	let zeroed = WatchViewport {
		kind: WatchKind::WearOs,
		shape: WatchShape::Square,
		size_class: WatchSizeClass::Small,
		width_dp: 10.0,
		height_dp: 10.0,
		safe_inset_dp: 20.0,
	};
	assert_eq!(zeroed.content_width(), 0.0);
	assert_eq!(zeroed.content_height(), 0.0);
	assert!(WatchSizeClass::Small < WatchSizeClass::Large);
}

#[test]
fn budgets_and_measures() {
	assert_eq!(WatchBudget::default(), WatchBudget::page());
	for budget in [
		WatchBudget::page(),
		WatchBudget::tile(),
		WatchBudget::glance(),
		WatchBudget::complication(),
	] {
		assert!(measure(&tiny(), budget).is_ok());
	}

	let many = View().children((0..40).map(|i| Text(format!("n{i}"))));
	assert!(matches!(
		measure(&many, WatchBudget::page()),
		Err(WatchError::TooManyNodes { .. })
	));

	let long = Text("x".repeat(200));
	assert!(matches!(
		measure(&long, WatchBudget::page()),
		Err(WatchError::TooMuchText { .. })
	));
}

#[test]
fn refused_widget_kinds() {
	use leptoskit_native::NativeKind;
	use leptoskit_native::components::Checkbox;
	use leptoskit_native::components::PasswordInput;
	use leptoskit_native::components::Slider;
	use leptoskit_native::controls::SliderState;

	let cases: &[(&str, Element)] = &[
		("ScrollView", Element::scroll_view()),
		("TextInput", Element::text_input()),
		("PasswordInput", PasswordInput()),
		("Checkbox", Checkbox(false)),
		("Radio", Element::new(NativeKind::Radio)),
		("Slider", Slider(SliderState::default())),
		("Rive", Element::new(NativeKind::Rive)),
		("custom", Element::new(NativeKind::Custom("custom"))),
	];
	for (name, element) in cases {
		assert_eq!(
			measure(element, WatchBudget::page()).unwrap_err(),
			WatchError::UnsupportedKind(name),
			"{name}"
		);
	}
	assert!(measure(&Element::image("local.png"), WatchBudget::page()).is_ok());
	assert!(measure(&Element::button("Go"), WatchBudget::page()).is_ok());
	assert!(
		measure(
			&leptoskit_native::components::Switch(true),
			WatchBudget::page()
		)
		.is_ok()
	);
	assert!(
		measure(
			&leptoskit_native::components::Pressable("Go"),
			WatchBudget::page()
		)
		.is_ok()
	);
	assert!(
		measure(
			&leptoskit_native::components::ActivityIndicator(true),
			WatchBudget::page()
		)
		.is_ok()
	);
	assert!(
		measure(
			&Element::new(NativeKind::LinearProgressIndicator),
			WatchBudget::page()
		)
		.is_ok()
	);
	assert!(measure(&Element::new(NativeKind::TextRun), WatchBudget::page()).is_ok());
	assert!(
		measure(
			&Element::vector(leptoskit_native::components::ViewBox::UNIT),
			WatchBudget::page()
		)
		.is_ok()
	);
}

#[test]
fn page_accessors_and_invalid_ids() {
	let page = WatchPage::new("home", tiny())
		.title("Home")
		.budget(WatchBudget::tile());
	assert_eq!(page.id(), "home");
	assert_eq!(page.page_title(), Some("Home"));
	assert_eq!(page.page_budget(), WatchBudget::tile());
	assert_eq!(page.content().props.text.as_deref(), Some("ok"));
	page.validate().unwrap();
	let _ = format!("{page:?}");

	assert_eq!(
		WatchPage::new("", tiny()).validate().unwrap_err(),
		WatchError::InvalidId(String::new())
	);
	assert_eq!(
		WatchPage::new("bad id", tiny()).validate().unwrap_err(),
		WatchError::InvalidId("bad id".into())
	);
}

#[test]
fn ambient_builders_accessors_and_validation() {
	let tile = WatchTile::new("t", tiny())
		.refresh(RefreshPolicy::Minute)
		.budget(WatchBudget::tile());
	assert_eq!(tile.id(), "t");
	assert_eq!(tile.refresh_policy(), RefreshPolicy::Minute);
	assert_eq!(tile.content().props.text.as_deref(), Some("ok"));
	tile.validate().unwrap();
	let _ = format!("{tile:?}");

	let glance =
		WatchGlance::new("g", tiny(), View().child(Text("body"))).budget(WatchBudget::glance());
	assert_eq!(glance.id(), "g");
	assert_eq!(glance.preview().props.text.as_deref(), Some("ok"));
	assert!(!glance.content().children.is_empty());
	glance.validate().unwrap();
	let _ = format!("{glance:?}");

	let complication = WatchComplication::new("c", ComplicationFamily::Corner, tiny())
		.budget(WatchBudget::complication());
	assert_eq!(complication.id(), "c");
	assert_eq!(complication.family(), ComplicationFamily::Corner);
	assert_eq!(complication.content().props.text.as_deref(), Some("ok"));
	complication.validate().unwrap();
	let _ = format!("{complication:?}");

	assert_eq!(WatchAmbient::Tile(tile.clone()).id(), "t");
	assert_eq!(WatchAmbient::Glance(glance.clone()).id(), "g");
	assert_eq!(WatchAmbient::Complication(complication.clone()).id(), "c");
	WatchAmbient::Tile(tile).validate().unwrap();
	WatchAmbient::Glance(glance).validate().unwrap();
	WatchAmbient::Complication(complication).validate().unwrap();

	assert_eq!(RefreshPolicy::default(), RefreshPolicy::Manual);
	for family in [
		ComplicationFamily::Circular,
		ComplicationFamily::CircularSmall,
		ComplicationFamily::Rectangular,
		ComplicationFamily::Inline,
		ComplicationFamily::Corner,
		ComplicationFamily::ExtraLarge,
	] {
		WatchComplication::new(format!("f-{family:?}"), family, tiny())
			.validate()
			.unwrap();
	}
}

#[test]
fn app_duplicates_navigation_and_accessors() {
	let app = WatchApp::new("steps", WatchKind::WearOs)
		.navigation(WatchNavigation::Pager)
		.page(WatchPage::new("home", tiny()))
		.ambient(WatchAmbient::Tile(WatchTile::new("tile", tiny())))
		.tile(WatchTile::new("tile-2", tiny()))
		.glance(WatchGlance::new("glance", tiny(), tiny()))
		.complication(WatchComplication::new(
			"face",
			ComplicationFamily::Inline,
			tiny(),
		));
	assert_eq!(app.id(), "steps");
	assert_eq!(app.kind(), WatchKind::WearOs);
	assert_eq!(app.navigation_model(), WatchNavigation::Pager);
	assert_eq!(app.pages().len(), 1);
	assert_eq!(app.ambients().len(), 4);
	app.validate().unwrap();
	let _ = format!("{app:?}");

	let dup_page = WatchApp::new("a", WatchKind::WearOs)
		.page(WatchPage::new("same", tiny()))
		.page(WatchPage::new("same", tiny()));
	assert_eq!(
		dup_page.validate().unwrap_err(),
		WatchError::DuplicateId("same".into())
	);

	let dup_ambient = WatchApp::new("a", WatchKind::WearOs)
		.page(WatchPage::new("home", tiny()))
		.tile(WatchTile::new("home", tiny()));
	assert_eq!(
		dup_ambient.validate().unwrap_err(),
		WatchError::DuplicateId("home".into())
	);

	for navigation in [
		WatchNavigation::Stack,
		WatchNavigation::Pager,
		WatchNavigation::ListDetail,
	] {
		assert_eq!(
			WatchApp::new("empty", WatchKind::WatchOs)
				.navigation(navigation)
				.validate()
				.unwrap_err(),
			WatchError::MissingPage(navigation)
		);
	}
	WatchApp::new("ok", WatchKind::WatchOs)
		.navigation(WatchNavigation::Single)
		.validate()
		.unwrap();
	assert_eq!(WatchNavigation::default(), WatchNavigation::Single);
}

#[test]
fn recording_host_clear_and_default_trait_path() {
	let mut host = RecordingHost::new(WatchViewport::wear_os_medium());
	assert_eq!(host.viewport().kind, WatchKind::WearOs);
	host.present_page(&WatchPage::new("p", tiny())).unwrap();
	host.present_ambient(&WatchAmbient::Tile(WatchTile::new("a", tiny())))
		.unwrap();
	assert_eq!(host.presentations().len(), 2);
	host.clear();
	assert_eq!(host.presentations(), []);

	struct Thin(RecordingHost);
	impl WatchHost for Thin {
		fn viewport(&self) -> WatchViewport {
			self.0.viewport()
		}

		fn present_page(&mut self, page: &WatchPage) -> Result<(), WatchError> {
			self.0.present_page(page)
		}

		fn present_ambient(&mut self, ambient: &WatchAmbient) -> Result<(), WatchError> {
			self.0.present_ambient(ambient)
		}
	}

	let app = WatchApp::new("demo", WatchKind::WearOs)
		.page(WatchPage::new("home", tiny()))
		.tile(WatchTile::new("tile", tiny()));
	let mut thin = Thin(RecordingHost::new(WatchViewport::wear_os_medium()));
	WatchHost::present_app(&mut thin, &app).unwrap();
	assert!(
		thin.0
			.presentations()
			.contains(&WatchPresentation::Page("home".into()))
	);
	assert!(
		thin.0
			.presentations()
			.contains(&WatchPresentation::Ambient("tile".into()))
	);

	let mut wrong = Thin(RecordingHost::new(WatchViewport::watch_os_medium()));
	assert!(matches!(
		WatchHost::present_app(&mut wrong, &app),
		Err(WatchError::Unavailable(_))
	));
}

#[test]
fn allowed_interactive_widgets_compose() {
	let tree = View()
		.child(Text("label"))
		.child(Button("Go"))
		.child(Element::view().child(Text("nested")));
	measure(&tree, WatchBudget::page()).unwrap();
}
