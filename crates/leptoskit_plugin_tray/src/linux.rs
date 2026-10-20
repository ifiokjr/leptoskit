use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::mpsc::SyncSender;
use std::sync::mpsc::sync_channel;
use std::thread::JoinHandle;
use std::time::Duration;

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
	ready: SyncSender<Result<(), String>>,
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

	fn watcher_online(&self) {
		let _ = self.ready.try_send(Ok(()));
	}

	fn watcher_offine(&self) -> bool {
		let _ = self
			.ready
			.try_send(Err("no StatusNotifierWatcher is available".into()));
		false
	}
}

pub(super) struct NativeTray {
	handle: ksni::Handle<Model>,
	worker: Option<JoinHandle<()>>,
	running: Arc<AtomicBool>,
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
		let (ready, receiver) = sync_channel(1);
		let model = Model {
			id,
			name: name.into(),
			icon: convert_icon(icon),
			title: options.title.clone().unwrap_or_default(),
			tooltip: options.tooltip.clone().unwrap_or_default(),
			visible: true,
			menu: menu.map(|menu| menu.native_menu().snapshot_handle()),
			events,
			ready: ready.clone(),
		};
		let service = ksni::TrayService::new(model);
		let handle = service.handle();
		let running = Arc::new(AtomicBool::new(true));
		let worker_running = Arc::clone(&running);
		// Own the thread instead of ksni's detached/panicking spawn helper.
		let worker = std::thread::Builder::new()
			.name("leptoskit-tray".into())
			.spawn(move || {
				let result =
					std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| service.run()));
				match result {
					Ok(Err(error)) => {
						let _ = ready.try_send(Err(error.to_string()));
					}
					Err(_) => {
						let _ = ready.try_send(Err("native tray worker panicked".into()));
					}
					Ok(Ok(())) => {}
				}
				worker_running.store(false, Ordering::Release);
			})
			.map_err(|error| Error::Native(error.to_string()))?;
		let native = Self {
			handle,
			worker: Some(worker),
			running,
		};
		match receiver.recv_timeout(Duration::from_secs(3)) {
			Ok(Ok(())) if native.running.load(Ordering::Acquire) => Ok(native),
			Ok(Err(error)) => Err(Error::Native(error)),
			_ => {
				Err(Error::Native(
					"tray service did not finish registration".into(),
				))
			}
		}
		// Failed initialization drops native, shuts down and joins its worker.
	}

	fn update(&self, action: impl FnOnce(&mut Model)) -> Result<(), Error> {
		if !self.running.load(Ordering::Acquire) {
			return Err(Error::Unavailable);
		}
		std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.handle.update(action)))
			.map_err(|_| Error::Native("native tray state is no longer usable".into()))
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
		if self.running.load(Ordering::Acquire) {
			Ok(None)
		} else {
			Err(Error::Unavailable)
		}
	}
}
impl Drop for NativeTray {
	fn drop(&mut self) {
		self.handle.shutdown();
		if let Some(worker) = self.worker.take() {
			// Worker failure cannot be returned from Drop. The running flag
			// already makes future operations fail; joining guarantees exit.
			let _ = worker.join();
		}
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
				// The bounded public DesktopMenu only creates command/check/submenu/
				// separator items. Other native structures violate its escape-hatch contract.
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
	use super::*;
	#[test]
	fn pixels_are_argb_in_network_order_and_labels_preserve_literals() {
		let icon = convert_icon(RgbaIcon::new(vec![1, 2, 3, 4], 1, 1).unwrap());
		assert_eq!(icon.data, vec![4, 1, 2, 3]);
		assert_eq!(label("A_B && &Open"), "A__B & _Open");
	}
}
