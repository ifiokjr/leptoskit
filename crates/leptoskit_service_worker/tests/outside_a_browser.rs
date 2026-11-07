//! Outside a browser every service worker API is unavailable, so an app's
//! code compiles and its tests run on any host. Each call names the browser
//! API it would have called.
#![cfg(not(target_arch = "wasm32"))]

use futures::executor::block_on;
use leptoskit_service_worker::CacheRule;
use leptoskit_service_worker::Error;
use leptoskit_service_worker::PushOptions;
use leptoskit_service_worker::RegisterOptions;
use leptoskit_service_worker::Strategy;
use leptoskit_service_worker::WorkerMessage;
use leptoskit_service_worker::controller;
use leptoskit_service_worker::delete;
use leptoskit_service_worker::get_registration;
use leptoskit_service_worker::has;
use leptoskit_service_worker::keys;
use leptoskit_service_worker::on_controller_change;
use leptoskit_service_worker::on_message;
use leptoskit_service_worker::open;
use leptoskit_service_worker::post_message;
use leptoskit_service_worker::ready;
use leptoskit_service_worker::register;
use leptoskit_service_worker::send;

fn unavailable(path: &str) -> Error {
	Error::Unavailable(path.to_owned())
}

#[test]
fn registration_names_its_apis() {
	assert!(matches!(
		block_on(register("/sw.js", RegisterOptions::default())),
		Err(error) if error == unavailable("navigator.serviceWorker.register")
	));
	assert!(matches!(
		block_on(ready()),
		Err(error) if error == unavailable("navigator.serviceWorker.ready")
	));
	assert!(matches!(
		block_on(get_registration(None)),
		Err(error) if error == unavailable("navigator.serviceWorker.getRegistration")
	));
	assert!(matches!(
		controller(),
		Err(error) if error == unavailable("navigator.serviceWorker.controller")
	));
}

#[test]
fn messaging_and_caches_name_their_apis() {
	assert_eq!(
		post_message(&"ping"),
		Err(unavailable("navigator.serviceWorker.controller"))
	);
	assert_eq!(
		send(&WorkerMessage::SkipWaiting),
		Err(unavailable("navigator.serviceWorker.controller"))
	);
	assert!(matches!(
		on_message::<String, _>(|_| {}),
		Err(error) if error == unavailable("navigator.serviceWorker.onmessage")
	));
	assert!(matches!(
		on_controller_change(|| {}),
		Err(error) if error == unavailable("navigator.serviceWorker.oncontrollerchange")
	));

	assert!(matches!(
		block_on(open("static-v1")),
		Err(error) if error == unavailable("caches.open")
	));
	assert_eq!(block_on(has("static-v1")), Err(unavailable("caches.has")));
	assert_eq!(
		block_on(delete("static-v1")),
		Err(unavailable("caches.delete"))
	);
	assert_eq!(block_on(keys()), Err(unavailable("caches.keys")));
}

#[test]
fn strategies_are_usable_without_a_browser() {
	let rule = CacheRule::new("static-v1", Strategy::cache_first()).prefixes(["/assets/"]);
	assert!(rule.matches_url("/assets/app.wasm"));
	assert_eq!(
		Strategy::network_first().prefer(),
		leptoskit_service_worker::Source::Network
	);
	let _ = PushOptions::new("BNcRdytQv3RZjq3...");
	assert_eq!(
		unavailable("caches.open").to_string(),
		"`caches.open` isn't available in this browser"
	);
}
