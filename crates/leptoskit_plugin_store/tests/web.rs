#![cfg(all(target_arch = "wasm32", not(feature = "guest")))]

use leptoskit_bridge::Error;
use leptoskit_plugin_store::Change;
use leptoskit_plugin_store::Save;
use leptoskit_plugin_store::Value;
use leptoskit_plugin_store::web::STORAGE_KEY;
use leptoskit_plugin_store::web::Store;
use serde_json::json;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen(inline_js = r"
const originalSet = Storage.prototype.setItem;
const originalGet = Storage.prototype.getItem;
export function rejectStoreWrite(name) {
  Storage.prototype.setItem = function(key, value) {
    if (key === 'leptoskit:store:v1') throw new DOMException('test storage failure', name);
    return originalSet.call(this, key, value);
  };
}
export function rejectStoreRead() {
  Storage.prototype.getItem = function(key) {
    if (key === 'leptoskit:store:v1') throw new DOMException('test storage failure', 'SecurityError');
    return originalGet.call(this, key);
  };
}
export function restoreStoreStorage() {
  Storage.prototype.setItem = originalSet;
  Storage.prototype.getItem = originalGet;
}
")]
extern "C" {
	#[wasm_bindgen(js_name = rejectStoreWrite)]
	fn reject_write(name: &str);
	#[wasm_bindgen(js_name = rejectStoreRead)]
	fn reject_read();
	#[wasm_bindgen(js_name = restoreStoreStorage)]
	fn restore_storage();
}

struct Fixture {
	storage: web_sys::Storage,
	previous: Option<String>,
}

impl Fixture {
	fn new() -> Self {
		let storage = web_sys::window().unwrap().local_storage().unwrap().unwrap();
		let previous = storage.get_item(STORAGE_KEY).unwrap();
		storage.remove_item(STORAGE_KEY).unwrap();
		Self { storage, previous }
	}
}
impl Drop for Fixture {
	fn drop(&mut self) {
		restore_storage();

		match &self.previous {
			Some(value) => self.storage.set_item(STORAGE_KEY, value).unwrap(),
			None => self.storage.remove_item(STORAGE_KEY).unwrap(),
		}
	}
}

#[wasm_bindgen_test]
fn real_local_storage_reload_atomic_failure_clear_and_error_mapping() {
	let fixture = Fixture::new();
	let mut store = Store::open().unwrap();
	store.set("theme", json!({"name":"dark"})).unwrap();
	assert_eq!(fixture.storage.get_item(STORAGE_KEY).unwrap(), None);
	store.save().unwrap();
	assert_eq!(
		Store::open().unwrap().get("theme").unwrap(),
		Some(&json!({"name":"dark"}))
	);
	let saved = fixture.storage.get_item(STORAGE_KEY).unwrap();

	reject_write("QuotaExceededError");
	assert_eq!(
		store.apply(&[Change::Set("theme".into(), json!("new"))], Save::Commit),
		Err(Error::TooLarge)
	);
	assert_eq!(store.get("theme").unwrap(), Some(&json!({"name":"dark"})));
	assert_eq!(fixture.storage.get_item(STORAGE_KEY).unwrap(), saved);
	reject_write("SecurityError");
	assert_eq!(store.save(), Err(Error::PermissionDenied));
	reject_write("UnknownError");
	assert_eq!(store.save(), Err(Error::Native));

	restore_storage();

	fixture.storage.set_item(STORAGE_KEY, "broken").unwrap();
	assert_eq!(store.load(), Err(Error::InvalidArgument));
	assert_eq!(store.get("theme").unwrap(), Some(&json!({"name":"dark"})));
	store.save().unwrap();
	store
		.apply(
			&[Change::Clear, Change::Set("null".into(), Value::Null)],
			Save::Commit,
		)
		.unwrap();
	let restored = Store::open().unwrap();
	assert_eq!(restored.get("null").unwrap(), Some(&Value::Null));
	assert_eq!(restored.get("missing").unwrap(), None);

	store.clear().unwrap();
	assert_eq!(Store::open().unwrap().entries().len(), 1);
	store.save().unwrap();
	assert!(Store::open().unwrap().entries().is_empty());
	reject_read();
	assert!(matches!(Store::open(), Err(Error::PermissionDenied)));
}

#[cfg(feature = "async-storage")]
mod async_storage {
	use futures::executor::block_on;
	use leptoskit_plugin_store::StoreStorage;
	use leptoskit_use::AsyncStorage;
	use leptoskit_use::use_async_storage;
	use reactive_graph::owner::Owner;
	use reactive_graph::prelude::Get;

	use super::*;

	#[wasm_bindgen_test]
	fn adapter_round_trips_reloads_and_resets_through_use_async_storage() {
		let fixture = Fixture::new();
		let owner = Owner::new();
		let storage = StoreStorage::<String>::new(Store::open().unwrap());
		let hook = block_on(owner.with(|| use_async_storage(storage, "name", "Guest".to_owned())))
			.unwrap();
		assert_eq!(hook.value.get(), "Guest");
		block_on((hook.set_value)("Ada".to_owned())).unwrap();
		assert_eq!(hook.value.get(), "Ada");

		let reloaded = StoreStorage::<String>::new(Store::open().unwrap());
		assert_eq!(
			block_on(reloaded.get_async("name".into())).unwrap(),
			Some("Ada".to_owned())
		);
		block_on((hook.remove)()).unwrap();
		assert_eq!(hook.value.get(), "Guest");
		owner.cleanup();
	}

	#[wasm_bindgen_test]
	fn rejected_browser_writes_preserve_the_last_committed_value() {
		let fixture = Fixture::new();
		let owner = Owner::new();
		let storage = StoreStorage::<String>::new(Store::open().unwrap());
		let hook = block_on(owner.with(|| use_async_storage(storage, "name", "Guest".to_owned())))
			.unwrap();
		block_on((hook.set_value)("Ada".to_owned())).unwrap();
		reject_write("QuotaExceededError");
		let error = block_on((hook.set_value)("Grace".to_owned()));
		restore_storage();

		assert!(matches!(
			error.unwrap_err(),
			leptoskit_use::AsyncStorageError::Backend(leptoskit_plugin_store::StoreError::Backend(
				_
			))
		));
		assert_eq!(hook.value.get(), "Ada");
		owner.cleanup();
	}
}
