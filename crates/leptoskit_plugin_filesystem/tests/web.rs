#![cfg(all(target_arch = "wasm32", not(feature = "guest")))]
use leptoskit_bridge::Error;
use leptoskit_plugin_filesystem::Kind;
use leptoskit_plugin_filesystem::MAX_FILE_BYTES;
use leptoskit_plugin_filesystem::web;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
async fn opfs_roundtrip_and_boundaries() {
	// One private test subtree, with a fresh name for concurrent browser runs.
	let root = format!("test-{}-{}", js_sys::Date::now(), js_sys::Math::random());
	let directory = format!("{root}/nested");
	let path = format!("{directory}/binary");
	web::mkdir(&directory).await.unwrap();
	web::write(&path, &[0, 1, 255]).await.unwrap();
	assert_eq!(web::read(&path).await.unwrap().as_ref(), &[0, 1, 255]);
	let metadata = web::stat(&path).await.unwrap();
	assert_eq!(metadata.kind, Kind::File);
	assert_eq!(metadata.len, 3);
	assert_eq!(web::list(&directory).await.unwrap(), ["binary"]);
	assert_eq!(web::stat(&directory).await.unwrap().kind, Kind::Directory);
	assert!(web::remove(&directory).await.is_err());
	assert_eq!(web::read("../outside").await, Err(Error::InvalidArgument));
	assert_eq!(
		web::write(&path, &vec![0; MAX_FILE_BYTES + 1]).await,
		Err(Error::TooLarge)
	);
	assert_eq!(web::read(&path).await.unwrap().as_ref(), &[0, 1, 255]);
	let moved = format!("{directory}/moved");
	web::rename(&path, &moved).await.unwrap();
	assert!(web::read(&path).await.is_err());
	assert_eq!(web::read(&moved).await.unwrap().as_ref(), &[0, 1, 255]);
	web::remove(&moved).await.unwrap();
	web::remove(&directory).await.unwrap();
	web::remove(&root).await.unwrap();
}
