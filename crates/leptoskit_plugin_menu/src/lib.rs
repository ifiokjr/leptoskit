//! Native desktop menus with explicit host ownership.
//!
//! Definitions are bounded before native allocation. The desktop host owns the
//! UI thread, event callback, and window; this crate never discovers a window
//! or registers a guest bridge automatically. Linux supports the snapshot model
//! for KSNI trays, but not standalone menu bars. Mobile and browser
//! presentation are unavailable. See the crate README for event-loop
//! integration.

use std::collections::HashSet;

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
mod desktop;
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub use desktop::*;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

/// Maximum number of menu nodes, including separators and submenus.
pub const MAX_ITEMS: usize = 256;
/// Maximum submenu nesting depth.
pub const MAX_DEPTH: usize = 8;
/// Maximum queued native activations.
pub const MAX_EVENTS: usize = 1024;

/// Validation, ownership, or native menu failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
	/// Invalid, duplicate, oversized, or excessively nested definition.
	#[error("invalid or oversized menu definition")]
	InvalidDefinition,
	/// The accelerator cannot be parsed on this platform.
	#[error("invalid menu accelerator")]
	InvalidAccelerator,
	/// The identifier does not exist or names the wrong item kind.
	#[error("unknown menu item or incompatible item kind")]
	InvalidItem,
	/// This operation needs the native UI thread.
	#[error("menu operation must run on the native UI thread")]
	WrongThread,
	/// The application or window already has a managed menu lease.
	#[error("menu owner already has an active lease")]
	Busy,
	/// The operation is unavailable on this platform or for this menu shape.
	#[error("menu presentation is unavailable")]
	Unavailable,
	/// A native API failed.
	#[error("native menu operation failed")]
	Native,
}

/// A bounded, host-defined menu tree. Validate with [`Definition::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
	/// A command. Accelerators use muda syntax, e.g. `CmdOrCtrl+S`.
	Command {
		/// Unique local identifier; ASCII letters, digits, underscore, dot, or
		/// dash.
		id: String,
		/// Visible label. Ampersands follow the native platform's mnemonic
		/// rules.
		label: String,
		/// Whether users may activate the command.
		enabled: bool,
		/// Optional native keyboard accelerator.
		accelerator: Option<String>,
	},
	/// A toggle, whose checked state is updated by native activation.
	Check {
		/// Unique local identifier.
		id: String,
		/// Visible label.
		label: String,
		/// Whether users may activate this item.
		enabled: bool,
		/// Initial checked state.
		checked: bool,
		/// Optional native keyboard accelerator.
		accelerator: Option<String>,
	},
	/// A labeled group containing at least one child.
	Submenu {
		/// Unique local identifier.
		id: String,
		/// Visible label.
		label: String,
		/// Whether users may open this submenu.
		enabled: bool,
		/// Child commands, toggles, separators, or submenus.
		items: Vec<Self>,
	},
	/// A native visual separator.
	Separator,
}

/// Validated immutable menu definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Definition {
	items: Vec<Item>,
}

impl Definition {
	/// Validates total nodes, nesting, unique IDs, labels, and accelerator
	/// lengths.
	pub fn new(items: Vec<Item>) -> Result<Self, Error> {
		let mut ids = HashSet::new();
		let mut count = 0;
		validate(&items, 0, &mut ids, &mut count)?;
		Ok(Self { items })
	}

	/// The validated nodes.
	pub fn items(&self) -> &[Item] {
		&self.items
	}
}

fn validate<'a>(
	items: &'a [Item],
	depth: usize,
	ids: &mut HashSet<&'a str>,
	count: &mut usize,
) -> Result<(), Error> {
	if items.is_empty() || depth > MAX_DEPTH || items.len() > MAX_ITEMS {
		return Err(Error::InvalidDefinition);
	}
	for item in items {
		*count += 1;
		if *count > MAX_ITEMS {
			return Err(Error::InvalidDefinition);
		}
		let (id, label, accelerator) = match item {
			Item::Command {
				id,
				label,
				accelerator,
				..
			}
			| Item::Check {
				id,
				label,
				accelerator,
				..
			} => (id, label, accelerator.as_deref()),
			Item::Submenu {
				id, label, items, ..
			} => {
				validate(items, depth + 1, ids, count)?;
				(id, label, None)
			}
			Item::Separator => continue,
		};
		if id.is_empty()
			|| id.len() > 64
			|| !id
				.bytes()
				.all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
			|| !ids.insert(id)
			|| !valid_label(label)
		{
			return Err(Error::InvalidDefinition);
		}
		if let Some(accelerator) = accelerator
			&& (accelerator.is_empty()
				|| accelerator.len() > 64
				|| !accelerator.is_ascii()
				|| accelerator.chars().any(char::is_control))
		{
			return Err(Error::InvalidAccelerator);
		}
	}
	Ok(())
}

fn valid_label(label: &str) -> bool {
	!label.is_empty() && label.len() <= 256 && !label.chars().any(char::is_control)
}

/// Native presentation support. Linux's model can additionally feed a KSNI
/// tray.
pub const fn supports_menu_bar() -> bool {
	cfg!(any(target_os = "macos", target_os = "windows"))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn command(id: &str) -> Item {
		Item::Command {
			id: id.into(),
			label: "Save".into(),
			enabled: true,
			accelerator: None,
		}
	}

	#[test]
	fn rejects_duplicate_ids_across_submenus_and_invalid_labels() {
		assert_eq!(
			Definition::new(vec![
				command("save"),
				Item::Submenu {
					id: "file".into(),
					label: "File".into(),
					enabled: true,
					items: vec![command("save")],
				}
			]),
			Err(Error::InvalidDefinition)
		);
		let mut item = command("valid");
		if let Item::Command { label, .. } = &mut item {
			*label = "bad\0label".into();
		}
		assert_eq!(Definition::new(vec![item]), Err(Error::InvalidDefinition));
		assert_eq!(
			Definition::new(vec![command("bad id")]),
			Err(Error::InvalidDefinition)
		);
	}

	#[test]
	fn bounds_all_nodes_and_depth() {
		assert!(Definition::new(vec![Item::Separator; MAX_ITEMS]).is_ok());
		assert_eq!(
			Definition::new(vec![Item::Separator; MAX_ITEMS + 1]),
			Err(Error::InvalidDefinition)
		);
		let mut item = command("leaf");
		for depth in 0..=MAX_DEPTH {
			item = Item::Submenu {
				id: format!("level{depth}"),
				label: "Group".into(),
				enabled: true,
				items: vec![item],
			};
		}
		assert_eq!(Definition::new(vec![item]), Err(Error::InvalidDefinition));
		assert_eq!(Definition::new(vec![]), Err(Error::InvalidDefinition));
	}
}
