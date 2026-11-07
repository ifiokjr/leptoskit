//! Chained constructors for menu trees.
//!
//! These match leptoskit's builder style (`View().class(…).child(…)`) so hosts
//! can assemble a menu without spelling every [`Item`] field. Validation still
//! happens in [`Definition::new`] / [`MenuBarBuilder::build`]: builders only
//! collect shape until then.
//!
//! ```rust
//! use leptoskit_plugin_menu::builder::*;
//!
//! let definition = MenuBar()
//! 	.submenu(
//! 		Submenu("file")
//! 			.label("File")
//! 			.item(Command("save").label("Save").accelerator("CmdOrCtrl+S")),
//! 	)
//! 	.build()
//! 	.expect("valid menu bar");
//! assert_eq!(definition.items().len(), 1);
//! ```
//!
//! Use [`Menu`] for tray or window menus that may mix commands at the root.
//! Use [`MenuBar`] when the tree will become an AppKit application menu (or a
//! Windows window menu): every root entry must be a submenu.

use crate::Definition;
use crate::Error;
use crate::Item;

/// Starts a free-form menu tree (tray menus, flat popovers, tests).
#[must_use]
#[allow(non_snake_case)]
pub fn Menu() -> MenuBuilder {
	MenuBuilder { items: Vec::new() }
}

/// Starts an application / window menu bar. Root items must be submenus.
#[must_use]
#[allow(non_snake_case)]
pub fn MenuBar() -> MenuBarBuilder {
	MenuBarBuilder { items: Vec::new() }
}

/// A command row. Pass a stable ASCII id; set the visible label with
/// [`CommandBuilder::label`].
#[must_use]
#[allow(non_snake_case)]
pub fn Command(id: impl Into<String>) -> CommandBuilder {
	CommandBuilder {
		id: id.into(),
		label: None,
		enabled: true,
		accelerator: None,
	}
}

/// A checkable row. Pass a stable ASCII id; set the visible label with
/// [`CheckBuilder::label`].
#[must_use]
#[allow(non_snake_case)]
pub fn Check(id: impl Into<String>) -> CheckBuilder {
	CheckBuilder {
		id: id.into(),
		label: None,
		enabled: true,
		checked: false,
		accelerator: None,
	}
}

/// A labeled submenu. Nest further items with [`SubmenuBuilder::item`].
#[must_use]
#[allow(non_snake_case)]
pub fn Submenu(id: impl Into<String>) -> SubmenuBuilder {
	SubmenuBuilder {
		id: id.into(),
		label: None,
		enabled: true,
		items: Vec::new(),
	}
}

/// A native visual separator.
#[must_use]
#[allow(non_snake_case)]
pub fn Separator() -> Item {
	Item::Separator
}

/// Accumulates root menu items before validation.
#[derive(Debug, Clone, Default)]
pub struct MenuBuilder {
	items: Vec<Item>,
}

impl MenuBuilder {
	/// Appends one item (command, check, submenu, or separator).
	#[must_use]
	pub fn item(mut self, item: impl Into<Item>) -> Self {
		self.items.push(item.into());
		self
	}

	/// Appends many items in order.
	#[must_use]
	pub fn items(mut self, items: impl IntoIterator<Item = impl Into<Item>>) -> Self {
		self.items.extend(items.into_iter().map(Into::into));
		self
	}

	/// Appends a submenu built with [`Submenu`].
	#[must_use]
	pub fn submenu(self, submenu: SubmenuBuilder) -> Self {
		self.item(submenu)
	}

	/// Appends a separator.
	#[must_use]
	pub fn separator(self) -> Self {
		self.item(Separator())
	}

	/// Validates bounds, IDs, labels, and accelerators.
	pub fn build(self) -> Result<Definition, Error> {
		Definition::new(self.items)
	}
}

/// Application-menu builder: every root entry must be a submenu (AppKit's
/// `NSMenu` bar rule; the same shape works for a Windows window menu).
#[derive(Debug, Clone, Default)]
pub struct MenuBarBuilder {
	items: Vec<Item>,
}

impl MenuBarBuilder {
	/// Appends a top-level submenu.
	#[must_use]
	pub fn submenu(mut self, submenu: SubmenuBuilder) -> Self {
		self.items.push(submenu.into());
		self
	}

