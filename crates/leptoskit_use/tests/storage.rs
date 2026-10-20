use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use leptoskit_use::*;
use reactive_graph::owner::Owner;
use reactive_graph::prelude::*;

#[test]
fn storage_persists_reloads_removes_and_keeps_keys_and_requests_isolated() {
	Owner::new().with(|| {
		let store = MemoryStorage::<String>::default();
		let first = use_storage(store.clone(), "name", "fallback".into()).unwrap();
		assert_eq!(first.value.get(), "fallback");
		assert_eq!(store.get("name"), Ok(None));
		(first.set_value)("saved".into()).unwrap();
		assert_eq!(store.get("name"), Ok(Some("saved".into())));
		let second = use_storage(store.clone(), "name", "other".into()).unwrap();
		assert_eq!(second.value.get(), "saved");
		(first.set_value)("changed".into()).unwrap();
		assert_eq!(second.value.get(), "saved");
		(second.reload)().unwrap();
		assert_eq!(second.value.get(), "changed");
		store.set("another", &"untouched".into()).unwrap();
		(first.remove)().unwrap();
		assert_eq!(first.value.get(), "fallback");
		(second.reload)().unwrap();
		assert_eq!(second.value.get(), "other");
		(first.remove)().unwrap();
		assert_eq!(store.get("another"), Ok(Some("untouched".into())));
		let isolated = use_storage(
			MemoryStorage::default(),
			"another",
			"new request".to_owned(),
		)
		.unwrap();
		assert_eq!(isolated.value.get(), "new request");
	});
}

#[derive(Clone, Default)]
struct FailingStore {
	fail_read: Arc<AtomicBool>,
	fail_write: Arc<AtomicBool>,
	fail_remove: Arc<AtomicBool>,
	values: MemoryStorage<i32>,
}

impl Storage<i32> for FailingStore {
	type Error = &'static str;

	fn get(&self, key: &str) -> Result<Option<i32>, Self::Error> {
		if self.fail_read.load(Ordering::SeqCst) {
			Err("read failed")
		} else {
			Ok(self.values.get(key).unwrap())
		}
	}

	fn set(&self, key: &str, value: &i32) -> Result<(), Self::Error> {
		if self.fail_write.load(Ordering::SeqCst) {
			Err("write failed")
		} else {
			self.values.set(key, value).unwrap();
			Ok(())
		}
	}

	fn remove(&self, key: &str) -> Result<(), Self::Error> {
		if self.fail_remove.load(Ordering::SeqCst) {
			Err("remove failed")
		} else {
			self.values.remove(key).unwrap();
			Ok(())
		}
	}
}

#[test]
fn backend_errors_are_returned_and_do_not_change_visible_state() {
	Owner::new().with(|| {
		let store = FailingStore::default();
		store.fail_read.store(true, Ordering::SeqCst);
		assert!(matches!(
			use_storage(store.clone(), "key", 1),
			Err("read failed")
		));
		store.fail_read.store(false, Ordering::SeqCst);
		let hook = use_storage(store.clone(), "key", 1).unwrap();
		(hook.set_value)(2).unwrap();
		store.fail_write.store(true, Ordering::SeqCst);
		assert_eq!((hook.set_value)(3), Err("write failed"));
		assert_eq!(hook.value.get(), 2);
		assert_eq!(store.values.get("key"), Ok(Some(2)));
		store.fail_remove.store(true, Ordering::SeqCst);
		assert_eq!((hook.remove)(), Err("remove failed"));
		assert_eq!(hook.value.get(), 2);
		store.fail_read.store(true, Ordering::SeqCst);
		assert_eq!((hook.reload)(), Err("read failed"));
		assert_eq!(hook.value.get(), 2);
		store.fail_remove.store(false, Ordering::SeqCst);
		(hook.remove)().unwrap();
		assert_eq!(hook.value.get(), 1);
	});
}
