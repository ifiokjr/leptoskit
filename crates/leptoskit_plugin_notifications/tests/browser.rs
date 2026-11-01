#![cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]

use leptoskit_bridge::Error;
use leptoskit_plugin_notifications::Identifier;
use leptoskit_plugin_notifications::Notification;
use leptoskit_plugin_notifications::Permission;
use leptoskit_plugin_notifications::web;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen(inline_js = r#"
let original;
let posted;
let closed;
let requests;
export function install() {
    original = Object.getOwnPropertyDescriptor(globalThis, "Notification");
    posted = [];
    closed = 0;
    requests = 0;
    class TestNotification {
        static permission = "default";
        static failure = false;
        static requestPermission() {
            requests++;
            this.permission = "granted";
            return Promise.resolve("granted");
        }
        constructor(title, options) {
            if (TestNotification.failure) throw new TypeError("service worker required");
            posted.push({title, options});
            this.closed = false;
        }
        close() {
            if (!this.closed) closed++;
            this.closed = true;
        }
    }
    Object.defineProperty(globalThis, "Notification", {configurable:true, value:TestNotification});
}
export function restore() {
    if (original) Object.defineProperty(globalThis, "Notification", original);
    else delete globalThis.Notification;
}
export function deny() { Notification.permission = "denied"; }
export function unsupported() {
    Notification.permission = "granted";
    Notification.failure = true;
}
export function postedCount() { return posted.length; }
export function closedCount() { return closed; }
export function requestCount() { return requests; }
export function contentMatches() {
    return posted.every(value => value.title === "Finished" &&
        value.options.body === "Plain <text>" &&
        value.options.tag === "leptoskit.notifications.job" &&
        value.options.silent === true);
}
"#)]
extern "C" {
	fn install();
	fn restore();
	fn deny();
	fn unsupported();
	#[wasm_bindgen(js_name = postedCount)]
	fn posted_count() -> u32;
	#[wasm_bindgen(js_name = closedCount)]
	fn closed_count() -> u32;
	#[wasm_bindgen(js_name = requestCount)]
	fn request_count() -> u32;
	#[wasm_bindgen(js_name = contentMatches)]
	fn content_matches() -> bool;
}

struct Fixture;
impl Drop for Fixture {
	fn drop(&mut self) {
		restore();
	}
}

#[wasm_bindgen_test]
async fn explicit_permission_delivery_and_cancellation() {
	install();
	let _fixture = Fixture;
	let notification = Notification::new(
		Identifier::new("job").unwrap(),
		"Finished",
		"Plain <text>",
		Identifier::new("general").unwrap(),
	)
	.unwrap();
	assert_eq!(web::permission(), Ok(Permission::NotDetermined));
	assert!(matches!(
		web::show(&notification),
		Err(Error::PermissionDenied)
	));
	assert_eq!(request_count(), 0, "posting must never open a prompt");
	assert_eq!(web::request_permission().await, Ok(Permission::Granted));
	let handle = web::show(&notification).unwrap();

	assert_eq!(posted_count(), 1);
	assert!(content_matches());
	handle.cancel();
	handle.cancel();
	assert_eq!(closed_count(), 1);
	drop(web::show(&notification).unwrap());
	assert_eq!(
		closed_count(),
		1,
		"dropping a handle must preserve delivery"
	);
	deny();

	assert_eq!(web::permission(), Ok(Permission::Denied));
	assert!(matches!(
		web::show(&notification),
		Err(Error::PermissionDenied)
	));
	assert_eq!(posted_count(), 2);
	unsupported();
	assert!(matches!(web::show(&notification), Err(Error::Unavailable)));
}