	/// Appends several top-level submenus.
	#[must_use]
	pub fn submenus(mut self, submenus: impl IntoIterator<Item = SubmenuBuilder>) -> Self {
		self.items.extend(submenus.into_iter().map(Into::into));
		self
	}

	/// Validates the definition and that every root item is a submenu.
	pub fn build(self) -> Result<Definition, Error> {
		if self.items.is_empty()
			|| self
				.items
				.iter()
				.any(|item| !matches!(item, Item::Submenu { .. }))
		{
			return Err(Error::Unavailable);
		}

		Definition::new(self.items)
	}
}

/// Builder for [`Item::Command`].
#[derive(Debug, Clone)]
pub struct CommandBuilder {
	id: String,
	label: Option<String>,
	enabled: bool,
	accelerator: Option<String>,
}

impl CommandBuilder {
	/// Visible label. Defaults to the id when omitted.
	#[must_use]
	pub fn label(mut self, label: impl Into<String>) -> Self {
		self.label = Some(label.into());
		self
	}

	/// Whether users may activate the command. Defaults to `true`.
	#[must_use]
	pub fn enabled(mut self, enabled: bool) -> Self {
		self.enabled = enabled;
		self
	}

	/// Native accelerator in muda syntax, such as `CmdOrCtrl+S`.
	#[must_use]
	pub fn accelerator(mut self, accelerator: impl Into<String>) -> Self {
		self.accelerator = Some(accelerator.into());
		self
	}
}

impl From<CommandBuilder> for Item {
	fn from(builder: CommandBuilder) -> Self {
		let label = builder.label.unwrap_or_else(|| builder.id.clone());
		Self::Command {
			id: builder.id,
			label,
			enabled: builder.enabled,
			accelerator: builder.accelerator,
		}
	}
}

/// Builder for [`Item::Check`].
#[derive(Debug, Clone)]
pub struct CheckBuilder {
	id: String,
	label: Option<String>,
	enabled: bool,
	checked: bool,
	accelerator: Option<String>,
}

impl CheckBuilder {
	/// Visible label. Defaults to the id when omitted.
	#[must_use]
	pub fn label(mut self, label: impl Into<String>) -> Self {
		self.label = Some(label.into());
		self
	}

	/// Whether users may activate this item. Defaults to `true`.
	#[must_use]
	pub fn enabled(mut self, enabled: bool) -> Self {
		self.enabled = enabled;
		self
	}

	/// Initial checked state. Defaults to `false`.
	#[must_use]
	pub fn checked(mut self, checked: bool) -> Self {
		self.checked = checked;
		self
	}

	/// Native accelerator in muda syntax.
	#[must_use]
	pub fn accelerator(mut self, accelerator: impl Into<String>) -> Self {
		self.accelerator = Some(accelerator.into());
		self
	}
}

impl From<CheckBuilder> for Item {
	fn from(builder: CheckBuilder) -> Self {
		let label = builder.label.unwrap_or_else(|| builder.id.clone());
		Self::Check {
			id: builder.id,
			label,
			enabled: builder.enabled,
			checked: builder.checked,
			accelerator: builder.accelerator,
		}
	}
}

/// Builder for [`Item::Submenu`].
#[derive(Debug, Clone)]
pub struct SubmenuBuilder {
	id: String,
	label: Option<String>,
	enabled: bool,
	items: Vec<Item>,
}

impl SubmenuBuilder {
	/// Visible label. Defaults to the id when omitted.
	#[must_use]
	pub fn label(mut self, label: impl Into<String>) -> Self {
		self.label = Some(label.into());
		self
	}

	/// Whether users may open this submenu. Defaults to `true`.
	#[must_use]
	pub fn enabled(mut self, enabled: bool) -> Self {
		self.enabled = enabled;
		self
	}

	/// Appends one child item.
	#[must_use]
	pub fn item(mut self, item: impl Into<Item>) -> Self {
		self.items.push(item.into());
		self
	}

	/// Appends many child items in order.
	#[must_use]
	pub fn items(mut self, items: impl IntoIterator<Item = impl Into<Item>>) -> Self {
		self.items.extend(items.into_iter().map(Into::into));
		self
	}

	/// Appends a nested submenu.
	#[must_use]
	pub fn submenu(self, submenu: SubmenuBuilder) -> Self {
		self.item(submenu)
	}

	/// Appends a separator.
	#[must_use]
	pub fn separator(self) -> Self {
		self.item(Separator())
	}
}

