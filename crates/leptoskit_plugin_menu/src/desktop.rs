use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::SyncSender;
use std::sync::mpsc::sync_channel;

/// Native menu types shared with the host and tray integration.
pub use muda;
use muda::IsMenuItem;
use muda::accelerator::Accelerator;

use crate::Definition;
use crate::Error;
use crate::Item;
use crate::MAX_EVENTS;
use crate::valid_label;

static NEXT_MENU: AtomicU64 = AtomicU64::new(1);

/// A command activation mapped to an application's local ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
	/// The identifier from the validated definition.
	pub id: String,
	/// Current checked state for a check item; absent for a command.
	/// When multiple clicks queue, this is the state at dispatch time.
	pub checked: Option<bool>,
}

/// Bounded, nonblocking callback sender. Clone into the host's one muda
/// callback.
#[derive(Clone)]
pub struct EventSender {
	sender: SyncSender<muda::MenuEvent>,
	dropped: Arc<AtomicUsize>,
}

impl EventSender {
	/// Queues an activation without blocking the native UI or KSNI thread.
	/// Rejects oversized foreign IDs; overflow drops the newest event.
	pub fn forward(&self, event: muda::MenuEvent) {
		if event.id.0.len() > 128 || self.sender.try_send(event).is_err() {
			let _ = self
				.dropped
				.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
					Some(value.saturating_add(1))
				});
		}
	}
}

/// Host-owned receiver; dropping it makes future callbacks discard events.
///
/// Install `move |event| sender.forward(event)` through muda's process-wide
/// `MenuEvent::set_event_handler` **before any native activation**, exactly
/// once. Muda cannot report whether another component already installed its
/// callback; hosts already using muda must forward from their existing callback
/// instead. This crate deliberately never overwrites that process-global
/// integration.
pub struct EventChannel {
	receiver: Receiver<muda::MenuEvent>,
	sender: EventSender,
}

impl EventChannel {
	/// Creates a bounded event channel (1..=1024 entries).
	pub fn new(capacity: usize) -> Result<Self, Error> {
		if capacity == 0 || capacity > MAX_EVENTS {
			return Err(Error::InvalidDefinition);
		}
		let (sender, receiver) = sync_channel(capacity);
		Ok(Self {
			receiver,
			sender: EventSender {
				sender,
				dropped: Arc::new(AtomicUsize::new(0)),
			},
		})
	}

	/// Clone this sender into the host callback. Poll on the UI thread or wake
	/// its event loop with a separate coalesced signal after forwarding.
	pub fn sender(&self) -> EventSender {
		self.sender.clone()
	}

	/// Takes the next activation, without waiting.
	pub fn try_recv(&self) -> Option<muda::MenuEvent> {
		self.receiver.try_recv().ok()
	}

	/// Takes and resets the count of dropped activations.
	pub fn take_dropped(&self) -> usize {
		self.sender.dropped.swap(0, Ordering::Relaxed)
	}
}

struct Entry {
	local_id: String,
	item: muda::MenuItemKind,
	ancestors: Vec<muda::Submenu>,
}

/// Owned native menu, confined to its creating UI thread by `Rc` internals.
///
/// Linux uses muda's GTK-free snapshot backend for KSNI trays. It does not
/// display a standalone menu. Retain this wrapper as long as any tray borrows
/// it.
pub struct DesktopMenu {
	menu: muda::Menu,
	entries: HashMap<String, Entry>,
	local: HashMap<String, String>,
	pub(crate) definition: Definition,
}

