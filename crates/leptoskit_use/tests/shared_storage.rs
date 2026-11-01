#![cfg(all(feature = "local-storage", not(target_arch = "wasm32")))]

mod common;

use futures::executor::block_on;
use leptoskit_native::EventOutcome;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeSurface;
use leptoskit_use::*;
use reactive_graph::owner::Owner;

#[test]
fn shared_tailwind_screen_persists_through_native_press_and_reopens_after_restart() {
	let directory = tempfile::tempdir().unwrap();
	let store = LocalStorage::<String>::new(directory.path()).unwrap();
	let owner = Owner::new();
	let hook =
		block_on(owner.with(|| use_async_storage(store, "name", "Guest".to_owned()))).unwrap();
	let cloned = hook.clone();
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(900.5, 600.25);

	surface.set_root(&common::settings_screen(&cloned, |task| {
		block_on(task).unwrap();
	}));
	let root = surface.root().unwrap();
	let label = surface.children(root)[0];
	let button = surface.children(root)[1];
	assert_eq!(surface.props(label).text.as_deref(), Some("Name: Guest"));
	assert_eq!(
		surface.dispatch_event(button, &NativeEvent::Press),
		EventOutcome::Handled
	);
	surface.set_root(&common::settings_screen(&hook, |task| {
		block_on(task).unwrap();
	}));
	assert_eq!(surface.children(root)[0], label);
	assert_eq!(surface.props(label).text.as_deref(), Some("Name: Ada"));
	owner.cleanup();
	let reopened = LocalStorage::<String>::new(directory.path()).unwrap();
	let next_owner = Owner::new();
	let next = next_owner
		.with(|| use_storage(reopened, "name", "Guest".into()))
		.unwrap();
	let clone = next.clone();
	use reactive_graph::prelude::Get;
	assert_eq!(clone.value.get(), "Ada");
}
