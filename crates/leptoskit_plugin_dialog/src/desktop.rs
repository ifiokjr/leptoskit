//! Native macOS, Windows, and Linux dialogs backed by `rfd`.
//!
//! Linux packages require a supported XDG desktop portal and Zenity. The
//! backend returns `None` for dismissal **and some native failures**, which it
//! does not distinguish. A missing selection must never authorize an action.
//! Saving selects a destination; it does not write or reserve the file.

use std::path::Path;
use std::path::PathBuf;

pub use raw_window_handle::HasDisplayHandle;
pub use raw_window_handle::HasWindowHandle;
pub use rfd::MessageButtons;
pub use rfd::MessageDialogResult;
pub use rfd::MessageLevel;

use crate::Error;
use crate::FileFilter;
use crate::valid_text;

/// Options shared by native file and folder dialogs.
#[derive(Debug, Clone, Default)]
pub struct FileOptions {
	/// Dialog title. An empty title uses the backend's default.
	pub title: String,
	/// Initial directory. It must be absolute; the OS decides whether it
	/// exists.
	pub directory: Option<PathBuf>,
	/// Suggested filename for save dialogs; never a path.
	pub file_name: Option<String>,
	/// Filename extension filters, ignored by folder selection.
	pub filters: Vec<FileFilter>,
}

impl FileOptions {
	/// Checks user-controlled options before invoking native UI.
	pub fn validate(&self) -> Result<(), Error> {
		if !valid_text(&self.title) {
			return Err(Error::InvalidOption);
		}

		if let Some(directory) = &self.directory
			&& (!directory.is_absolute() || path_contains_nul(directory))
		{
			return Err(Error::InvalidOption);
		}

		if let Some(file_name) = &self.file_name
			&& (file_name.is_empty()
				|| matches!(file_name.as_str(), "." | "..")
				|| file_name.contains(['/', '\\'])
				|| !valid_text(file_name))
		{
			return Err(Error::InvalidOption);
		}

		Ok(())
	}

	fn build(self) -> Result<rfd::AsyncFileDialog, Error> {
		self.validate()?;
		let mut dialog = rfd::AsyncFileDialog::new();

		if !self.title.is_empty() {
			dialog = dialog.set_title(self.title);
		}

		if let Some(directory) = self.directory {
			dialog = dialog.set_directory(directory);
		}

		if let Some(file_name) = self.file_name {
			dialog = dialog.set_file_name(file_name);
		}

		for filter in self.filters {
			dialog = dialog.add_filter(filter.name, &filter.extensions);
		}

		Ok(dialog)
	}
}

// Native paths retain their OS encoding. This check does not convert to UTF-8.
#[cfg(unix)]
fn path_contains_nul(path: &Path) -> bool {
	use std::os::unix::ffi::OsStrExt as _;
	path.as_os_str().as_bytes().contains(&0)
}

#[cfg(windows)]
fn path_contains_nul(path: &Path) -> bool {
	use std::os::windows::ffi::OsStrExt as _;
	path.as_os_str().encode_wide().any(|unit| unit == 0)
}

/// A configured dialog borrowing its parent for the entire asynchronous call.
///
/// Construction validates options. The borrow prevents safe callers from
/// dropping the parent before the dialog finishes. Invoke on the UI thread of
/// an initialized application; a headless macOS process can block inside rfd.
pub struct FileDialog<'window, W: HasWindowHandle + HasDisplayHandle + ?Sized> {
	dialog: rfd::AsyncFileDialog,
	_parent: &'window W,
}

impl<'window, W: HasWindowHandle + HasDisplayHandle + ?Sized> FileDialog<'window, W> {
	/// Configures a native dialog attached to a live window.
	pub fn new(parent: &'window W, options: FileOptions) -> Result<Self, Error> {
		// rfd silently ignores unavailable handles; surface that failure here.
		parent.window_handle().map_err(|_| Error::Unavailable)?;
		parent.display_handle().map_err(|_| Error::Unavailable)?;
		Ok(Self {
			dialog: options.build()?.set_parent(parent),
			_parent: parent,
		})
	}

	/// Selects one file. `None` means no selection, including native failure.
	pub async fn open_file(self) -> Option<PathBuf> {
		self.dialog.pick_file().await.map(|file| file.path().into())
	}

