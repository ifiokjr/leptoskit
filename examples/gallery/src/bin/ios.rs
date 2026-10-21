//! The gallery's UIKit application.
//!
//! `gallery-ios <screen> [--verify]` shows one shared screen in a window
//! scene. With `--verify` it also inspects the mounted UIKit views, presses the
//! counter's native button, and writes `VERIFIED` or `FAILED` to
//! `Documents/gallery-<screen>.txt` in the app's data container. The bundle's
//! Info.plist opts into window scenes.

#[cfg(target_os = "ios")]
fn main() {
	app::run();
}

#[cfg(not(target_os = "ios"))]
fn main() -> std::process::ExitCode {
	eprintln!("gallery-ios runs inside an iOS application bundle; see examples/gallery/readme.md");
	std::process::ExitCode::FAILURE
}

#[cfg(target_os = "ios")]
mod app {
	use gallery::Screen;
	use gallery::ScreenState;
	use leptoskit::uikit::MainThreadMarker;
	use leptoskit::uikit::UIKitHost;
	use leptoskit::uikit::run_application;
	use objc2::Message;
	use objc2::rc::Retained;
	use objc2_ui_kit::UIButton;
	use objc2_ui_kit::UIControlEvents;
	use objc2_ui_kit::UILabel;
	use objc2_ui_kit::UIView;
	use objc2_ui_kit::UIWindow;

	pub fn run() -> ! {
		let mtm = MainThreadMarker::new().expect("UIKit starts applications on the main thread");
		let mut args = std::env::args().skip(1);
		let slug = args.next().unwrap_or_else(|| "counter".into());
		let verify = args.any(|arg| arg == "--verify");
		let screen = gallery::native::screen(&slug).unwrap_or_else(|error| panic!("{error}"));
		let assets = gallery::native::default_assets().unwrap_or_else(|error| panic!("{error}"));

		run_application(mtm, move |mtm, scene| {
			let state = ScreenState::new(assets.clone());
			let host = UIKitHost::new(mtm, {
				let state = state.clone();
				move || screen.render(&state)
			});
			host.set_dark_mode(false);
			let window = host.open_window(scene);
			if verify {
				report(screen, &state, &window);
			}
			window
		})
	}

	fn descendants(root: &UIView) -> Vec<Retained<UIView>> {
		let mut found = Vec::new();
		let mut pending = vec![root.retain()];
		while let Some(view) = pending.pop() {
			pending.extend(view.subviews().iter());
			found.push(view);
		}
		found
	}

	fn label_texts(views: &[Retained<UIView>]) -> Vec<String> {
		views
			.iter()
			.filter_map(|view| view.downcast_ref::<UILabel>()?.text())
			.map(|text| text.to_string())
			.collect()
	}

	/// Checks the native views a freshly opened window mounted.
	fn verify(
		screen: &Screen,
		state: &ScreenState,
		window: &UIWindow,
	) -> Result<Vec<String>, String> {
		// UIKit shows the window after the launch callback returns. Show and
		// lay it out now, so the checks cover its on-screen geometry.
		window.makeKeyAndVisible();
		window.layoutIfNeeded();
		if !window.isKeyWindow() {
			return Err("the gallery window did not become key".into());
		}
		let root = window
			.rootViewController()
			.and_then(|controller| controller.view())
			.ok_or("the window has no root view")?;
		let (shown, screen_size) = (root.bounds().size, window.bounds().size);
		// Layout is in points; anything under half a point is the same pixel.
		let fills = (shown.width - screen_size.width).abs() < 0.5
			&& (shown.height - screen_size.height).abs() < 0.5;
		if shown.width < 1.0 || !fills {
			return Err(format!(
				"the root view is {}x{} in a {}x{} window",
				shown.width, shown.height, screen_size.width, screen_size.height
			));
		}
		let views = descendants(&root);
		let texts = label_texts(&views);
		if texts.is_empty() {
			return Err(format!("`{}` mounted no native labels", screen.slug));
		}
		let mut passed = vec![format!(
			"PASS\t{} fills its {}x{} window with {} UIKit views and {} labels",
			screen.slug,
			screen_size.width,
			screen_size.height,
			views.len(),
			texts.len()
		)];

		if screen.slug == "counter" {
			let button = views
				.iter()
				.filter_map(|view| view.downcast_ref::<UIButton>())
				.find(|button| {
					button
						.currentTitle()
						.is_some_and(|title| title.to_string() == "Add one")
				})
				.ok_or("the counter has no native `Add one` button")?
				.retain();
			button.sendActionsForControlEvents(UIControlEvents::PrimaryActionTriggered);
			let expected = ScreenState::INITIAL_COUNT + 1;
			if state.count() != expected {
				return Err(format!(
					"pressing `Add one` left the count at {}",
					state.count()
				));
			}
			if !label_texts(&descendants(&root)).contains(&expected.to_string()) {
				return Err(format!("no native label shows the new count {expected}"));
			}
			passed.push("PASS\tthe native `Add one` button reconciled the shared count".into());
		}
		Ok(passed)
	}

	fn report(screen: &Screen, state: &ScreenState, window: &UIWindow) {
		let report = match verify(screen, state, window) {
			Ok(lines) => format!("VERIFIED\t{}\n{}\n", screen.slug, lines.join("\n")),
			Err(error) => format!("FAILED\t{error}\n"),
		};
		let home = std::env::var_os("HOME").expect("iOS sets HOME to the app's data container");
		let path = std::path::Path::new(&home)
			.join("Documents")
			.join(format!("gallery-{}.txt", screen.slug));
		std::fs::write(&path, &report).expect("the app's Documents directory is writable");
		eprint!("{report}");
	}
}
