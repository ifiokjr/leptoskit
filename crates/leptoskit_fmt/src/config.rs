//! The settings a format run needs: the markup style from `leptoskit.toml`,
//! and the layout rustfmt will use, read from the same `rustfmt.toml` rustfmt
//! reads so markup and Rust agree on width and indentation.

use std::path::Path;
use std::path::PathBuf;

pub use leptoskit_config::AttributeBraces;
pub use leptoskit_config::ClosingTags;
pub use leptoskit_config::FormatConfig;
use serde::Deserialize;

use crate::error::FormatError;

/// Everything that decides how one file is formatted.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Settings {
	/// The markup style.
	pub markup: FormatConfig,
	/// The layout rustfmt uses for the file.
	pub layout: Layout,
	/// The edition passed to rustfmt when its configuration names none.
	pub edition: String,
	/// The `rustfmt.toml` passed to rustfmt, when there is one.
	pub rustfmt_config: Option<PathBuf>,
	/// The directory of the file, which rustfmt runs in.
	pub directory: PathBuf,
}

impl Settings {
	/// The settings for a file at `path`: the nearest `leptoskit.toml`'s
	/// `[format]` table, the nearest rustfmt configuration, and the edition of
	/// the package that owns the file.
	///
	/// # Errors
	///
	/// Returns [`FormatError::Config`] when a configuration file exists but
	/// cannot be read or parsed.
	pub fn for_path(path: &Path) -> Result<Self, FormatError> {
		let dir = path.parent().unwrap_or(path);
		let dir = if dir.as_os_str().is_empty() {
			Path::new(".")
		} else {
			dir
		};
		let markup = load_markup_config(dir)?;
		let rustfmt_config = find_rustfmt_config(dir);
		let layout = match &rustfmt_config {
			Some(config) => Layout::from_rustfmt_toml(config)?,
			None => Layout::default(),
		};
		let edition = layout
			.edition
			.clone()
			.or_else(|| package_edition(dir))
			.unwrap_or_else(|| DEFAULT_EDITION.to_owned());
		Ok(Self {
			markup,
			layout,
			edition,
			rustfmt_config,
			directory: dir.to_owned(),
		})
	}
}

impl Default for Settings {
	fn default() -> Self {
		Self {
			markup: FormatConfig::default(),
			layout: Layout::default(),
			edition: DEFAULT_EDITION.to_owned(),
			rustfmt_config: None,
			directory: PathBuf::from("."),
		}
	}
}

/// The edition assumed when neither rustfmt nor cargo names one.
const DEFAULT_EDITION: &str = "2021";

/// How a line is laid out: the subset of rustfmt's configuration that decides
/// where markup goes.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(default)]
pub struct Layout {
	/// The column lines should not pass.
	pub max_width: usize,
	/// Indent with tabs instead of spaces.
	pub hard_tabs: bool,
	/// The width of one indentation level, and of a tab when measuring.
	pub tab_spaces: usize,
	/// The line ending to write.
	pub newline_style: NewlineStyle,
	/// The edition the configuration names, if any.
	pub edition: Option<String>,
}

impl Default for Layout {
	fn default() -> Self {
		Self {
			max_width: 100,
			hard_tabs: false,
			tab_spaces: 4,
			newline_style: NewlineStyle::Auto,
			edition: None,
		}
	}
}

impl Layout {
	/// Reads the layout settings from a rustfmt configuration file; every
	/// other key belongs to rustfmt and is ignored here.
	///
	/// # Errors
	///
	/// Returns [`FormatError::Config`] when the file cannot be read or is not
	/// valid TOML.
	pub fn from_rustfmt_toml(path: &Path) -> Result<Self, FormatError> {
		let raw = std::fs::read_to_string(path).map_err(|error| {
			FormatError::Config {
				path: path.to_owned(),
				message: error.to_string(),
			}
		})?;
		toml::from_str(&raw).map_err(|error| {
			FormatError::Config {
				path: path.to_owned(),
				message: error.to_string(),
			}
		})
	}

	/// One level of indentation.
	pub(crate) fn indent_unit(&self) -> String {
		if self.hard_tabs {
			"\t".to_owned()
		} else {
			" ".repeat(self.tab_spaces)
		}
	}