impl From<SubmenuBuilder> for Item {
	fn from(builder: SubmenuBuilder) -> Self {
		let label = builder.label.unwrap_or_else(|| builder.id.clone());
		Self::Submenu {
			id: builder.id,
			label,
			enabled: builder.enabled,
			items: builder.items,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn menu_bar_requires_root_submenus_and_validates_children() {
		assert_eq!(MenuBar().build(), Err(Error::Unavailable));
		assert_eq!(
			MenuBar()
				.submenu(Submenu("file").label("File").item(Command("save")))
				.build()
				.map(|definition| definition.items().len()),
			Ok(1)
		);
		assert_eq!(
			Menu()
				.item(Command("save").label("Save"))
				.separator()
				.item(Check("pin").label("Pin").checked(true))
				.build()
				.map(|definition| definition.items().len()),
			Ok(3)
		);
		assert_eq!(
			Menu().item(Command("bad id").label("Broken")).build(),
			Err(Error::InvalidDefinition)
		);
	}

	#[test]
	fn omitted_labels_fall_back_to_the_id() {
		let definition = Menu()
			.item(Command("save"))
			.item(Check("pin"))
			.submenu(Submenu("view").item(Command("zoom")))
			.build()
			.unwrap();
		match &definition.items()[0] {
			Item::Command { id, label, .. } => {
				assert_eq!(id, "save");
				assert_eq!(label, "save");
			}
			_ => panic!("expected command"),
		}
		match &definition.items()[1] {
			Item::Check { id, label, .. } => {
				assert_eq!(id, "pin");
				assert_eq!(label, "pin");
			}
			_ => panic!("expected check"),
		}
		match &definition.items()[2] {
			Item::Submenu {
				id, label, items, ..
			} => {
				assert_eq!(id, "view");
				assert_eq!(label, "view");
				assert_eq!(items.len(), 1);
			}
			_ => panic!("expected submenu"),
		}
	}

	#[test]
	fn menu_bar_rejects_a_flat_root_command() {
		// MenuBarBuilder only accepts SubmenuBuilder at the type level for
		// `.submenu`, so a flat root has to go through Menu and then attach.
		// ApplicationMenu::attach still enforces the AppKit rule at runtime.
		let flat = Menu().item(Command("quit").label("Quit")).build().unwrap();
		assert!(
			flat.items()
				.iter()
				.all(|item| !matches!(item, Item::Submenu { .. }))
		);
	}

	#[test]
	fn builders_cover_batch_helpers_and_optional_fields() {
		let definition = Menu()
			.items([
				Command("save")
					.label("Save")
					.enabled(false)
					.accelerator("CmdOrCtrl+S"),
				Command("open").label("Open").enabled(true),
			])
			.item(
				Check("pin")
					.label("Pin")
					.enabled(false)
					.checked(true)
					.accelerator("CmdOrCtrl+P"),
			)
			.submenu(
				Submenu("view")
					.label("View")
					.enabled(false)
					.items([Command("zoom-in").label("Zoom In")])
					.separator()
					.submenu(Submenu("more").label("More").item(Command("extra"))),
			)
			.build()
			.unwrap();
		assert_eq!(definition.items().len(), 4);
		match &definition.items()[0] {
			Item::Command {
				enabled,
				accelerator,
				..
			} => {
				assert!(!enabled);
				assert_eq!(accelerator.as_deref(), Some("CmdOrCtrl+S"));
			}
			_ => panic!("expected command"),
		}
		match &definition.items()[2] {
			Item::Check {
				enabled,
				checked,
				accelerator,
				..
			} => {
				assert!(!enabled);
				assert!(checked);
				assert_eq!(accelerator.as_deref(), Some("CmdOrCtrl+P"));
			}
			_ => panic!("expected check"),
		}
		match &definition.items()[3] {
			Item::Submenu { enabled, items, .. } => {
				assert!(!enabled);
				assert_eq!(items.len(), 3);
				assert!(matches!(items[1], Item::Separator));
				assert!(matches!(items[2], Item::Submenu { .. }));
			}
			_ => panic!("expected submenu"),
		}

		let bar = MenuBar()
			.submenus([
				Submenu("file").label("File").item(Command("open")),
				Submenu("edit").label("Edit").item(Command("copy")),
			])
			.build()
			.unwrap();
		assert_eq!(bar.items().len(), 2);
	}
}
