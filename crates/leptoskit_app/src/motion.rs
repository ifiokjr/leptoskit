//! App-owned, vector-only motion on the existing Rive integration.
use std::sync::LazyLock;

use leptoskit::prelude::*;
use leptoskit::rive::Asset;
use leptoskit::rive::Name;
use leptoskit::rive::Playback;
use leptoskit::rive::Scene;

#[cfg(all(target_arch = "wasm32", feature = "web"))]
#[derive(Clone, Copy)]
struct MotionReady;

static BRAND: LazyLock<Asset> = LazyLock::new(|| {
	Asset::new(include_bytes!("../web/rive/brand.riv")).expect("verified, app-owned Rive asset")
});

/// A short, one-shot timeline whose first and last poses preserve the mark.
/// Host visibility and reduced-motion policies govern actual playback.
pub fn brand_playback() -> Playback {
	Playback::new(BRAND.clone()).scene(Scene::Animation(
		Name::new("Settle").expect("fixed timeline name"),
	))
}

pub fn mark() -> Element {
	// Static server markup has a visible pose without requiring JavaScript.
	#[cfg(not(target_arch = "wasm32"))]
	return Image("/docs/assets/mark.svg")
		.class("w-12 h-12")
		.aria_hidden(true);
	#[cfg(all(target_arch = "wasm32", feature = "web", not(feature = "native")))]
	if leptos::prelude::use_context::<MotionReady>().is_none() {
		return Image("/docs/assets/mark.svg")
			.class("w-12 h-12")
			.aria_hidden(true);
	}
	#[cfg(target_arch = "wasm32")]
	Rive(brand_playback())
		.class("w-12 h-12")
		.running(true)
		.aria_hidden(true)
}

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub fn provide_runtime() {
	if let Ok(runtime) = leptoskit::rive::web::Runtime::from_global("/rive/rive.wasm") {
		leptoskit::web::provide_rive_runtime(runtime);
		leptos::prelude::provide_context(MotionReady);
	}
}