impl DesktopMenu {
	/// Creates native items after validating every accelerator, before
	/// attaching any menu to a window or application. macOS requires the
	/// main thread.
	pub fn new(definition: Definition) -> Result<Self, Error> {
		#[cfg(target_os = "macos")]
		if objc2::MainThreadMarker::new().is_none() {
			return Err(Error::WrongThread);
		}
		validate_accelerators(definition.items())?;
		let serial = NEXT_MENU
			.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
				value.checked_add(1)
			})
			.map_err(|_| Error::Busy)?;
		let mut result = Self {
			menu: muda::Menu::new(),
			entries: HashMap::new(),
			local: HashMap::new(),
			definition,
		};
		for item in result.definition.items().to_vec() {
			let native = result.build(&item, serial, &[])?;
			result
				.menu
				.append(native.as_ref())
				.map_err(|_| Error::Native)?;
		}
		Ok(result)
	}

	/// Native integration escape hatch. The host must preserve this wrapper's
	/// bounded tree and IDs: do not mutate structure through this reference.
	/// Retain an `Rc<DesktopMenu>` in a tray and clone only the native menu
	/// handle.
	pub fn native_menu(&self) -> &muda::Menu {
		&self.menu
	}

	/// Maps an owned, still-enabled command; ignores foreign IDs, separators,
	/// submenus, and commands disabled since the native event was queued.
	pub fn handle_event(&self, event: &muda::MenuEvent) -> Option<Event> {
		let entry = self.entries.get(&event.id.0)?;
		if entry.ancestors.iter().any(|item| !item.is_enabled()) {
			return None;
		}
		let checked = match &entry.item {
			muda::MenuItemKind::MenuItem(item) if item.is_enabled() => None,
			muda::MenuItemKind::Check(item) if item.is_enabled() => Some(item.is_checked()),
			_ => return None,
		};
		Some(Event {
			id: entry.local_id.clone(),
			checked,
		})
	}

	/// Updates a command, toggle, or submenu without replacing native objects.
	pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), Error> {
		match &self.entry(id)?.item {
			muda::MenuItemKind::MenuItem(item) => item.set_enabled(enabled),
			muda::MenuItemKind::Check(item) => item.set_enabled(enabled),
			muda::MenuItemKind::Submenu(item) => item.set_enabled(enabled),
			_ => return Err(Error::InvalidItem),
		}
		Ok(())
	}

	/// Updates a toggle. Commands and submenus return `InvalidItem`.
	pub fn set_checked(&self, id: &str, checked: bool) -> Result<(), Error> {
		let muda::MenuItemKind::Check(item) = &self.entry(id)?.item else {
			return Err(Error::InvalidItem);
		};
		item.set_checked(checked);
		Ok(())
	}

	/// Reads the native toggle state, including changes from user activation.
	pub fn is_checked(&self, id: &str) -> Result<bool, Error> {
		let muda::MenuItemKind::Check(item) = &self.entry(id)?.item else {
			return Err(Error::InvalidItem);
		};
		Ok(item.is_checked())
	}

	/// Updates a visible label with the same size/control-character validation.
	pub fn set_label(&self, id: &str, label: &str) -> Result<(), Error> {
		if !valid_label(label) {
			return Err(Error::InvalidDefinition);
		}
		match &self.entry(id)?.item {
			muda::MenuItemKind::MenuItem(item) => item.set_text(label),
			muda::MenuItemKind::Check(item) => item.set_text(label),
			muda::MenuItemKind::Submenu(item) => item.set_text(label),
			_ => return Err(Error::InvalidItem),
		}
		Ok(())
	}

	fn entry(&self, id: &str) -> Result<&Entry, Error> {
		self.local
			.get(id)
			.and_then(|native| self.entries.get(native))
			.ok_or(Error::InvalidItem)
	}

	fn build(
		&mut self,
		definition: &Item,
		serial: u64,
		ancestors: &[muda::Submenu],
	) -> Result<Box<dyn IsMenuItem>, Error> {
		let (id, item): (&str, Box<dyn IsMenuItem>) = match definition {
			Item::Command {
				id,
				label,
				enabled,
				accelerator,
			} => {
				(
					id,
					Box::new(muda::MenuItem::with_id(
						format!("leptoskit.menu.{serial}.{id}"),
						label,
						*enabled,
						parse(accelerator.as_deref())?,
					)),
				)
			}
			Item::Check {
				id,
				label,
				enabled,
				checked,
				accelerator,
			} => {
				(
					id,
					Box::new(muda::CheckMenuItem::with_id(
						format!("leptoskit.menu.{serial}.{id}"),
						label,
						*enabled,
						*checked,
						parse(accelerator.as_deref())?,
					)),
				)
			}
			Item::Submenu {
				id,
				label,
				enabled,
				items,
			} => {
				let submenu = muda::Submenu::with_id(
					format!("leptoskit.menu.{serial}.{id}"),
					label,
					*enabled,
				);
				let mut children_ancestors = ancestors.to_vec();
				children_ancestors.push(submenu.clone());
				for child in items {
					submenu
						.append(self.build(child, serial, &children_ancestors)?.as_ref())
						.map_err(|_| Error::Native)?;
				}
				(id, Box::new(submenu))
			}
			Item::Separator => return Ok(Box::new(muda::PredefinedMenuItem::separator())),
		};
		let native_id = item.id().0.clone();
		self.local.insert(id.to_owned(), native_id.clone());
		self.entries.insert(
			native_id,
			Entry {
				local_id: id.to_owned(),
				item: item.kind(),
				ancestors: ancestors.to_vec(),
			},
		);
		Ok(item)
	}
}

