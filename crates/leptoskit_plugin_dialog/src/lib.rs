//! Asynchronous desktop dialogs and browser file selection.
//!
//! Desktop calls need an initialized application event loop. Keep the parent
//! window alive until the dialog completes, and start dialogs on its UI thread.
//! The browser backend requires a secure context, user activation, and the
//! File System Access API. No synchronous bridge handler is registered: a host
//! must await dialogs outside the guest interpreter.

/// Dialog setup or browser operation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
	/// A title, filename, directory, or filter is malformed.
	#[error("invalid dialog option")]
	InvalidOption,
	/// The platform does not expose the requested dialog API.
	#[error("dialog API unavailable")]
	Unavailable,
	/// The browser denied permission or requires user activation.
	#[error("dialog permission denied")]
	PermissionDenied,
	/// The browser rejected an operation for another reason.
	#[error("browser dialog operation failed")]
	Native,
	/// Reading the selected file would exceed the caller's byte limit.
	#[error("selected file exceeds the byte limit")]
	TooLarge,
}

/// A display label and literal filename extensions, without dots or wildcards.
///
/// Filters guide selection; they do not validate a file's contents or grant
/// filesystem permissions. Omit filters to allow all files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFilter {
	name: String,
	extensions: Vec<String>,
}

impl FileFilter {
	/// Creates a nonempty filter, such as `("Images", ["png", "jpg"])`.
	pub fn new(
		name: impl Into<String>,
		extensions: impl IntoIterator<Item = impl Into<String>>,
	) -> Result<Self, Error> {
		let name = name.into();
		let extensions: Vec<String> = extensions.into_iter().map(Into::into).collect();
		if name.trim().is_empty()
			|| !valid_text(&name)
			|| extensions.is_empty()
			|| extensions.iter().any(|extension| {
				extension.is_empty()
					|| !extension
						.bytes()
						.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
			}) {
			return Err(Error::InvalidOption);
		}
		Ok(Self { name, extensions })
	}

	/// Human-readable filter label.
	pub fn name(&self) -> &str {
		&self.name
	}

	/// Literal extensions in the caller's original order and case.
	pub fn extensions(&self) -> &[String] {
		&self.extensions
	}
}

fn valid_text(text: &str) -> bool {
	!text.contains('\0')
}

#[cfg(any(
	target_os = "macos",
	target_os = "windows",
	all(target_os = "linux", not(target_env = "ohos"))
))]
pub mod desktop;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod web;

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn filters_preserve_labels_and_literal_extensions() {
		let filter = FileFilter::new("Images", ["PNG", "jpg"]).unwrap();
		assert_eq!(filter.name(), "Images");
		assert_eq!(filter.extensions(), ["PNG", "jpg"]);
	}

	#[test]
	fn filters_reject_globs_paths_mime_types_and_empty_values() {
		for extension in [
			"",
			".png",
			"*.png",
			"a/b",
			"a\\b",
			"image/png",
			"a\0b",
			"tar.gz",
		] {
			assert_eq!(
				FileFilter::new("Files", [extension]),
				Err(Error::InvalidOption)
			);
		}
		assert_eq!(FileFilter::new(" ", ["png"]), Err(Error::InvalidOption));
		assert_eq!(
			FileFilter::new("Files", Vec::<String>::new()),
			Err(Error::InvalidOption)
		);
	}
}
