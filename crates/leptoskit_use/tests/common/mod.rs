use leptoskit_native::Element;
use leptoskit_native::components::Button;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_use::AsyncStorageError;
use leptoskit_use::StorageError;
use leptoskit_use::StorageFuture;
use leptoskit_use::UseAsyncStorageReturn;
use reactive_graph::prelude::*;

/// Exactly the same screen and callback run through both renderers. Only the
/// host's storage directory/namespace and task scheduler differ.
pub fn settings_screen(
	storage: &UseAsyncStorageReturn<String, StorageError>,
	run: impl Fn(StorageFuture<'static, Result<(), AsyncStorageError<StorageError>>>)
	+ Send
	+ Sync
	+ 'static,
) -> Element {
	let save = storage.set_value.clone();
	View().class("gap-4 p-4 md:flex-row").children([
		Text(format!("Name: {}", storage.value.get())),
		Button("Save Ada").on_press(move || run(save("Ada".into()))),
	])
}