fn parse(accelerator: Option<&str>) -> Result<Option<Accelerator>, Error> {
	accelerator
		.map(str::parse)
		.transpose()
		.map_err(|_| Error::InvalidAccelerator)
}

fn validate_accelerators(items: &[Item]) -> Result<(), Error> {
	for item in items {
		match item {
			Item::Command { accelerator, .. } | Item::Check { accelerator, .. } => {
				parse(accelerator.as_deref())?;
			}
			Item::Submenu { items, .. } => validate_accelerators(items)?,
			Item::Separator => {}
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn event_capacity_and_foreign_id_length_are_bounded() {
		assert!(EventChannel::new(0).is_err());
		assert!(EventChannel::new(MAX_EVENTS + 1).is_err());
		let channel = EventChannel::new(1).unwrap();
		let sender = channel.sender();
		sender.forward(muda::MenuEvent { id: "first".into() });
		sender.forward(muda::MenuEvent {
			id: "overflow".into(),
		});
		assert_eq!(channel.take_dropped(), 1);
		assert_eq!(channel.try_recv().unwrap().id.0, "first");
		sender.forward(muda::MenuEvent {
			id: "x".repeat(129).into(),
		});
		assert!(channel.try_recv().is_none());
		assert_eq!(channel.take_dropped(), 1);
		assert_eq!(channel.take_dropped(), 0);
	}

	// Linux uses muda's GTK-free snapshot backend, so the wrapper runs on the
	// test thread. macOS menus need the main thread (tests/macos.rs) and
	// Windows menus are exercised against a real window (tests/windows.rs).
	#[cfg(target_os = "linux")]
	#[test]
	fn commands_toggles_and_submenus_map_events_and_accept_updates() {
		let menu = DesktopMenu::new(
			Definition::new(vec![
				Item::Command {
					id: "save".into(),
					label: "Save".into(),
					enabled: true,
					accelerator: Some("CmdOrCtrl+S".into()),
				},
				Item::Separator,
				Item::Submenu {
					id: "view".into(),
					label: "View".into(),
					enabled: true,
					items: vec![Item::Check {
						id: "pin".into(),
						label: "Pin".into(),
						enabled: true,
						checked: false,
						accelerator: None,
					}],
				},
			])
			.unwrap(),
		)
		.unwrap();
		assert_eq!(menu.native_menu().items().len(), 3);
		let event = |id: &str| {
			muda::MenuEvent {
				id: menu.local[id].clone().into(),
			}
		};

		assert_eq!(
			menu.handle_event(&event("save")),
			Some(Event {
				id: "save".into(),
				checked: None,
			})
		);
		menu.set_checked("pin", true).unwrap();
		assert!(menu.is_checked("pin").unwrap());
		assert_eq!(
			menu.handle_event(&event("pin")),
			Some(Event {
				id: "pin".into(),
				checked: Some(true),
			})
		);
		assert_eq!(
			menu.handle_event(&muda::MenuEvent {
				id: "foreign".into()
			}),
			None
		);
		// Submenus are not commands, and a disabled ancestor or item suppresses
		// activations that were already queued.
		assert_eq!(menu.handle_event(&event("view")), None);
		menu.set_enabled("view", false).unwrap();
		assert_eq!(menu.handle_event(&event("pin")), None);
		menu.set_enabled("view", true).unwrap();
		menu.set_enabled("pin", false).unwrap();
		assert_eq!(menu.handle_event(&event("pin")), None);
		menu.set_enabled("save", false).unwrap();
		assert_eq!(menu.handle_event(&event("save")), None);

		for id in ["save", "pin", "view"] {
			menu.set_label(id, "Renamed").unwrap();
		}
		assert_eq!(
			menu.set_label("save", "bad\0label"),
			Err(Error::InvalidDefinition)
		);
		assert_eq!(menu.set_label("missing", "Label"), Err(Error::InvalidItem));
		assert_eq!(menu.set_enabled("missing", true), Err(Error::InvalidItem));
		assert_eq!(menu.set_checked("save", true), Err(Error::InvalidItem));
		assert_eq!(menu.is_checked("view"), Err(Error::InvalidItem));
	}

	#[cfg(target_os = "linux")]
	#[test]
	fn accelerators_that_muda_cannot_parse_are_rejected_before_any_item() {
		assert!(matches!(
			DesktopMenu::new(
				Definition::new(vec![Item::Command {
					id: "save".into(),
					label: "Save".into(),
					enabled: true,
					accelerator: Some("NotAKey+Q".into()),
				}])
				.unwrap(),
			),
			Err(Error::InvalidAccelerator)
		));
	}
}