	/// Selects multiple files without coercing paths to Unicode.
	pub async fn open_files(self) -> Option<Vec<PathBuf>> {
		self.dialog
			.pick_files()
			.await
			.map(|files| files.into_iter().map(|file| file.path().into()).collect())
	}

	/// Selects one directory.
	pub async fn open_directory(self) -> Option<PathBuf> {
		self.dialog
			.pick_folder()
			.await
			.map(|file| file.path().into())
	}

	/// Selects multiple directories.
	pub async fn open_directories(self) -> Option<Vec<PathBuf>> {
		self.dialog
			.pick_folders()
			.await
			.map(|files| files.into_iter().map(|file| file.path().into()).collect())
	}

	/// Selects a destination without creating, truncating, or reserving it.
	///
	/// Extension handling and overwrite prompts are controlled by the OS.
	/// Callers still handle write failures and races when saving afterward.
	pub async fn save_file(self) -> Option<PathBuf> {
		self.dialog.save_file().await.map(|file| file.path().into())
	}
}

/// A message, warning, or confirmation attached to its parent window.
pub struct MessageDialog<'window, W: HasWindowHandle + HasDisplayHandle + ?Sized> {
	dialog: rfd::AsyncMessageDialog,
	_parent: &'window W,
}

impl<'window, W: HasWindowHandle + HasDisplayHandle + ?Sized> MessageDialog<'window, W> {
	/// Configures a dialog. Use standard buttons for consistent platform
	/// support.
	///
	/// Custom button labels require Windows common-controls v6 support in the
	/// application manifest and rfd feature configuration.
	pub fn new(
		parent: &'window W,
		title: &str,
		message: &str,
		level: MessageLevel,
		buttons: MessageButtons,
	) -> Result<Self, Error> {
		validate_message(title, message, &buttons)?;
		parent.window_handle().map_err(|_| Error::Unavailable)?;
		parent.display_handle().map_err(|_| Error::Unavailable)?;
		Ok(Self {
			dialog: rfd::AsyncMessageDialog::new()
				.set_parent(parent)
				.set_title(title)
				.set_description(message)
				.set_level(level)
				.set_buttons(buttons),
			_parent: parent,
		})
	}

	/// Waits for an explicit button result. Never treat `Cancel` as
	/// confirmation.
	pub async fn show(self) -> MessageDialogResult {
		self.dialog.show().await
	}
}

