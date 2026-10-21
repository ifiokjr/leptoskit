//! The gallery's conformance scenario on the desktop's real native widgets:
//! AppKit on macOS, GTK 4 on Linux (under a display), and Win32 on Windows.
//!
//! The iOS app (`gallery-ios --verify`) and the Android instrumentation run
//! the same scenario on their hosts: every screen renders with native widgets
//! (here through `gallery snapshot`, in `tests/cli.rs`), and pressing the
//! counter's native button updates the shared state without replacing the
//! control.
//!
//! AppKit and GTK only run on the thread that started them, so this binary
//! brings its own harness and runs every test on the main thread.

use libtest_mimic::Arguments;
use libtest_mimic::Trial;

fn main() {
	let mut args = Arguments::from_args();
	// A single test thread is the only mode in which the harness runs tests
	// on the main thread itself.
	args.test_threads = Some(1);
	libtest_mimic::run(&args, trials()).exit();
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn trials() -> Vec<Trial> {
	Vec::new()
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
fn trials() -> Vec<Trial> {
	[
		(
			"native_press_updates_the_shared_counter",
			scenario::native_press_updates_the_shared_counter as fn(),
		),
		(
			"press_only_activates_buttons",
			scenario::press_only_activates_buttons,
		),
		(
			"unknown_screens_are_rejected",
			scenario::unknown_screens_are_rejected,
		),
	]
	.into_iter()
	.map(|(name, test)| {
		Trial::test(name, move || {
			test();
			Ok(())
		})
	})
	.collect()
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
mod scenario {
	use gallery::ScreenState;
	use gallery::desktop;
	use gallery::desktop::Session;
	use gallery::native::default_assets;
	use leptoskit_yoga::NodeId;

	/// The first node under `node` whose text is `expected`.
	fn find(session: &Session, node: NodeId, expected: &str) -> Option<NodeId> {
		let gallery = session.borrow();
		if gallery.surface.props(node).text.as_deref() == Some(expected) {
			return Some(node);
		}
		let children = gallery.surface.children(node).to_vec();
		drop(gallery);
		children
			.into_iter()
			.find_map(|child| find(session, child, expected))
	}

	fn root(session: &Session) -> NodeId {
		session
			.borrow()
			.surface
			.root()
			.expect("a mounted screen has a root")
	}

	pub fn native_press_updates_the_shared_counter() {
		let host = desktop::open("counter", default_assets().unwrap()).unwrap();
		let session = host.session();
		let add = find(session, root(session), "Add one").expect("the counter has `Add one`");
		let initial = ScreenState::INITIAL_COUNT.to_string();
		assert!(find(session, root(session), &initial).is_some());

		assert!(host.press(add));
		assert!(host.press(add));

		let expected = ScreenState::INITIAL_COUNT + 2;
		assert_eq!(session.borrow().state.count(), expected);
		assert!(
			find(session, root(session), &expected.to_string()).is_some(),
			"the native label shows the new count"
		);
		assert_eq!(
			find(session, root(session), "Add one"),
			Some(add),
			"re-rendering kept the native button"
		);
	}

	pub fn press_only_activates_buttons() {
		let host = desktop::open("counter", default_assets().unwrap()).unwrap();
		let session = host.session();
		let label = find(session, root(session), "Daily taps").expect("the counter has a title");
		assert!(!host.press(label));
		assert!(!host.press(root(session)));
		assert_eq!(session.borrow().state.count(), ScreenState::INITIAL_COUNT);
	}

	pub fn unknown_screens_are_rejected() {
		let Err(error) = desktop::open("missing", default_assets().unwrap()) else {
			panic!("`missing` is not a gallery screen");
		};
		assert!(error.contains("no screen named `missing`"), "{error}");
	}
}
