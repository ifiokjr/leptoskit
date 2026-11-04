//! Outside a browser every extension API is unavailable, so an app's
//! extension code compiles and its tests run on any host. Each call names
//! the browser API it would have called, which pins every wrapper to the
//! right function in `chrome.*` and `browser.*`.
#![cfg(not(target_arch = "wasm32"))]

use futures::executor::block_on;
use leptoskit_extension::Background;
use leptoskit_extension::Error;
use leptoskit_extension::PageName;
use leptoskit_extension::Surface;
use leptoskit_extension::action;
use leptoskit_extension::alarms;
use leptoskit_extension::background;
use leptoskit_extension::commands;
use leptoskit_extension::context_menus;
use leptoskit_extension::devtools;
use leptoskit_extension::notifications;
use leptoskit_extension::page;
use leptoskit_extension::permissions;
use leptoskit_extension::runtime;
use leptoskit_extension::scripting;
use leptoskit_extension::side_panel;
use leptoskit_extension::storage;
use leptoskit_extension::tabs;
use serde_json::Value;
use serde_json::json;

fn unavailable(path: &str) -> Error {
	Error::Unavailable(path.to_owned())
}

#[test]
fn the_toolbar_alarms_shortcuts_and_menus_name_their_apis() {
	assert_eq!(
		block_on(action::set_badge("3", Some(7))),
		Err(unavailable("action.setBadgeText"))
	);
	assert_eq!(
		block_on(action::set_badge("", None)),
		Err(unavailable("action.setBadgeText"))
	);
	assert_eq!(
		block_on(action::set_badge_color("#ff0000")),
		Err(unavailable("action.setBadgeBackgroundColor"))
	);
	assert_eq!(
		block_on(action::set_title("Clip")),
		Err(unavailable("action.setTitle"))
	);
	assert_eq!(
		block_on(action::open_popup()),
		Err(unavailable("action.openPopup"))
	);

	assert_eq!(
		block_on(alarms::every("sync", 15.0)),
		Err(unavailable("alarms.create"))
	);
	assert_eq!(
		block_on(alarms::once("remind", 1.0)),
		Err(unavailable("alarms.create"))
	);
	assert_eq!(
		block_on(alarms::clear("sync")),
		Err(unavailable("alarms.clear"))
	);

	assert_eq!(
		block_on(commands::all()),
		Err(unavailable("commands.getAll"))
	);

	assert_eq!(
		block_on(context_menus::create(
			"clip",
			"Clip",
			&[
				context_menus::Context::Selection,
				context_menus::Context::Page,
				context_menus::Context::Link,
				context_menus::Context::Image,
			],
		)),
		Err(unavailable("contextMenus.create"))
	);
	assert_eq!(
		block_on(context_menus::remove_all()),
		Err(unavailable("contextMenus.removeAll"))
	);
}

#[test]
fn notifications_permissions_and_the_runtime_name_their_apis() {
	let origins = vec!["https://example.org/*".to_owned()];

	// The icon's address is looked up first, and its absence doesn't stop
	// the notification: only the notification's own API does.
	assert_eq!(
		block_on(notifications::show("saved", "Saved", "The clip is saved")),
		Err(unavailable("notifications.create"))
	);

	assert_eq!(
		block_on(permissions::request(&origins)),
		Err(unavailable("permissions.request"))
	);
	assert_eq!(
		block_on(permissions::contains(&origins)),
		Err(unavailable("permissions.contains"))
	);
	assert_eq!(
		block_on(permissions::remove(&origins)),
		Err(unavailable("permissions.remove"))
	);

	assert_eq!(
		block_on(runtime::send::<_, Value>(&json!({ "kind": "ping" }))),
		Err(unavailable("runtime.sendMessage"))
	);
	assert_eq!(runtime::id(), Err(unavailable("runtime.id")));
	assert_eq!(
		block_on(runtime::url("assets/icon-128.png")),
		Err(unavailable("runtime.getURL"))
	);
	assert_eq!(
		block_on(runtime::open_options()),
		Err(unavailable("runtime.openOptionsPage"))
	);
	assert_eq!(
		block_on(runtime::reload()),
		Err(unavailable("runtime.reload"))
	);
}

#[test]
fn scripts_and_tabs_name_their_apis() {
	let origins = vec!["https://example.org/*".to_owned()];

	assert_eq!(
		block_on(scripting::inject(4, "clipper")),
		Err(unavailable("scripting.executeScript"))
	);
	assert_eq!(
		block_on(scripting::register("clipper", &origins)),
		Err(unavailable("scripting.registerContentScripts"))
	);
	assert_eq!(
		block_on(scripting::unregister("clipper")),
		Err(unavailable("scripting.unregisterContentScripts"))
	);
	assert_eq!(
		block_on(scripting::registered("clipper")),
		Err(unavailable("scripting.getRegisteredContentScripts"))
	);

	assert_eq!(block_on(tabs::active()), Err(unavailable("tabs.query")));
	assert_eq!(
		block_on(tabs::open("https://example.org")),
		Err(unavailable("tabs.create"))
	);
	assert_eq!(
		block_on(tabs::send::<_, Value>(4, &json!({ "kind": "ping" }))),
		Err(unavailable("tabs.sendMessage"))
	);

	assert!(matches!(block_on(page::connect()), Err(error) if error == unavailable("page")));
}

