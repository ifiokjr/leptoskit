//! The generator entry point: options, targets, and the files they produce.

use std::fmt;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use crate::spec::ApiFunction;
use crate::spec::ApiSpec;
use crate::spec::CodegenError;

/// What `leptoskit rpc generate` can write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetLanguage {
	/// A dependency-free TypeScript package over `fetch`.
	TypeScript,
	/// A standard-library Python package, with an optional `httpx` async
	/// client.
	Python,
	/// A Go package over `net/http`.
	Go,
	/// A `reqwest`-based Rust crate.
	Rust,
	/// A Dart package over `package:http`.
	Dart,
	/// A clap-based command line, one command per RPC method.
	Cli,
	/// The OpenAPI 3.1 document, as JSON and YAML.
	OpenApi,
}

impl TargetLanguage {
	/// Every target, in display order.
	pub const ALL: &'static [Self] = &[
		Self::TypeScript,
		Self::Python,
		Self::Go,
		Self::Rust,
		Self::Dart,
		Self::Cli,
		Self::OpenApi,
	];

	/// The canonical name, as accepted on the command line and in
	/// `leptoskit.toml`.
	#[must_use]
	pub fn as_str(self) -> &'static str {
		match self {
			Self::TypeScript => "typescript",
			Self::Python => "python",
			Self::Go => "go",
			Self::Rust => "rust",
			Self::Dart => "dart",
			Self::Cli => "cli",
			Self::OpenApi => "openapi",
		}
	}

	/// Where the target's files go by default, relative to the project root.
	#[must_use]
	pub fn default_out_dir(self) -> &'static str {
		match self {
			Self::TypeScript => "clients/typescript",
			Self::Python => "clients/python",
			Self::Go => "clients/go",
			Self::Rust => "clients/rust",
			Self::Dart => "clients/dart",
			Self::Cli => "clients/cli",
			Self::OpenApi => "clients/openapi",
		}
	}

	/// Whether the target is a cargo crate `--build` can check.
	#[must_use]
	pub fn is_cargo(self) -> bool {
		matches!(self, Self::Rust | Self::Cli)
	}
}

impl fmt::Display for TargetLanguage {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.as_str())
	}
}

impl FromStr for TargetLanguage {
	type Err = CodegenError;

	fn from_str(value: &str) -> Result<Self, Self::Err> {
		match value.to_ascii_lowercase().as_str() {
			"typescript" | "ts" => Ok(Self::TypeScript),
			"python" | "py" => Ok(Self::Python),
			"golang" | "go" => Ok(Self::Go),
			"rust" | "rs" => Ok(Self::Rust),
			"dart" | "flutter" => Ok(Self::Dart),
			"cli" | "command-line" => Ok(Self::Cli),
			"openapi" | "oas" => Ok(Self::OpenApi),
			other => Err(CodegenError::UnknownTarget(other.to_owned())),
		}
	}
}

/// How much of a target regeneration writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Scaffold {
	/// Generated code always; scaffolding only where the file does not exist
	/// yet. The default: the package is created once, then the user owns its
	/// manifest while the code keeps tracking the RPC.
	#[default]
	Create,
	/// Everything, overwriting existing scaffolding too.
	Full,
	/// Generated code only; scaffolding is never written, so the user defines
	/// the package (`package.json`, `pyproject.toml`, `go.mod`, `Cargo.toml`,
	/// `pubspec.yaml`) themselves.
	Files,
}

impl Scaffold {
	/// Every mode, in display order.
	pub const ALL: &'static [Self] = &[Self::Create, Self::Full, Self::Files];

	/// The canonical name, as accepted on the command line and in
	/// `leptoskit.toml`.
	#[must_use]
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Create => "create",
			Self::Full => "full",
			Self::Files => "files",
		}
	}
}

impl fmt::Display for Scaffold {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.as_str())
	}
}

impl FromStr for Scaffold {
	type Err = CodegenError;

