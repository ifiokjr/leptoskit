use std::sync::Arc;

use leptoskit_plugin_menu::DesktopMenu;
use muda::ContextMenu;
use muda::MenuItemKindSnapshot;
use muda::MenuSnapshotHandle;
use muda::SubmenuSnapshot;

use super::Events;
use crate::Button;
use crate::Error;
use crate::Event;
use crate::EventKind;
use crate::Options;
use crate::Rect;
use crate::RgbaIcon;
use crate::TrayId;

struct Model {
	id: TrayId,
	name: String,
	icon: ksni::Icon,
	title: String,
	tooltip: String,
	visible: bool,
	menu: Option<MenuSnapshotHandle>,
	events: Arc<Events>,
}
impl ksni::Tray for Model {
	fn id(&self) -> String {
		self.name.clone()
	}

	fn title(&self) -> String {
		self.title.clone()
	}

	fn status(&self) -> ksni::Status {
		if self.visible {
			ksni::Status::Active
		} else {
			ksni::Status::Passive
		}
	}

	fn icon_pixmap(&self) -> Vec<ksni::Icon> {
		vec![self.icon.clone()]
	}

	fn tool_tip(&self) -> ksni::ToolTip {
		ksni::ToolTip {
			title: self.tooltip.clone(),
			..Default::default()
		}
	}

	fn activate(&mut self, _: i32, _: i32) {
		self.events.push(Event {
			id: self.id,
			kind: EventKind::Activated(Button::Left),
			position: None,
			bounds: None,
		});
	}

	fn secondary_activate(&mut self, _: i32, _: i32) {
		self.events.push(Event {
			id: self.id,
			kind: EventKind::Activated(Button::Middle),
			position: None,
			bounds: None,
		});
	}

	fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
		self.menu
			.as_ref()
			.map_or_else(Vec::new, |menu| project_menu(&menu.items()))
	}

	// A watcher that disappears after registration ends the service, so later
	// operations report `Unavailable` instead of updating an unseen item.
	fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
		false
	}
}

pub(super) struct NativeTray {
	handle: ksni::blocking::Handle<Model>,
}
impl NativeTray {
	pub(super) fn new(
		id: TrayId,
		name: &str,
		icon: RgbaIcon,
		options: &Options,
		menu: Option<&DesktopMenu>,
		events: Arc<Events>,
	) -> Result<Self, Error> {
		let model = Model {
			id,
			name: name.into(),
			icon: convert_icon(icon),
			title: options.title.clone().unwrap_or_default(),
			tooltip: options.tooltip.clone().unwrap_or_default(),
			visible: true,
			menu: menu.map(|menu| menu.native_menu().snapshot_handle()),
			events,
		};
		// `spawn` returns after the item registers with the
		// StatusNotifierWatcher, and fails when no watcher is
		// available.
		let handle = ksni::blocking::TrayMethods::spawn(model)
			.map_err(|error| Error::Native(error.to_string()))?;
		Ok(Self { handle })
	}

	fn update(&self, action: impl FnOnce(&mut Model)) -> Result<(), Error> {
		self.handle.update(action).ok_or(Error::Unavailable)
	}

	pub(super) fn set_icon(&self, icon: RgbaIcon, _template: bool) -> Result<(), Error> {
		self.update(|model| model.icon = convert_icon(icon))
	}

	pub(super) fn set_tooltip(&self, text: Option<&str>) -> Result<(), Error> {
		self.update(|model| model.tooltip = text.unwrap_or_default().into())
	}

	pub(super) fn set_title(&self, title: Option<&str>) -> Result<(), Error> {
		self.update(|model| model.title = title.unwrap_or_default().into())
	}

	pub(super) fn set_visible(&self, visible: bool) -> Result<(), Error> {
		self.update(|model| model.visible = visible)
	}

	pub(super) fn set_menu(&mut self, menu: Option<&DesktopMenu>) -> Result<(), Error> {
		self.update(|model| model.menu = menu.map(|menu| menu.native_menu().snapshot_handle()))
	}

	pub(super) fn refresh_menu(&self) -> Result<(), Error> {
		self.update(|_| {})
	}