fn validate_message(title: &str, message: &str, buttons: &MessageButtons) -> Result<(), Error> {
	let labels: &[&str] = match buttons {
		MessageButtons::OkCustom(ok) => &[ok],
		MessageButtons::OkCancelCustom(ok, cancel) => &[ok, cancel],
		MessageButtons::YesNoCancelCustom(yes, no, cancel) => &[yes, no, cancel],
		_ => &[],
	};

	if !valid_text(title)
		|| !valid_text(message)
		|| labels
			.iter()
			.any(|label| label.trim().is_empty() || !valid_text(label))
	{
		return Err(Error::InvalidOption);
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	struct UnavailableWindow;

	impl HasWindowHandle for UnavailableWindow {
		fn window_handle(
			&self,
		) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
			Err(raw_window_handle::HandleError::Unavailable)
		}
	}

	impl HasDisplayHandle for UnavailableWindow {
		fn display_handle(
			&self,
		) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
			Err(raw_window_handle::HandleError::Unavailable)
		}
	}

	/// A parent that is never shown: a web handle holds an element id, not a
	/// pointer, so it borrows nothing.
	struct DetachedWindow;

	impl HasWindowHandle for DetachedWindow {
		#[allow(unsafe_code)]
		fn window_handle(
			&self,
		) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
			let raw = raw_window_handle::WebWindowHandle::new(1).into();
			// SAFETY: a web window handle holds an id, not borrowed data.
			Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(raw) })
		}
	}

	impl HasDisplayHandle for DetachedWindow {
		fn display_handle(
			&self,
		) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
			Ok(raw_window_handle::DisplayHandle::web())
		}
	}

	#[test]
	fn dialogs_configure_without_presenting_until_awaited() {
		let options = FileOptions {
			file_name: Some("notes.txt".into()),
			..FileOptions::default()
		};
		assert!(FileDialog::new(&DetachedWindow, options).is_ok());
		assert!(
			FileDialog::new(
				&DetachedWindow,
				FileOptions {
					file_name: Some("../escape".into()),
					..FileOptions::default()
				}
			)
			.is_err()
		);
		assert!(
			MessageDialog::new(
				&DetachedWindow,
				"Title",
				"Message",
				MessageLevel::Warning,
				MessageButtons::OkCancelCustom("Keep".into(), "Discard".into()),
			)
			.is_ok()
		);
		assert!(
			MessageDialog::new(
				&DetachedWindow,
				"Title",
				"Message",
				MessageLevel::Info,
				MessageButtons::YesNoCancelCustom("Yes".into(), " ".into(), "Cancel".into()),
			)
			.is_err()
		);
	}

	#[test]
	fn a_destroyed_window_cannot_open_a_dialog() {
		assert!(matches!(
			FileDialog::new(&UnavailableWindow, FileOptions::default()),
			Err(Error::Unavailable)
		));
		assert!(matches!(
			MessageDialog::new(
				&UnavailableWindow,
				"Title",
				"Message",
				MessageLevel::Info,
				MessageButtons::Ok
			),
			Err(Error::Unavailable)
		));
	}

	#[test]
	fn filenames_cannot_be_paths_or_contain_nul() {
		for name in ["", ".", "..", "../notes.txt", "folder\\file", "a\0b"] {
			assert_eq!(
				FileOptions {
					file_name: Some(name.into()),
					..FileOptions::default()
				}
				.validate(),
				Err(Error::InvalidOption)
			);
		}

		assert!(
			FileOptions {
				file_name: Some("notes.txt".into()),
				..FileOptions::default()
			}
			.validate()
			.is_ok()
		);
	}

	#[test]
	fn initial_directory_is_absolute_and_nul_free() {
		for directory in ["relative", ""] {
			assert_eq!(
				FileOptions {
					directory: Some(directory.into()),
					..FileOptions::default()
				}
				.validate(),
				Err(Error::InvalidOption)
			);
		}
	}

	#[test]
	fn non_unicode_native_directory_is_preserved() {
		// Each platform's own non-Unicode path: arbitrary bytes on Unix, an
		// unpaired UTF-16 surrogate on Windows.
		#[cfg(unix)]
		let directory = {
			use std::os::unix::ffi::OsStringExt as _;
			PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/\xff".to_vec()))
		};
		#[cfg(windows)]
		let directory = {
			use std::os::windows::ffi::OsStringExt as _;
			let mut wide = "C:\\Temp\\".encode_utf16().collect::<Vec<_>>();
			wide.push(0xD800);
			PathBuf::from(std::ffi::OsString::from_wide(&wide))
		};
		assert!(directory.to_str().is_none());
		let options = FileOptions {
			directory: Some(directory.clone()),
			..FileOptions::default()
		};
		assert!(options.validate().is_ok());
		assert_eq!(options.directory.as_ref(), Some(&directory));

		assert_eq!(
			FileOptions {
				directory: Some("/tmp/\0bad".into()),
				..FileOptions::default()
			}
			.validate(),
			Err(Error::InvalidOption)
		);
	}

	#[test]
	fn invalid_labels_fail_before_showing_a_dialog() {
		assert_eq!(
			validate_message("a\0b", "text", &MessageButtons::Ok),
			Err(Error::InvalidOption)
		);
		assert_eq!(
			validate_message("title", "text", &MessageButtons::OkCustom(String::new())),
			Err(Error::InvalidOption)
		);
		assert!(validate_message("title", "text", &MessageButtons::YesNoCancel).is_ok());
	}

	#[test]
	fn every_option_configures_the_native_dialog_after_validation() {
		let options = FileOptions {
			title: "Import notes".into(),
			directory: Some(std::env::temp_dir()),
			file_name: Some("notes.txt".into()),
			filters: vec![FileFilter::new("Text", ["txt", "md"]).unwrap()],
		};
		assert!(options.build().is_ok());
		assert!(FileOptions::default().build().is_ok());
		assert!(matches!(
			FileOptions {
				title: "a\0b".into(),
				..FileOptions::default()
			}
			.build(),
			Err(Error::InvalidOption)
		));
	}
}