	fn from_str(value: &str) -> Result<Self, Self::Err> {
		match value.to_ascii_lowercase().as_str() {
			"create" => Ok(Self::Create),
			"full" => Ok(Self::Full),
			"files" | "code" => Ok(Self::Files),
			other => Err(CodegenError::UnknownScaffold(other.to_owned())),
		}
	}
}

/// What a generated file is, which decides whether regeneration may replace
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileKind {
	/// Generated source: the typed client, the commands, the OpenAPI
	/// document. Always rewritten, because it has to track the RPC.
	Code,
	/// Package scaffolding: manifests (`package.json`, `pyproject.toml`,
	/// `go.mod`, `Cargo.toml`, `pubspec.yaml`), tool configuration, READMEs.
	/// Written once and then left to the user, unless a full scaffold is
	/// asked for.
	Scaffold,
}

/// One generated file, relative to the output directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFile {
	/// The path, relative to the output directory.
	pub path: PathBuf,
	/// The file contents.
	pub contents: String,
	/// What the file is.
	pub kind: FileKind,
}

impl GeneratedFile {
	/// Generated code, always rewritten.
	pub(crate) fn code(path: impl Into<PathBuf>, contents: impl Into<String>) -> Self {
		Self {
			path: path.into(),
			contents: contents.into(),
			kind: FileKind::Code,
		}
	}

	/// Package scaffolding, written per [`Scaffold`] mode.
	pub(crate) fn scaffold(path: impl Into<PathBuf>, contents: impl Into<String>) -> Self {
		Self {
			path: path.into(),
			contents: contents.into(),
			kind: FileKind::Scaffold,
		}
	}
}

/// The output of one generator run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Generated {
	/// The files to write.
	pub files: Vec<GeneratedFile>,
}

/// What [`Generated::write_to`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Written {
	/// The files written, as absolute paths.
	pub written: Vec<PathBuf>,
	/// Scaffolding left alone because the mode said so, as absolute paths.
	pub kept: Vec<PathBuf>,
}

impl Generated {
	/// Writes the files under `dir` according to `scaffold`, creating
	/// directories as needed.
	///
	/// # Errors
	///
	/// Returns the first IO error.
	pub fn write_to(&self, dir: &Path, scaffold: Scaffold) -> std::io::Result<Written> {
		let mut report = Written::default();

		for file in &self.files {
			let path = dir.join(&file.path);

			let write = match (file.kind, scaffold) {
				(FileKind::Code, _) | (FileKind::Scaffold, Scaffold::Full) => true,
				(FileKind::Scaffold, Scaffold::Create) => !path.exists(),
				(FileKind::Scaffold, Scaffold::Files) => false,
			};

			if !write {
				report.kept.push(path);
				continue;
			}

			if let Some(parent) = path.parent() {
				std::fs::create_dir_all(parent)?;
			}

			std::fs::write(&path, &file.contents)?;
			report.written.push(path);
		}

		Ok(report)
	}
}

/// What to generate, and how to name it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerateOptions {
	/// The target.
	pub language: TargetLanguage,
	/// The directory the files are written to. Generators only use it to
	/// compute relative paths; writing is the caller's job.
	pub out_dir: PathBuf,
	/// The package name: the npm package, the PyPI distribution and import
	/// module, the Go module suffix, the cargo crate, or the Dart package.
	pub package_name: String,
	/// The package version; defaults to the spec's version.
	pub version: Option<String>,
	/// The server origin baked in as the client default; defaults to the
	/// spec's first server, or none (the page's own origin).
	pub server_url: Option<String>,
	/// Whether `#[api(internal)]` functions are included.
	pub include_internal: bool,
	/// Function-name globs to leave out.
	pub exclude: Vec<String>,
}

impl GenerateOptions {
	/// Options for `language`, written to its default directory.
	#[must_use]
	pub fn new(language: TargetLanguage, package_name: &str) -> Self {
		Self {
			language,
			out_dir: PathBuf::from(language.default_out_dir()),
			package_name: package_name.to_owned(),
			version: None,
			server_url: None,
			include_internal: false,
			exclude: Vec::new(),
		}
	}
}

