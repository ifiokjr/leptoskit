//! Cargo build-script support for platform-specific application modules.
//!
//! Call [`platforms`] from the application's `build.rs`. It records the
//! selected platform for `leptoskit::platform_module!` and watches the source
//! directory, including files that are added or removed between builds.

use std::fmt;
use std::path::PathBuf;

const PLATFORMS: [&str; 8] = [
	"web",
	"ios",
	"android",
	"macos",
	"windows",
	"linux",
	"tui",
	"extension",
];

/// Configures platform module selection for the application being built.
///
/// Features named `web`, `ios`, `android`, `macos`, `windows`, `linux`,
/// `tui`, and `extension` select the corresponding `.<platform>.rs`
/// overrides. An extension is built from the web client, so `extension`
/// together with `web` selects `extension`. With no platform
/// feature enabled, modules use their shared `.rs` source. Multiple platform
/// features are rejected because native guests all target Wasm, so the Rust
/// compilation target cannot identify which device the application is for.
///
/// This belongs in the application's build script, rather than the SDK's:
/// SDK features remain additive, while one application build selects one
/// device platform.
///
/// ```no_run
/// fn main() -> Result<(), leptoskit_build::Error> {
/// 	leptoskit_build::platforms()
/// }
/// ```
///
/// # Errors
///
/// Returns an error outside a Cargo build script, when the application has
/// no source directory, or when multiple platform features are enabled.
pub fn platforms() -> Result<(), Error> {
	let manifest = std::env::var_os("CARGO_MANIFEST_DIR")
		.map(PathBuf::from)
		.ok_or(Error::MissingManifestDirectory)?;
	let source = manifest.join("src");
	let metadata = std::fs::metadata(&source).map_err(|source_error| {
		Error::SourceDirectory {
			path: source.clone(),
			source: source_error,
		}
	})?;
	let enabled = PLATFORMS
		.iter()
		.copied()
		.filter(|platform| {
			std::env::var_os(format!("CARGO_FEATURE_{}", platform.to_ascii_uppercase())).is_some()
		})
		.collect::<Vec<_>>();
	let platform = select_platform(&enabled).inspect_err(|error| {
		println!("cargo:error={error}");
	})?;

	if !metadata.is_dir() {
		return Err(Error::NotSourceDirectory(source));
	}

	// Cargo recursively watches directories, which also observes overrides
	// that did not exist when the procedural macro last selected a source.
	println!("cargo:rerun-if-changed={}", source.display());
	println!(
		"cargo:rerun-if-changed={}",
		manifest.join("Cargo.toml").display()
	);
	println!("cargo:rustc-env=LEPTOSKIT_PLATFORM={platform}");
	Ok(())
}

/// An application build cannot safely select its platform modules.
#[derive(Debug)]
pub enum Error {
	/// Cargo did not supply the directory of the application manifest.
	MissingManifestDirectory,
	/// Reading the application's source directory failed.
	SourceDirectory {
		/// The directory that could not be read.
		path: PathBuf,
		/// The underlying filesystem error.
		source: std::io::Error,
	},
	/// The application's `src` path exists but is not a directory.
	NotSourceDirectory(PathBuf),
	/// Application features select more than one device platform.
	ConflictingPlatforms(Vec<&'static str>),
}

impl fmt::Display for Error {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::MissingManifestDirectory => {
				formatter.write_str(
					"leptoskit_build::platforms() must run from the application's Cargo build \
					 script",
				)
			}
			Self::SourceDirectory { path, source } => {
				write!(formatter, "could not read {}: {source}", path.display())
			}
			Self::NotSourceDirectory(path) => {
				write!(formatter, "{} must be a source directory", path.display())
			}
			Self::ConflictingPlatforms(platforms) => {
				write!(
					formatter,
					"application platform features are mutually exclusive; enabled: {}. Build one \
					 platform at a time with --no-default-features --features <platform>",
					platforms.join(", "),
				)
			}
		}
	}
}

impl std::error::Error for Error {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			Self::SourceDirectory { source, .. } => Some(source),
			_ => None,
		}
	}
}

/// Resolves one device identity without consulting the compilation target.
fn select_platform(enabled: &[&'static str]) -> Result<&'static str, Error> {
	match enabled {
		[] => Ok("shared"),
		[platform] => Ok(platform),
		// The extension feature turns on the web client it is built from.
		["web", "extension"] => Ok("extension"),
		platforms => Err(Error::ConflictingPlatforms(platforms.to_vec())),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn no_feature_uses_shared_modules() {
		assert_eq!(select_platform(&[]).unwrap(), "shared");
	}

	#[test]
	fn each_device_feature_selects_itself() {
		for platform in PLATFORMS {
			assert_eq!(select_platform(&[platform]).unwrap(), platform);
		}
	}

	#[test]
	fn an_extension_is_built_on_the_web_client() {
		assert_eq!(select_platform(&["web", "extension"]).unwrap(), "extension");
	}

	#[test]
	fn conflicting_platforms_name_the_features_and_fix() {
		let error = select_platform(&["web", "ios"]).unwrap_err().to_string();

		assert!(error.contains("web, ios"), "{error}");
		assert!(error.contains("--no-default-features"), "{error}");
	}
}
