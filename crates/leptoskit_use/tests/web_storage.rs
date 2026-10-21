#![cfg(all(
	feature = "local-storage",
	target_arch = "wasm32",
	target_os = "unknown"
))]

mod common;

use js_sys::Function;
use leptos::mount::mount_to;
use leptos::task::spawn_local;
use leptos::task::tick;
use leptoskit_use::*;
use reactive_graph::owner::Owner;
use reactive_graph::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
async fn local_storage_persists_namespaces_and_uses_the_async_contract() {
	let store = LocalStorage::<String>::new("leptoskit-use:persistence").unwrap();
	let isolated = LocalStorage::<String>::new("leptoskit-use:persistence:").unwrap();
	let owner = Owner::new();
	store.remove("key").unwrap();
	isolated.remove("key").unwrap();
	let hook = owner
		.with(|| use_async_storage(store.clone(), "key", "fallback".into()))
		.await
		.unwrap();
	assert_eq!(hook.value.get(), "fallback");
	(hook.set_value)("saved".into()).await.unwrap();
	let reopened = LocalStorage::<String>::new("leptoskit-use:persistence").unwrap();
	assert_eq!(reopened.get("key").unwrap(), Some("saved".into()));
	assert!(isolated.get("key").unwrap().is_none());
	reopened
		.set_async("key".into(), "external".into())
		.await
		.unwrap();
	(hook.reload)().await.unwrap();
	assert_eq!(hook.value.get(), "external");
	(hook.remove)().await.unwrap();
	assert_eq!(hook.value.get(), "fallback");
	assert!(store.get_async("key".into()).await.unwrap().is_none());
	store.remove_async("key".into()).await.unwrap();
	for key in ["", ":key", "../name", "🌍"] {
		store.set(key, &key.to_owned()).unwrap();
		assert_eq!(reopened.get(key).unwrap(), Some(key.to_owned()));
		store.remove(key).unwrap();
	}
}

#[wasm_bindgen_test]
async fn the_same_tailwind_screen_saves_through_browser_press_and_reopens() {
	let store = LocalStorage::<String>::new("leptoskit-use:shared-screen").unwrap();
	store.remove("name").unwrap();
	let owner = Owner::new();
	let hook = owner
		.with(|| use_async_storage(store.clone(), "name", "Guest".into()))
		.await
		.unwrap();
	let document = web_sys::window().unwrap().document().unwrap();
	let parent = document
		.create_element("div")
		.unwrap()
		.dyn_into::<web_sys::HtmlElement>()
		.unwrap();
	document.body().unwrap().append_child(&parent).unwrap();
	let mount = mount_to(parent.clone(), move || {
		move || {
			leptoskit_native::web::render(common::settings_screen(&hook, |task| {
				spawn_local(async move {
					task.await.unwrap();
				});
			}))
		}
	});
	tick().await;
	assert!(parent.text_content().unwrap().contains("Name: Guest"));
	let button = parent
		.query_selector("button")
		.unwrap()
		.unwrap()
		.dyn_into::<web_sys::HtmlElement>()
		.unwrap();
	button.click();
	tick().await;
	assert!(parent.text_content().unwrap().contains("Name: Ada"));
	let reopened = LocalStorage::<String>::new("leptoskit-use:shared-screen").unwrap();
	assert_eq!(reopened.get("name").unwrap(), Some("Ada".into()));
	drop(mount);
	parent.remove();
	store.remove("name").unwrap();
}

/// Replace only this suite's storage methods, using real JS exceptions from
/// the browser boundary. The returned closure restores the original method.
fn fail_method(method: &str) -> Function {
	Function::new_with_args(
		"method",
		r"
        const original = Storage.prototype[method];
        Storage.prototype[method] = function(key, ...rest) {
            if (key.includes('leptoskit-use:errors')) {
                throw new DOMException('forced failure', 'QuotaExceededError');
            }
            return original.call(this, key, ...rest);
        };
        return () => { Storage.prototype[method] = original; };
    ",
	)
	.call1(&JsValue::NULL, &JsValue::from_str(method))
	.unwrap()
	.dyn_into()
	.unwrap()
}

#[wasm_bindgen_test]
fn browser_exceptions_and_corrupt_values_are_returned_without_resetting_state() {
	let store = LocalStorage::<String>::new("leptoskit-use:errors").unwrap();
	store.set("key", &"saved".into()).unwrap();
	let owner = Owner::new();
	let hook = owner
		.with(|| use_storage(store.clone(), "key", "fallback".into()))
		.unwrap()
		.clone();
	for method in ["getItem", "setItem", "removeItem"] {
		let restore = fail_method(method);
		let result = match method {
			"getItem" => (hook.reload)(),
			"setItem" => (hook.set_value)("next".into()),
			_ => (hook.remove)(),
		};
		assert!(matches!(result, Err(StorageError::Browser(_))));
		assert_eq!(hook.value.get(), "saved");
		restore.call0(&JsValue::NULL).unwrap();
	}
	Function::new_no_args(
		r"
        const key = Object.keys(localStorage).find(key => key.includes('leptoskit-use:errors'));
        localStorage.setItem(key, 'malformed JSON');
    ",
	)
	.call0(&JsValue::NULL)
	.unwrap();
	assert!(matches!((hook.reload)(), Err(StorageError::Codec(_))));
	assert_eq!(hook.value.get(), "saved");
	store.remove("key").unwrap();
}

#[wasm_bindgen_test]
fn inaccessible_or_disabled_browser_storage_is_a_typed_error() {
	let install = Function::new_with_args(
		"mode",
		r"
        const original = Object.getOwnPropertyDescriptor(window, 'localStorage');
        Object.defineProperty(window, 'localStorage', {
            configurable: true,
            get() {
                if (mode === 'denied') throw new DOMException('denied', 'SecurityError');
                return undefined;
            }
        });
        return () => Object.defineProperty(window, 'localStorage', original);
    ",
	);
	for mode in ["denied", "disabled"] {
		let restore: Function = install
			.call1(&JsValue::NULL, &JsValue::from_str(mode))
			.unwrap()
			.dyn_into()
			.unwrap();
		let result = LocalStorage::<String>::new("leptoskit-use:unavailable");
		if mode == "denied" {
			assert!(matches!(result, Err(StorageError::Browser(_))));
		} else {
			assert!(matches!(result, Err(StorageError::Unavailable(_))));
		}
		restore.call0(&JsValue::NULL).unwrap();
	}
}