impl ApiSpec {
	/// Generates `options.language`.
	///
	/// Internal functions and channels (unless included) and
	/// `options.exclude` matches are removed first, along with the schemas
	/// only they referenced; functions
	/// whose encodings the target cannot speak are skipped with a note in the
	/// README rather than failing the run.
	pub fn generate(&self, options: &GenerateOptions) -> Generated {
		let spec = self.published(options.include_internal, &options.exclude);
		let context = Context::new(&spec, options);

		match options.language {
			TargetLanguage::TypeScript => crate::typescript::generate(&context),
			TargetLanguage::Python => crate::python::generate(&context),
			TargetLanguage::Go => crate::go::generate(&context),
			TargetLanguage::Rust => crate::rust::generate(&context),
			TargetLanguage::Dart => crate::dart::generate(&context),
			TargetLanguage::Cli => crate::cli::generate(&context),
			TargetLanguage::OpenApi => {
				let document = spec.to_openapi();
				let json = serde_json::to_string_pretty(&document)
					.expect("the OpenAPI document always serializes");
				Generated {
					files: vec![
						GeneratedFile::code("openapi.json", json + "\n"),
						GeneratedFile::code("openapi.yaml", crate::yaml::to_yaml(&document)),
					],
				}
			}
		}
	}
}

/// What every generator works from: the filtered spec, the resolved naming
/// and URL choices, and the functions it can actually call.
pub(crate) struct Context<'a> {
	pub spec: &'a ApiSpec,
	pub options: &'a GenerateOptions,
	pub version: String,
	/// The default server origin, when one is configured.
	pub server_url: Option<String>,
	/// Functions whose encodings generated clients can speak.
	pub functions: Vec<&'a ApiFunction>,
	/// Functions left out, with the reason.
	pub skipped: Vec<(&'a str, String)>,
}

impl<'a> Context<'a> {
	fn new(spec: &'a ApiSpec, options: &'a GenerateOptions) -> Self {
		let version = options
			.version
			.clone()
			.unwrap_or_else(|| spec.version.clone());
		let server_url = options
			.server_url
			.clone()
			.or_else(|| spec.servers.first().cloned())
			.filter(|url| !url.is_empty());
		let mut functions = Vec::new();
		let mut skipped = Vec::new();

		for function in &spec.functions {
			if !function.input.is_supported() {
				skipped.push((
					function.name.as_str(),
					format!(
						"requests are encoded as {}, which generated clients cannot build",
						function.input.content_type()
					),
				));
			} else if !function.output.is_supported() {
				skipped.push((
					function.name.as_str(),
					format!(
						"responses are encoded as {}, which generated clients cannot decode",
						function.output.content_type()
					),
				));
			} else {
				functions.push(function);
			}
		}

		Self {
			spec,
			options,
			version,
			server_url,
			functions,
			skipped,
		}
	}