	/// The display width of `text`, counting a tab as `tab_spaces` columns as
	/// rustfmt does.
	pub(crate) fn width(&self, text: &str) -> usize {
		if text.is_ascii() {
			let tabs = text.bytes().filter(|byte| *byte == b'\t').count();
			return text.len() + tabs * self.tab_spaces.saturating_sub(1);
		}
		text.chars()
			.map(|character| {
				if character == '\t' {
					self.tab_spaces
				} else {
					unicode_width::UnicodeWidthChar::width(character).unwrap_or(0)
				}
			})
			.sum()
	}
}

/// rustfmt's `newline_style`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Deserialize)]
pub enum NewlineStyle {
	/// Use the style of the first line ending in the input.
	#[default]
	Auto,
	/// The platform's line ending.
	Native,
	/// `\n`
	Unix,
	/// `\r\n`
	Windows,
}

impl NewlineStyle {
	/// Whether to write `\r\n`, given the input text.
	pub(crate) fn use_crlf(self, input: &str) -> bool {
		match self {
			Self::Auto => {
				input
					.find('\n')
					.is_some_and(|index| index > 0 && input.as_bytes()[index - 1] == b'\r')
			}
			Self::Native => cfg!(windows),
			Self::Unix => false,
			Self::Windows => true,
		}
	}
}

/// rustfmt's own search: the nearest `rustfmt.toml` or `.rustfmt.toml` in
/// `dir` or an ancestor, then the user's configuration directory.
pub fn find_rustfmt_config(dir: &Path) -> Option<PathBuf> {
	let dir = std::path::absolute(dir).ok()?;
	let found = dir.ancestors().find_map(rustfmt_config_in);
	found.or_else(|| {
		let home = std::env::var_os("HOME").map(PathBuf::from);
		let config_home = std::env::var_os("XDG_CONFIG_HOME")
			.map(PathBuf::from)
			.or_else(|| home.as_ref().map(|home| home.join(".config")));
		home.as_deref().and_then(rustfmt_config_in).or_else(|| {
			config_home
				.map(|config| config.join("rustfmt"))
				.as_deref()
				.and_then(rustfmt_config_in)
		})
	})
}

fn rustfmt_config_in(dir: &Path) -> Option<PathBuf> {
	["rustfmt.toml", ".rustfmt.toml"]
		.into_iter()
		.map(|name| dir.join(name))
		.find(|path| path.is_file())
}

/// The `[format]` table of the nearest `leptoskit.toml`, or the defaults.
fn load_markup_config(dir: &Path) -> Result<FormatConfig, FormatError> {
	let Ok(dir) = std::path::absolute(dir) else {
		return Ok(FormatConfig::default());
	};
	let Some(path) = dir
		.ancestors()
		.map(|dir| dir.join(leptoskit_config::CONFIG_FILE))
		.find(|path| path.is_file())
	else {
		return Ok(FormatConfig::default());
	};
	FormatConfig::load_from_file(&path).map_err(|error| {
		FormatError::Config {
			path,
			message: error.to_string(),
		}
	})
}