	pub(super) fn bounds(&self) -> Result<Option<Rect>, Error> {
		if self.handle.is_closed() {
			Err(Error::Unavailable)
		} else {
			Ok(None)
		}
	}
}
impl Drop for NativeTray {
	fn drop(&mut self) {
		// Waits until the service has unregistered and its thread has finished.
		self.handle.shutdown().wait();
	}
}
fn convert_icon(icon: RgbaIcon) -> ksni::Icon {
	// StatusNotifierItem pixmaps are ARGB32 in network byte order: rotate each
	// RGBA pixel in place instead of copying the buffer.
	let mut data = icon.pixels;
	for pixel in data.chunks_exact_mut(4) {
		pixel.rotate_right(1);
	}
	ksni::Icon {
		width: icon.width as i32,
		height: icon.height as i32,
		data,
	}
}
fn label(text: &str) -> String {
	let mut output = String::with_capacity(text.len());
	let mut chars = text.chars().peekable();
	while let Some(character) = chars.next() {
		match character {
			'_' => output.push_str("__"),
			'&' if chars.peek() == Some(&'&') => {
				chars.next();
				output.push('&');
			}
			'&' => output.push('_'),
			value => output.push(value),
		}
	}
	output
}
fn project_menu(items: &[MenuItemKindSnapshot]) -> Vec<ksni::MenuItem<Model>> {
	project_nodes(items, &[])
}
fn project_nodes(
	items: &[MenuItemKindSnapshot],
	ancestors: &[SubmenuSnapshot],
) -> Vec<ksni::MenuItem<Model>> {
	items
		.iter()
		.map(|item| {
			match item {
				MenuItemKindSnapshot::MenuItem(item) => {
					let item = item.clone();
					let parents = ancestors.to_vec();
					ksni::menu::StandardItem {
						label: label(&item.text()),
						enabled: item.is_enabled(),
						activate: Box::new(move |_| {
							if item.is_enabled() && parents.iter().all(SubmenuSnapshot::is_enabled)
							{
								(item.activate)();
							}
						}),
						..Default::default()
					}
					.into()
				}
				MenuItemKindSnapshot::Check(item) => {
					let item = item.clone();
					let parents = ancestors.to_vec();
					ksni::menu::CheckmarkItem {
						label: label(&item.text()),
						enabled: item.is_enabled(),
						checked: item.is_checked(),
						activate: Box::new(move |_| {
							if item.is_enabled() && parents.iter().all(SubmenuSnapshot::is_enabled)
							{
								(item.activate)();
							}
						}),
						..Default::default()
					}
					.into()
				}
				MenuItemKindSnapshot::Submenu(item) => {
					let mut parents = ancestors.to_vec();
					parents.push(item.clone());
					ksni::menu::SubMenu {
						label: label(&item.text()),
						enabled: item.is_enabled(),
						submenu: project_nodes(&item.items(), &parents),
						..Default::default()
					}
					.into()
				}
				// The bounded public DesktopMenu only creates command, check,
				// submenu and separator items. Any other native structure
				// violates its escape-hatch contract.
				MenuItemKindSnapshot::Predefined(item) if item.is_separator() => {
					ksni::MenuItem::Separator
				}
				_ => {
					ksni::menu::StandardItem {
						label: "Unsupported menu item".into(),
						enabled: false,
						..Default::default()
					}
					.into()
				}
			}
		})
		.collect()
}
#[cfg(test)]
mod tests {
	use ksni::MenuItem;
	use ksni::Tray as _;
	use leptoskit_plugin_menu::Definition;
	use leptoskit_plugin_menu::Item;

	use super::*;

	fn model(menu: Option<MenuSnapshotHandle>) -> Model {
		let events = Arc::new(Events {
			queue: parking_lot::Mutex::new(super::super::Queue::default()),
			wake: Box::new(|| {}),
		});
		events
			.queue
			.lock()
			.ids
			.insert("leptoskit-tray-3".into(), TrayId(3));
		Model {
			id: TrayId(3),
			name: "leptoskit-tray-3".into(),
			icon: convert_icon(RgbaIcon::new(vec![9, 8, 7, 6], 1, 1).unwrap()),
			title: "Title".into(),
			tooltip: "Tooltip".into(),
			visible: true,
			menu,
			events,
		}
	}

	#[test]
	fn the_status_notifier_model_reports_its_state_and_activations() {
		let mut model = model(None);
		assert_eq!(model.id(), "leptoskit-tray-3");
		assert_eq!(model.title(), "Title");
		assert_eq!(model.tool_tip().title, "Tooltip");
		assert_eq!(model.icon_pixmap()[0].data, vec![6, 9, 8, 7]);
		assert!(matches!(model.status(), ksni::Status::Active));
		model.visible = false;
		assert!(matches!(model.status(), ksni::Status::Passive));
		assert!(model.menu().is_empty());
		assert!(!model.watcher_offline(ksni::OfflineReason::No));
		model.activate(0, 0);
		model.secondary_activate(0, 0);
		let kinds = model
			.events
			.queue
			.lock()
			.drain()
			.events
			.into_iter()
			.map(|event| event.kind)
			.collect::<Vec<_>>();
		assert_eq!(
			kinds,
			[
				EventKind::Activated(Button::Left),
				EventKind::Activated(Button::Middle)
			]
		);
	}