	pub(crate) fn functions(&self) -> impl Iterator<Item = &'a ApiFunction> {
		self.functions.iter().copied()
	}

	/// The line every generated file opens with.
	pub(crate) fn banner(&self, comment: &str) -> String {
		format!(
			"{comment} Generated by `leptoskit rpc generate` from {} {}. Do not edit; regenerate \
			 instead.\n",
			crate::naming::comment_text(&self.spec.title),
			crate::naming::comment_text(&self.spec.version)
		)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn targets_parse_from_their_aliases() {
		assert_eq!(
			"ts".parse::<TargetLanguage>().ok(),
			Some(TargetLanguage::TypeScript)
		);
		assert_eq!(
			"PY".parse::<TargetLanguage>().ok(),
			Some(TargetLanguage::Python)
		);
		assert_eq!(
			"golang".parse::<TargetLanguage>().ok(),
			Some(TargetLanguage::Go)
		);
		assert_eq!(
			"rs".parse::<TargetLanguage>().ok(),
			Some(TargetLanguage::Rust)
		);
		assert_eq!(
			"flutter".parse::<TargetLanguage>().ok(),
			Some(TargetLanguage::Dart)
		);
		assert_eq!(
			"command-line".parse::<TargetLanguage>().ok(),
			Some(TargetLanguage::Cli)
		);
		assert_eq!(
			"OpenAPI".parse::<TargetLanguage>().ok(),
			Some(TargetLanguage::OpenApi)
		);
		assert!(matches!(
			"cobol".parse::<TargetLanguage>(),
			Err(CodegenError::UnknownTarget(_))
		));

		for target in TargetLanguage::ALL {
			assert_eq!(
				target.as_str().parse::<TargetLanguage>().ok(),
				Some(*target)
			);
		}
	}

	#[test]
	fn scaffold_modes_parse_and_display() {
		for mode in Scaffold::ALL {
			assert_eq!(mode.as_str().parse::<Scaffold>().ok(), Some(*mode));
			assert_eq!(mode.to_string(), mode.as_str());
		}

		assert_eq!("code".parse::<Scaffold>().ok(), Some(Scaffold::Files));
		assert!(matches!(
			"sometimes".parse::<Scaffold>(),
			Err(CodegenError::UnknownScaffold(_))
		));
		assert_eq!(Scaffold::default(), Scaffold::Create);
	}

	#[test]
	fn write_to_respects_the_scaffold_mode() {
		let dir = std::env::temp_dir().join(format!(
			"leptoskit_rpc_codegen_write_{}_{}",
			std::process::id(),
			std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.map(|d| d.as_nanos())
				.unwrap_or_default()
		));
		let generated = Generated {
			files: vec![
				GeneratedFile::code("src/lib.rs", "v1"),
				GeneratedFile::scaffold("Cargo.toml", "v1"),
			],
		};

		// Files mode: code only.
		let report = generated.write_to(&dir, Scaffold::Files).expect("write");
		assert_eq!(report.written, [dir.join("src/lib.rs")]);
		assert_eq!(report.kept, [dir.join("Cargo.toml")]);
		assert!(!dir.join("Cargo.toml").exists());

		// Create mode: scaffolding appears once...
		let report = generated.write_to(&dir, Scaffold::Create).expect("write");
		assert_eq!(report.written.len(), 2);
		assert_eq!(report.kept, [] as [PathBuf; 0]);
		std::fs::write(dir.join("Cargo.toml"), "edited by hand").expect("edit");

		// ...and is never overwritten afterwards, while code always is.
		let updated = Generated {
			files: vec![
				GeneratedFile::code("src/lib.rs", "v2"),
				GeneratedFile::scaffold("Cargo.toml", "v2"),
			],
		};
		let report = updated.write_to(&dir, Scaffold::Create).expect("write");
		assert_eq!(report.written, [dir.join("src/lib.rs")]);
		assert_eq!(report.kept, [dir.join("Cargo.toml")]);
		assert_eq!(
			std::fs::read_to_string(dir.join("src/lib.rs")).expect("lib"),
			"v2"
		);
		assert_eq!(
			std::fs::read_to_string(dir.join("Cargo.toml")).expect("manifest"),
			"edited by hand"
		);

		// Full mode: everything, overwriting.
		let report = updated.write_to(&dir, Scaffold::Full).expect("write");
		assert_eq!(report.written.len(), 2);
		assert_eq!(
			std::fs::read_to_string(dir.join("Cargo.toml")).expect("manifest"),
			"v2"
		);
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn empty_specs_generate_valid_packages() {
		let spec = ApiSpec::empty("Nothing", "0.0.0");

		for language in TargetLanguage::ALL {
			let generated = spec.generate(&GenerateOptions::new(*language, "nothing"));
			assert!(!generated.files.is_empty(), "{language}");

			for file in &generated.files {
				assert!(
					!file.contents.is_empty(),
					"{language}: {}",
					file.path.display()
				);
			}
		}
	}
}
