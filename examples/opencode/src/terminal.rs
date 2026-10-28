//! The terminal host: the interface's state behind a [`TuiApplication`],
//! with opencode's keyboard shortcuts.

use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use leptoskit_native::NativeEvent;
use leptoskit_native::NativeSurface;
use leptoskit_tui::Key;
use leptoskit_tui::KeyCode;
use leptoskit_tui::TuiApplication;
use leptoskit_tui::TuiBackend;
use leptoskit_tui::TuiOptions;
use leptoskit_yoga::NodeId;

use crate::Action;
use crate::Dispatch;
use crate::Opencode;
use crate::ShortcutKey;
use crate::shortcut;

/// The interface in a terminal: a surface over the terminal backend, the
/// state, and the actions event handlers queue for it.
pub struct TerminalApp {
	surface: NativeSurface<TuiBackend>,
	state: Opencode,
	queue: Arc<Mutex<Vec<Action>>>,
	dispatch: Dispatch,
}

impl TerminalApp {
	/// An app on the home screen, showing `directory` in its footer.
	#[must_use]
	pub fn new(directory: impl Into<String>) -> Self {
		let queue = Arc::new(Mutex::new(Vec::new()));
		let dispatch: Dispatch = {
			let queue = Arc::clone(&queue);
			Arc::new(move |action| {
				queue
					.lock()
					.unwrap_or_else(PoisonError::into_inner)
					.push(action);
			})
		};

		Self {
			surface: NativeSurface::new(TuiBackend::new()),
			state: Opencode::new(directory),
			queue,
			dispatch,
		}
	}

	/// The interface's state.
	#[must_use]
	pub const fn state(&self) -> &Opencode {
		&self.state
	}

	/// Applies an action and redraws, as a key press or event would.
	pub fn apply(&mut self, action: Action) {
		self.state.apply(action);
		self.render();
	}

	/// The surface the interface commits into.
	pub fn surface(&mut self) -> &mut NativeSurface<TuiBackend> {
		&mut self.surface
	}

	/// Commits the current state and moves focus to where it belongs: the
	/// prompt, or the palette's search while the palette is open.
	fn render(&mut self) {
		self.surface.set_root(&self.state.render(&self.dispatch));

		let target = self.state.focus_target();
		let backend = self.surface.backend_mut();
		let focused = backend
			.focused()
			.and_then(|node| backend.props(node))
			.and_then(|props| props.id.as_deref());

		if focused != Some(target) {
			for event in backend.focus_id(target) {
				self.surface.dispatch_event(event.node, &event.event);
			}
		}
	}

	fn drain(&mut self) {
		let actions =
			std::mem::take(&mut *self.queue.lock().unwrap_or_else(PoisonError::into_inner));
		for action in actions {
			self.state.apply(action);
		}
	}
}

impl TuiApplication for TerminalApp {
	fn backend(&mut self) -> &mut TuiBackend {
		self.surface.backend_mut()
	}

	fn dispatch(&mut self, node: NodeId, event: &NativeEvent) {
		self.surface.dispatch_event(node, event);
		self.drain();
		self.render();
	}

	fn resize(&mut self, cols: u16, rows: u16) {
		self.state.size = (cols, rows);
		let (width, height) = leptoskit_tui::viewport(cols, rows);
		self.surface.set_viewport(width, height);
		self.render();
	}

	fn mount(&mut self) {
		self.render();
	}

	fn shortcut(&mut self, key: Key) -> bool {
		let Some(action) = shortcut_key(key).and_then(|code| shortcut(&self.state, code, key.ctrl))
		else {
			return false;
		};
		self.apply(action);
		true
	}
}

/// The interface's [`ShortcutKey`] for a terminal key, if it has one.
#[must_use]
pub fn shortcut_key(key: Key) -> Option<ShortcutKey> {
	Some(match key.code {
		KeyCode::Char(symbol) => ShortcutKey::Char(symbol),
		KeyCode::Escape => ShortcutKey::Escape,
		KeyCode::Up => ShortcutKey::Up,
		KeyCode::Down => ShortcutKey::Down,
		KeyCode::Enter => ShortcutKey::Enter,
		KeyCode::BackTab => ShortcutKey::BackTab,
		_ => return None,
	})
}

/// The host options: the alternate screen, the mouse, and a window title.
#[must_use]
pub fn options() -> TuiOptions {
	TuiOptions {
		title: "opencode".to_owned(),
		..TuiOptions::default()
	}
}

/// The working directory as opencode labels it: the home directory as
/// `~`, then the git branch.
#[must_use]
pub fn working_directory() -> String {
	let Ok(directory) = std::env::current_dir() else {
		return "~".to_owned();
	};
	let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
	directory_label(&directory, home.as_deref())
}

/// [`working_directory`] for an explicit directory and home.
#[must_use]
pub fn directory_label(directory: &Path, home: Option<&Path>) -> String {
	let mut label = match home.and_then(|home| directory.strip_prefix(home).ok()) {
		Some(relative) if relative.as_os_str().is_empty() => "~".to_owned(),
		Some(relative) => format!("~/{}", relative.display()),
		None => directory.display().to_string(),
	};

	let head = directory
		.ancestors()
		.find_map(|ancestor| std::fs::read_to_string(ancestor.join(".git/HEAD")).ok());
	if let Some(branch) = head
		.as_deref()
		.and_then(|head| head.trim().strip_prefix("ref: refs/heads/"))
	{
		label.push(':');
		label.push_str(branch);
	}

	label
}