#[test]
fn each_storage_area_calls_its_own_api() {
	for (area, path) in [
		(storage::LOCAL, "storage.local"),
		(storage::SESSION, "storage.session"),
		(storage::SYNC, "storage.sync"),
	] {
		assert_eq!(
			block_on(area.get::<String>("draft")),
			Err(unavailable(&format!("{path}.get")))
		);
		assert_eq!(
			block_on(area.set("draft", &json!({ "text": "hello" }))),
			Err(unavailable(&format!("{path}.set")))
		);
		assert_eq!(
			block_on(area.remove("draft")),
			Err(unavailable(&format!("{path}.remove")))
		);
		assert!(matches!(
			storage::on_changed(area, |_, _| {}),
			Err(error) if error == unavailable("storage.onChanged")
		));
	}
}

#[test]
fn the_side_panel_and_developer_tools_are_absent() {
	assert!(!side_panel::available());
	assert_eq!(block_on(side_panel::open(1)), Err(unavailable("sidePanel")));

	assert!(!devtools::available());
	assert_eq!(
		block_on(devtools::create_panel("Inspector", "assets/icon-32.png")),
		Err(unavailable("devtools.panels.create"))
	);
	assert_eq!(
		devtools::inspected_tab(),
		Err(unavailable("devtools.inspectedWindow.tabId"))
	);
	assert_eq!(
		block_on(devtools::eval::<Value>("document.title")),
		Err(unavailable("devtools.inspectedWindow.eval"))
	);
	assert_eq!(
		block_on(devtools::inspected_session()),
		Err(unavailable("devtools.inspectedWindow.eval"))
	);
	assert!(matches!(
		devtools::on_navigated(|_| {}),
		Err(error) if error == unavailable("devtools.network.onNavigated")
	));
	assert!(matches!(
		devtools::watch_session(|_| {}),
		Err(error) if error == unavailable("devtools.inspectedWindow")
	));
	assert!(matches!(
		devtools::HubFrame::new("36px"),
		Err(error) if error == unavailable("document")
	));
}

#[test]
fn errors_say_what_went_wrong() {
	assert_eq!(
		unavailable("tabs.query").to_string(),
		"`tabs.query` isn't available in this browser"
	);
	assert_eq!(
		Error::Browser("no tab with id 4".into()).to_string(),
		"no tab with id 4"
	);
	assert_eq!(
		Error::Json("expected a string".into()).to_string(),
		"couldn't convert a value: expected a string"
	);
}

#[test]
fn every_page_surface_round_trips_its_name() {
	for page in [
		PageName::Popup,
		PageName::SidePanel,
		PageName::Options,
		PageName::NewTab,
		PageName::Devtools,
		PageName::DevtoolsPanel,
	] {
		assert_eq!(
			Surface::parse(page.name(), "/"),
			Some(Surface::Page {
				name: page,
				route: "/".into()
			})
		);
	}
}

fn worker(worker: &Background) {
	worker.on_installed(|installed| {
		async move {
			assert_eq!(installed.reason, "update");
			assert_eq!(installed.previous_version.as_deref(), Some("1.0"));
		}
	});
	worker.on_startup(|| async {});
	worker.on_alarm(|alarm| {
		async move {
			assert_eq!(alarm.name, "sync");
		}
	});
	worker.on_context_menu(|click| {
		async move {
			assert_eq!(click.menu_item_id, json!("clip"));
			assert_eq!(click.selection_text.as_deref(), Some("a quote"));
			// Chromium sends the tab beside the click rather than in it.
			assert_eq!(click.tab.and_then(|tab| tab.id), Some(9));
		}
	});
	worker.on_tab_updated(|update| {
		async move {
			assert_eq!(update.tab_id, 9);
			assert_eq!(update.change_info["status"], "complete");
			assert_eq!(update.tab.url.as_deref(), Some("https://example.org/"));
		}
	});
}

#[test]
fn the_worker_answers_each_browser_event() {
	background::start(worker);

	for (kind, detail) in [
		(
			"runtime.installed",
			json!({ "reason": "update", "previousVersion": "1.0" }),
		),
		("runtime.startup", json!(null)),
		(
			"alarms",
			json!({ "name": "sync", "scheduledTime": 1_700_000_000_000.0 }),
		),
		(
			"context_menus",
			json!({
				"info": { "menuItemId": "clip", "selectionText": "a quote" },
				"tab": { "id": 9, "active": true },
			}),
		),
		(
			"tabs.updated",
			json!({
				"tabId": 9,
				"changeInfo": { "status": "complete" },
				"tab": { "id": 9, "url": "https://example.org/" },
			}),
		),
	] {
		assert_eq!(
			block_on(background::simulate(kind, detail)),
			Ok(Value::Null),
			"{kind}"
		);
	}

	// An event whose detail doesn't fit is refused rather than handled.
	assert!(block_on(background::simulate("alarms", json!({ "name": 3 }))).is_err());
	assert_eq!(
		block_on(background::simulate("tabs.removed", json!({}))),
		Err("nothing handles `tabs.removed`".to_owned())
	);
}
