//! Browser selection and executable discovery for extension sessions.
use std::path::Path;
use std::path::PathBuf;

use leptoskit_config::Browser;

/// Browser family used by extension development and smoke tests.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ExtensionBrowser {
	/// Chrome for Testing.
	#[default]
	#[value(alias = "chrome")]
	Chromium,
	/// Firefox through geckodriver.
	Firefox,
}

impl ExtensionBrowser {
	/// The matching package family.
	#[must_use]
	pub const fn config(self) -> Browser {
		match self {
			Self::Chromium => Browser::Chromium,
			Self::Firefox => Browser::Firefox,
		}
	}

	/// Checks the executables needed before building the app.
	/// # Errors
	/// Describes which executable to install or configure.
	pub fn support(self) -> Result<(), String> {
		match self {
			Self::Chromium => super::dev::support(),
			Self::Firefox => {
				firefox().ok_or("install Firefox, or set LEPTOSKIT_FIREFOX to its executable")?;
				geckodriver().ok_or(
					"install geckodriver on PATH, or set LEPTOSKIT_GECKODRIVER to its executable",
				)?;
				Ok(())
			}
		}
	}
}

fn find(variable: &str, candidates: &[PathBuf]) -> Option<PathBuf> {
	if let Some(path) = std::env::var_os(variable) {
		return Some(PathBuf::from(path)).filter(|p| p.is_file());
	}

	candidates.iter().find(|p| p.is_file()).cloned()
}

fn on_path(program: &str) -> Vec<PathBuf> {
	std::env::var_os("PATH")
		.map(|path| {
			std::env::split_paths(&path)
				.map(|dir| dir.join(program))
				.collect()
		})
		.unwrap_or_default()
}

/// Finds Firefox through its explicit override, PATH, or usual install
/// location.
#[must_use]
pub fn firefox() -> Option<PathBuf> {
	let mut candidates = on_path(if cfg!(windows) {
		"firefox.exe"
	} else {
		"firefox"
	});
	candidates.push(PathBuf::from(
		"/Applications/Firefox.app/Contents/MacOS/firefox",
	));

	if let Some(home) = std::env::var_os("HOME") {
		candidates.push(Path::new(&home).join("Applications/Firefox.app/Contents/MacOS/firefox"));
	}

	for name in ["PROGRAMFILES", "PROGRAMFILES(X86)"] {
		if let Some(dir) = std::env::var_os(name) {
			candidates.push(Path::new(&dir).join("Mozilla Firefox/firefox.exe"));
		}
	}

	find("LEPTOSKIT_FIREFOX", &candidates)
}

/// Finds geckodriver through its explicit override or PATH.
#[must_use]
pub fn geckodriver() -> Option<PathBuf> {
	find(
		"LEPTOSKIT_GECKODRIVER",
		&on_path(if cfg!(windows) {
			"geckodriver.exe"
		} else {
			"geckodriver"
		}),
	)
}
