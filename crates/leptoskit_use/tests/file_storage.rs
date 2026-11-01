#![cfg(all(feature = "local-storage", not(target_arch = "wasm32")))]

use std::fs;
use std::sync::mpsc;
use std::thread;

use futures::executor::block_on;
use leptoskit_use::*;
use reactive_graph::owner::Owner;
use reactive_graph::prelude::*;
use serde::Deserialize;
use serde::Serialize;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct Settings {
	dark: bool,
	name: String,
}

#[test]
fn file_storage_reopens_persisted_values_and_keeps_keys_within_its_directory() {
	let directory = tempfile::tempdir().unwrap();
	let root = directory.path().join("app/preferences");
	let store = FileStorage::<Settings>::new(&root).unwrap();
	let value = Settings {
		dark: true,
		name: "Ada".into(),
	};

	for key in ["", "../outside", "/absolute", "🌍/name", &"x".repeat(1000)] {
		assert!(store.get(key).unwrap().is_none());
		store.set(key, &value).unwrap();
		let reopened = FileStorage::new(&root).unwrap();

		assert_eq!(reopened.get(key).unwrap(), Some(value.clone()));
		store.remove(key).unwrap();
		store.remove(key).unwrap();
		assert!(reopened.get(key).unwrap().is_none());
	}

	assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
	assert!(!directory.path().join("outside").exists());
	let other = FileStorage::<Settings>::new(directory.path().join("another-app")).unwrap();
	store.set("key", &value).unwrap();
	assert!(other.get("key").unwrap().is_none());
	let second = Settings {
		dark: false,
		name: "Grace".into(),
	};
	store.set("key", &second).unwrap();
	assert_eq!(store.get("key").unwrap(), Some(second));
}

#[test]
fn async_file_storage_runs_without_a_runtime_and_preserves_reactive_state() {
	let directory = tempfile::tempdir().unwrap();
	let store = LocalStorage::<String>::new(directory.path()).unwrap();
	let owner = Owner::new();
	let hook =
		block_on(owner.with(|| use_async_storage(store.clone(), "name", "initial".to_owned())))
			.unwrap();
	block_on((hook.set_value)("saved".into())).unwrap();
	assert_eq!(hook.value.get(), "saved");
	assert_eq!(
		block_on(store.get_async("name".into())).unwrap(),
		Some("saved".into())
	);

	let reopened = LocalStorage::<String>::new(directory.path()).unwrap();
	block_on(reopened.set_async("name".into(), "external".into())).unwrap();
	block_on((hook.reload)()).unwrap();
	assert_eq!(hook.value.get(), "external");
	block_on((hook.remove)()).unwrap();
	block_on(store.remove_async("name".into())).unwrap();
	assert_eq!(hook.value.get(), "initial");
}

#[derive(Clone, Deserialize)]
#[serde(from = "u32")]
struct Observed {
	value: u32,
	observer: Option<mpsc::Sender<thread::ThreadId>>,
}

impl From<u32> for Observed {
	fn from(value: u32) -> Self {
		Self {
			value,
			observer: None,
		}
	}
}

impl Serialize for Observed {
	fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		self.observer
			.as_ref()
			.unwrap()
			.send(thread::current().id())
			.unwrap();
		self.value.serialize(serializer)
	}
}

#[test]
fn async_native_io_and_serialization_run_off_the_callers_thread() {
	let directory = tempfile::tempdir().unwrap();
	let store = FileStorage::new(directory.path()).unwrap();
	let (send, receive) = mpsc::channel();
	let caller = thread::current().id();
	block_on(store.set_async(
		"key".into(),
		Observed {
			value: 4,
			observer: Some(send),
		},
	))
	.unwrap();
	assert_ne!(receive.recv().unwrap(), caller);
	assert_eq!(
		block_on(store.get_async("key".into()))
			.unwrap()
			.unwrap()
			.value,
		4
	);
}

#[derive(Deserialize)]
struct CannotEncode;

impl Serialize for CannotEncode {
	fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
		Err(serde::ser::Error::custom("cannot encode"))
	}
}

#[test]
fn encoding_and_io_errors_are_returned_without_hiding_corrupt_data() {
	let directory = tempfile::tempdir().unwrap();
	let store = FileStorage::<String>::new(directory.path()).unwrap();
	store.set("key", &"saved".into()).unwrap();
	let bad = FileStorage::new(directory.path()).unwrap();
	assert!(matches!(
		bad.set("key", &CannotEncode),
		Err(StorageError::Codec(_))
	));
	assert_eq!(store.get("key").unwrap(), Some("saved".into()));
	let path = fs::read_dir(directory.path())
		.unwrap()
		.next()
		.unwrap()
		.unwrap()
		.path();
	let owner = Owner::new();

	let hook = owner
		.with(|| use_storage(store.clone(), "key", "fallback".into()))
		.unwrap();
	fs::write(&path, b"not JSON").unwrap();
	assert!(matches!((hook.reload)(), Err(StorageError::Codec(_))));
	assert_eq!(hook.value.get(), "saved");
	fs::remove_file(&path).unwrap();
	fs::create_dir(&path).unwrap();
	assert!(matches!(store.get("key"), Err(StorageError::Io(_))));
	assert!(matches!(store.remove("key"), Err(StorageError::Io(_))));
	assert!(matches!(
		store.set("key", &"next".into()),
		Err(StorageError::Io(_))
	));
	let file = directory.path().join("plain-file");
	fs::write(&file, b"file").unwrap();
	assert!(matches!(
		FileStorage::<String>::new(&file),
		Err(StorageError::Io(_))
	));
}