/// The edition of the package containing `dir`, following
/// `edition.workspace = true` to the workspace root.
fn package_edition(dir: &Path) -> Option<String> {
	#[derive(Deserialize)]
	struct Manifest {
		package: Option<Package>,
		workspace: Option<Workspace>,
	}
	#[derive(Deserialize)]
	struct Package {
		edition: Option<toml::Value>,
	}
	#[derive(Deserialize)]
	struct Workspace {
		package: Option<WorkspacePackage>,
	}
	#[derive(Deserialize)]
	struct WorkspacePackage {
		edition: Option<String>,
	}

	let dir = std::path::absolute(dir).ok()?;
	let mut inherits = false;
	for ancestor in dir.ancestors() {
		let path = ancestor.join("Cargo.toml");
		let Ok(raw) = std::fs::read_to_string(&path) else {
			continue;
		};
		let Ok(manifest) = toml::from_str::<Manifest>(&raw) else {
			continue;
		};
		if !inherits {
			match manifest.package.and_then(|package| package.edition) {
				Some(toml::Value::String(edition)) => return Some(edition),
				Some(toml::Value::Table(_)) => inherits = true,
				_ => {}
			}
		}
		if let Some(edition) = manifest
			.workspace
			.and_then(|workspace| workspace.package)
			.and_then(|package| package.edition)
		{
			return Some(edition);
		}
		if !inherits {
			// The nearest manifest owns the file; without an edition or a
			// workspace to inherit from, cargo's default applies.
			return Some("2015".to_owned());
		}
	}
	None
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reads_layout_from_rustfmt_toml() {
		let layout: Layout =
			toml::from_str("hard_tabs = true\nmax_width = 80\nunstable_features = true").unwrap();
		assert!(layout.hard_tabs);
		assert_eq!(layout.max_width, 80);
		assert_eq!(layout.tab_spaces, 4);
		assert_eq!(layout.indent_unit(), "\t");
	}

	#[test]
	fn measures_tabs_and_wide_characters() {
		let layout = Layout::default();
		assert_eq!(layout.width("\tab"), 6);
		assert_eq!(layout.width("日本"), 4);
	}

	#[test]
	fn detects_crlf_input() {
		assert!(NewlineStyle::Auto.use_crlf("a\r\nb"));
		assert!(!NewlineStyle::Auto.use_crlf("a\nb\r\n"));
		assert!(NewlineStyle::Windows.use_crlf("a\nb"));
	}

	fn scratch(name: &str) -> PathBuf {
		let dir = std::env::temp_dir().join(format!(
			"leptoskit-fmt-config-{name}-{}",
			std::process::id()
		));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).expect("scratch dir");
		dir
	}

	fn write(path: &Path, contents: &str) {
		std::fs::create_dir_all(path.parent().expect("parent")).expect("create dir");
		std::fs::write(path, contents).expect("write");
	}

	#[test]
	fn finds_the_package_edition() {
		let root = scratch("edition");
		write(
			&root.join("explicit/Cargo.toml"),
			"[package]\nname = \"a\"\nedition = \"2021\"\n",
		);
		assert_eq!(
			package_edition(&root.join("explicit/src")).as_deref(),
			Some("2021")
		);

		write(
			&root.join("workspace/Cargo.toml"),
			"[workspace]\nmembers = [\"member\"]\n[workspace.package]\nedition = \"2024\"\n",
		);
		write(
			&root.join("workspace/member/Cargo.toml"),
			"[package]\nname = \"m\"\nedition.workspace = true\n",
		);
		assert_eq!(
			package_edition(&root.join("workspace/member/src")).as_deref(),
			Some("2024")
		);

		write(&root.join("plain/Cargo.toml"), "[package]\nname = \"p\"\n");
		assert_eq!(
			package_edition(&root.join("plain")).as_deref(),
			Some("2015")
		);

		write(&root.join("broken/Cargo.toml"), "not toml [");
		assert_eq!(package_edition(&root.join("broken")), None);
		std::fs::remove_dir_all(root).expect("clean up");
	}

	#[test]
	fn reports_unreadable_rustfmt_configuration() {
		let error = Layout::from_rustfmt_toml(Path::new("/nonexistent/rustfmt.toml")).unwrap_err();
		assert!(matches!(error, FormatError::Config { .. }), "{error}");
	}

	#[test]
	fn resolves_every_newline_style() {
		assert_eq!(NewlineStyle::Native.use_crlf("a\nb"), cfg!(windows));
		assert!(!NewlineStyle::Unix.use_crlf("a\r\nb"));
		assert_eq!(Layout::default().width("\t日"), 6);
	}

	#[test]
	fn a_bare_file_name_reads_the_working_directory() {
		let settings = Settings::for_path(Path::new("lib.rs")).unwrap();
		assert_eq!(settings.directory, Path::new("."));
	}

	#[test]
	fn finds_this_repository_settings() {
		let settings = Settings::for_path(
			Path::new(env!("CARGO_MANIFEST_DIR"))
				.join("src/lib.rs")
				.as_path(),
		)
		.unwrap();
		assert!(settings.layout.hard_tabs);
		assert_eq!(settings.edition, "2024");
		assert!(settings.rustfmt_config.is_some());
	}
}