	#[test]
	fn menus_project_every_item_kind_and_respect_disabled_ancestors() {
		let menu = DesktopMenu::new(
			Definition::new(vec![
				Item::Command {
					id: "open".into(),
					label: "_Open".into(),
					enabled: true,
					accelerator: None,
				},
				Item::Separator,
				Item::Check {
					id: "pin".into(),
					label: "Pin".into(),
					enabled: false,
					checked: true,
					accelerator: None,
				},
				Item::Submenu {
					id: "more".into(),
					label: "More".into(),
					enabled: true,
					items: vec![Item::Command {
						id: "nested".into(),
						label: "Nested".into(),
						enabled: true,
						accelerator: None,
					}],
				},
			])
			.unwrap(),
		)
		.unwrap();
		let mut model = model(Some(menu.native_menu().snapshot_handle()));
		let items = model.menu();
		assert_eq!(items.len(), 4);
		let MenuItem::Standard(open) = &items[0] else {
			panic!("command projects to a standard item")
		};
		assert_eq!(open.label, "__Open");
		assert!(open.enabled);
		assert!(matches!(items[1], MenuItem::Separator));
		let MenuItem::Checkmark(pin) = &items[2] else {
			panic!("toggle projects to a checkmark")
		};
		assert!(pin.checked && !pin.enabled);
		let MenuItem::SubMenu(more) = &items[3] else {
			panic!("submenu projects to a submenu")
		};
		let MenuItem::Standard(nested) = &more.submenu[0] else {
			panic!("nested command projects to a standard item")
		};
		while muda::MenuEvent::receiver().try_recv().is_ok() {}
		(open.activate)(&mut model);
		assert!(muda::MenuEvent::receiver().try_recv().is_ok());
		// Disabling the parent after projection still suppresses its children.
		menu.set_enabled("more", false).unwrap();
		(nested.activate)(&mut model);
		assert!(muda::MenuEvent::receiver().try_recv().is_err());
	}

	#[test]
	fn toggles_activate_unsupported_items_are_inert_and_wake_panics_are_reported() {
		let menu = DesktopMenu::new(
			Definition::new(vec![Item::Check {
				id: "pin".into(),
				label: "Pin".into(),
				enabled: true,
				checked: false,
				accelerator: None,
			}])
			.unwrap(),
		)
		.unwrap();
		// The native escape hatch can add structures the bounded definition
		// never creates; they project as a disabled placeholder rather
		// than an action.
		menu.native_menu()
			.append(&muda::PredefinedMenuItem::copy(None))
			.unwrap();
		let mut model = model(Some(menu.native_menu().snapshot_handle()));
		let items = model.menu();
		let MenuItem::Checkmark(pin) = &items[0] else {
			panic!("toggle projects to a checkmark")
		};
		let MenuItem::Standard(unsupported) = &items[1] else {
			panic!("unsupported items project to a standard item")
		};
		assert_eq!(unsupported.label, "Unsupported menu item");
		assert!(!unsupported.enabled);
		while muda::MenuEvent::receiver().try_recv().is_ok() {}
		(pin.activate)(&mut model);
		assert!(muda::MenuEvent::receiver().try_recv().is_ok());

		let panicking = Arc::new(Events {
			queue: parking_lot::Mutex::new(super::super::Queue::default()),
			wake: Box::new(|| panic!("host wake failed")),
		});
		panicking
			.queue
			.lock()
			.ids
			.insert("leptoskit-tray-3".into(), TrayId(3));
		model.events = Arc::clone(&panicking);
		model.activate(0, 0);
		let batch = panicking.queue.lock().drain();
		assert!(batch.wake_failed);
		assert_eq!(batch.events.len(), 1);
	}

	#[test]
	fn pixels_are_argb_in_network_order_and_labels_preserve_literals() {
		let icon = convert_icon(RgbaIcon::new(vec![1, 2, 3, 4], 1, 1).unwrap());
		assert_eq!(icon.data, vec![4, 1, 2, 3]);
		assert_eq!(label("A_B && &Open"), "A__B & _Open");
	}
}
