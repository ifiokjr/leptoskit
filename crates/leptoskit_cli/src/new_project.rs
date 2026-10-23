//! Generates a workspace with one application crate and app-owned platform
//! runners.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use camino::Utf8Path;
use camino::Utf8PathBuf;
use leptoskit_config::AppConfig;
use leptoskit_config::BuildConfig;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::NativeConfig;
use leptoskit_config::NativeTarget;
use leptoskit_config::Platform;
use leptoskit_config::RenderMode;

const NATIVE_PLATFORMS: [&str; 5] = ["ios", "android", "macos", "windows", "linux"];

/// Writes a new application workspace, depending on the published SDK.
///
/// # Errors
///
/// Returns an error for an invalid package name or an existing template file.
pub fn run(path: &str, platform: Platform, name: Option<&str>) -> Result<()> {
	run_with_sdk(path, platform, name, None)
}

/// Writes a new workspace, optionally depending on a local SDK checkout.
///
/// Standard workspaces enable all six platforms, with `platform` choosing
/// the default build destination. HarmonyOS uses its experimental standalone
/// starter. Existing files are checked before writing any part of the scaffold.
///
/// # Errors
///
/// Returns an error for an invalid package name, SDK checkout, or conflicting
/// file.
pub fn run_with_sdk(
	path: &str,
	platform: Platform,
	name: Option<&str>,
	sdk: Option<&str>,
) -> Result<()> {
	if platform == Platform::Harmonyos {
		return crate::native::scaffold::run_for_target(
			path,
			name,
			sdk,
			Some(NativeTarget::Harmonyos),
		);
	}

	let sdk = sdk.map(resolve_sdk).transpose()?;
	std::fs::create_dir_all(path).with_context(|| format!("could not create {path}"))?;
	let dir = std::fs::canonicalize(path).with_context(|| format!("could not resolve {path}"))?;
	let dir = Utf8PathBuf::from_path_buf(dir)
		.map_err(|dir| anyhow::anyhow!("{} is not UTF-8", dir.display()))?;
	let name = name
		.or_else(|| dir.file_name())
		.context("the project directory has no name")?;
	ensure_package_name(name)?;
	let platform = concrete_platform(platform);
	let config = LeptoskitConfig {
		app: AppConfig {
			name: name.to_owned(),
			default_platform: platform,
			render: RenderMode::Ssr,
			..AppConfig::default()
		},
		build: BuildConfig {
			client_package: Some(format!("{name}_app")),
			..BuildConfig::default()
		},
		native: NativeConfig {
			package_id: leptoskit_config::default_package_id(name),
			..NativeConfig::default()
		},
		..LeptoskitConfig::default()
	};
	let files = template(&config, sdk.as_deref())?;

	for (relative, _) in &files {
		let path = dir.join(relative);
		preflight_file(&dir, &path)?;
	}

	for (relative, contents) in &files {
		write_new_bytes(&dir.join(relative), contents)?;
	}

	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		std::fs::set_permissions(
			dir.join("android/gradlew"),
			std::fs::Permissions::from_mode(0o755),
		)?;
	}

	println!("created {dir} (all platforms; default: {platform})");
	println!("next: cd {dir} && {} build", crate::cli::invoked_name());
	Ok(())
}

/// Rejects existing files and redirected parents before writing the workspace.
fn preflight_file(root: &Utf8Path, path: &Utf8Path) -> Result<()> {
	for entry in path.ancestors().take_while(|entry| *entry != root) {
		let metadata = match std::fs::symlink_metadata(entry) {
			Ok(metadata) => metadata,
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
			Err(error) => return Err(error).with_context(|| format!("could not inspect {entry}")),
		};
		anyhow::ensure!(entry != path, "{path} already exists");
		anyhow::ensure!(
			metadata.is_dir() && !metadata.file_type().is_symlink(),
			"{entry} must be an ordinary directory"
		);
	}

	Ok(())
}

/// Converts legacy build selectors into a concrete application destination.
fn concrete_platform(platform: Platform) -> Platform {
	if platform != Platform::Native {
		return platform;
	}

	match std::env::consts::OS {
		"macos" => Platform::Macos,
		"windows" => Platform::Windows,
		_ => Platform::Linux,
	}
}

/// Creates a text file without replacing an existing destination.
pub(crate) fn write_new(path: &Utf8Path, contents: &str) -> Result<()> {
	write_new_bytes(path, contents.as_bytes())
}

/// Creates a template file, including bundled binary assets.
fn write_new_bytes(path: &Utf8Path, contents: &[u8]) -> Result<()> {
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent).with_context(|| format!("could not create {parent}"))?;
	}

	let mut file = std::fs::OpenOptions::new()
		.write(true)
		.create_new(true)
		.open(path)
		.with_context(|| format!("could not create {path}"))?;
	file.write_all(contents)
		.with_context(|| format!("could not write {path}"))
}

/// Rejects names Cargo would refuse as a package name.
pub(crate) fn ensure_package_name(name: &str) -> Result<()> {
	let mut characters = name.chars();
	let valid = characters
		.next()
		.is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
		&& characters.all(|character| {
			character.is_ascii_alphanumeric() || character == '_' || character == '-'
		});
	anyhow::ensure!(
		valid,
		"`{name}` is not a valid cargo package name: use ASCII letters, digits, `_`, and `-`, \
		 starting with a letter or `_` (or pass --name)"
	);
	Ok(())
}

/// Resolves the SDK once; each dependency below also checks its crate manifest.
pub(crate) fn resolve_sdk(sdk: &str) -> Result<Utf8PathBuf> {
	let root = std::fs::canonicalize(sdk).context("resolve --sdk-path")?;
	let root = Utf8PathBuf::from_path_buf(root)
		.map_err(|root| anyhow::anyhow!("{} is not UTF-8", root.display()))?;
	anyhow::ensure!(
		root.join("crates/leptoskit/Cargo.toml").is_file(),
		"--sdk-path {root} has no crates/leptoskit"
	);
	Ok(root)
}

/// A pinned SDK dependency with an optional local path and feature list.
pub(crate) fn sdk_dependency(
	package: &str,
	sdk: Option<&Utf8Path>,
	features: &[&str],
) -> Result<String> {
	let version = env!("CARGO_PKG_VERSION");
	let mut dependency = format!("{{ version = \"={version}\", default-features = false");

	if let Some(sdk) = sdk {
		let path = sdk.join("crates").join(package);
		anyhow::ensure!(
			path.join("Cargo.toml").is_file(),
			"--sdk-path lacks {package}"
		);
		write!(dependency, ", path = {}", toml::Value::from(path.as_str()))?;
	}

	if !features.is_empty() {
		write!(dependency, ", features = {features:?}")?;
	}

	dependency.push_str(" }");
	Ok(dependency)
}

/// Assembles every template before creating any destination files.
fn template(config: &LeptoskitConfig, sdk: Option<&Utf8Path>) -> Result<Vec<(String, Vec<u8>)>> {
	let name = &config.app.name;
	let app = format!("{name}_app");
	let backend = format!("{name}_backend");
	let shared = format!("{name}_shared");
	let crate_name = name.replace('-', "_");
	let expand = |source: &str| {
		source
			.replace("__NAME__", name)
			.replace("__CRATE__", &crate_name)
	};
	let mut files = vec![
		(
			leptoskit_config::CONFIG_FILE.to_owned(),
			config.to_toml_string().context("serialize config")?,
		),
		(
			".gitignore".into(),
			"/target\n/dist\n/.leptoskit\n/android/.gradle\n/android/**/build\n/android/local.\
			 properties\n"
				.into(),
		),
		("Cargo.toml".into(), server_manifest(name)),
		(
			"src/main.rs".into(),
			expand(include_str!("new_project/templates/server.rs")),
		),
		(format!("{app}/Cargo.toml"), app_manifest(name, sdk)?),
		(
			format!("{app}/build.rs"),
			"fn main() -> Result<(), leptoskit_build::Error> \
			 {\n\tleptoskit_build::platforms()\n}\n"
				.into(),
		),
		(
			format!("{app}/src/lib.rs"),
			expand(include_str!("new_project/templates/app.rs")),
		),
		(
			format!("{app}/src/routes.rs"),
			"//! App routes with automatic platform overrides.\n\nleptoskit::platform_module!(pub \
			 mod home = \"routes/home\");\n"
				.into(),
		),
		(
			format!("{app}/src/routes/home.rs"),
			include_str!("new_project/templates/home.rs").into(),
		),
		(
			format!("{app}/src/components/mod.rs"),
			"//! Reusable application components shared by all platforms.\n".into(),
		),
		(
			format!("{app}/src/services/mod.rs"),
			include_str!("new_project/templates/services.rs").into(),
		),
		(
			format!("{backend}/Cargo.toml"),
			format!(
				"[package]\nname = \"{backend}\"\nversion = \"0.1.0\"\nedition = \
				 \"2024\"\npublish = false\n\n[dependencies]\naxum = \"0.8\"\n{shared} = {{ path \
				 = \"../{shared}\" }}\n"
			),
		),
		(
			format!("{backend}/src/lib.rs"),
			include_str!("new_project/templates/backend.rs").into(),
		),
		(
			format!("{backend}/src/rpc/mod.rs"),
			expand(include_str!("new_project/templates/rpc.rs")),
		),
		(
			format!("{shared}/Cargo.toml"),
			format!(
				"[package]\nname = \"{shared}\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish \
				 = false\n\n[dependencies]\nserde = {{ version = \"1\", features = [\"derive\"] \
				 }}\n"
			),
		),
		(
			format!("{shared}/src/lib.rs"),
			include_str!("new_project/templates/shared.rs").into(),
		),
		("web/index.html".into(), index_html(name)),
		(
			"assets/app.css".into(),
			include_str!("new_project/templates/app.css").into(),
		),
		("README.md".into(), readme(name)),
	];
	files.extend(platform_files(config, sdk)?);
	let mut files: Vec<_> = files
		.into_iter()
		.map(|(path, source)| (path, source.into_bytes()))
		.collect();
	files.push((
		"android/gradle/wrapper/gradle-wrapper.jar".into(),
		include_bytes!("native/templates/gradle/gradle-wrapper.jar").to_vec(),
	));
	Ok(files)
}

/// The root SSR package owns workspace composition and imports the shared app.
fn server_manifest(name: &str) -> String {
	format!(
		r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"
publish = false

[workspace]
resolver = "2"
members = ["{name}_app", "{name}_backend", "{name}_shared", "ios", "android", "macos", "windows", "linux"]
default-members = ["."]

[features]
default = ["ssr"]
ssr = ["{name}_app/web", "{name}_app/ssr", "leptos/ssr"]

[dependencies]
{name}_app = {{ path = "{name}_app", default-features = false }}
{name}_backend = {{ path = "{name}_backend" }}
axum = "0.8"
leptos = {{ version = "0.8", default-features = false }}
leptos_axum = "0.8"
tokio = {{ version = "1", features = ["macros", "rt-multi-thread", "net"] }}
tower-http = {{ version = "0.6", features = ["fs"] }}

[profile.release]
lto = "thin"
codegen-units = 1

# Keep host proc-macros loadable on macOS 27 (rust-lang/rust#157750).
[profile.release.build-override]
strip = "none"
"#
	)
}

/// One application library supplies SSR, browser Wasm, and native Wasm exports.
fn app_manifest(name: &str, sdk: Option<&Utf8Path>) -> Result<String> {
	let dependency = sdk_dependency("leptoskit", sdk, &[])?;
	let build_dependency = sdk_dependency("leptoskit_build", sdk, &[])?;
	let wasm_bindgen = crate::wasm::embedded_wasm_bindgen();
	Ok(format!(
		r#"[package]
name = "{name}_app"
version = "0.1.0"
edition = "2024"
publish = false

[lib]
crate-type = ["rlib", "cdylib"]

[features]
default = []
web = ["leptoskit/web", "dep:leptos", "dep:wasm-bindgen"]
native = ["leptoskit/guest"]
ios = ["native", "leptoskit/ios"]
android = ["native", "leptoskit/android"]
macos = ["native", "leptoskit/macos"]
windows = ["native", "leptoskit/windows"]
linux = ["native", "leptoskit/linux"]
csr = ["web", "leptos/csr"]
hydrate = ["web", "leptos/hydrate"]
ssr = ["web", "leptos/ssr"]

[dependencies]
{name}_shared = {{ path = "../{name}_shared" }}
leptoskit = {dependency}
leptos = {{ version = "0.8", default-features = false, optional = true }}
# The embedded wasm-bindgen CLI accepts exactly its matching library version.
wasm-bindgen = {{ version = "={wasm_bindgen}", optional = true }}

[build-dependencies]
leptoskit_build = {build_dependency}
"#
	))
}

/// Platform folders contain app-owned native integration and host manifests.
fn platform_files(
	config: &LeptoskitConfig,
	sdk: Option<&Utf8Path>,
) -> Result<Vec<(String, String)>> {
	let name = &config.app.name;
	let package = &config.native.package_id;
	let expand = |source: &str| {
		source
			.replace("__PACKAGE__", package)
			.replace("__NAME__", name)
			.replace("__VERSION__", "0.1.0")
	};
	let mut files = vec![
		(
			"ios/App.swift".into(),
			include_str!("native/templates/App.swift").into(),
		),
		(
			"ios/Info.plist".into(),
			expand(include_str!("native/templates/Info.plist")),
		),
		(
			"ios/AppUITests.swift".into(),
			include_str!("new_project/templates/AppUITests.swift").into(),
		),
		(
			"macos/App.swift".into(),
			include_str!("native/templates/MacApp.swift").into(),
		),
		(
			"macos/Info.plist".into(),
			expand(include_str!("native/templates/MacInfo.plist")),
		),
		(
			"android/gradlew".into(),
			include_str!("native/templates/gradle/gradlew").into(),
		),
		(
			"android/gradlew.bat".into(),
			include_str!("native/templates/gradle/gradlew.bat").into(),
		),
		(
			"android/gradle/wrapper/gradle-wrapper.properties".into(),
			include_str!("native/templates/gradle/gradle-wrapper.properties").into(),
		),
		(
			"android/gradle/LICENSE".into(),
			include_str!("native/templates/gradle/LICENSE").into(),
		),
		(
			"android/settings.gradle.kts".into(),
			"pluginManagement { repositories { google(); mavenCentral(); gradlePluginPortal() } \
			 }\ndependencyResolutionManagement { \
			 repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS); repositories { \
			 google(); mavenCentral() } }\nrootProject.name = \
			 \"LeptosKitApp\"\ninclude(\":app\")\n"
				.into(),
		),
		(
			"android/build.gradle.kts".into(),
			"plugins { id(\"com.android.application\") version \"9.0.1\" apply false }\n".into(),
		),
		(
			"android/gradle.properties".into(),
			"org.gradle.jvmargs=-Xmx2g\norg.gradle.workers.max=2\nandroid.useAndroidX=true\n"
				.into(),
		),
		(
			"android/app/build.gradle.kts".into(),
			expand(include_str!("native/templates/build.gradle.kts"))
				.replace("../../../.leptoskit", "../../.leptoskit")
				.replace(".leptoskit/mobile", ".leptoskit/android/mobile"),
		),
		(
			// Release minification renames by default; the JNI library calls
			// the bridge's classes by their exact names.
			"android/app/proguard-rules.pro".into(),
			"-keep class dev.leptoskit.bridge.** { *; }\n".into(),
		),
		(
			"android/app/src/main/AndroidManifest.xml".into(),
			expand(include_str!("native/templates/AndroidManifest.xml")),
		),
		(
			"android/app/src/main/res/values/styles.xml".into(),
			include_str!("native/templates/styles.xml").into(),
		),
		(
			"android/app/src/main/java/MainActivity.kt".into(),
			include_str!("native/templates/MainActivity.kt")
				.replace("dev.leptoskit.generated", package),
		),
		(
			"android/app/src/androidTest/java/NativeAppTest.kt".into(),
			include_str!("new_project/templates/NativeAppTest.kt")
				.replace("dev.leptoskit.generated", package),
		),
		(
			"windows/src/main.rs".into(),
			expand(include_str!("native/templates/WindowsMain.rs")),
		),
		(
			"linux/src/main.rs".into(),
			expand(include_str!("native/templates/LinuxMain.rs")),
		),
		(
			"windows/src/runtime.rs".into(),
			include_str!("native/templates/DesktopRuntime.rs").into(),
		),
		(
			"linux/src/runtime.rs".into(),
			include_str!("native/templates/DesktopRuntime.rs").into(),
		),
		(
			"windows/build.rs".into(),
			include_str!("native/templates/WindowsBuild.rs").into(),
		),
		(
			"windows/app.manifest".into(),
			include_str!("native/templates/WindowsManifest.xml").into(),
		),
	];

	for platform in NATIVE_PLATFORMS {
		files.push((
			format!("{platform}/Cargo.toml"),
			host_manifest(name, platform, sdk)?,
		));
		files.push((
			format!("{platform}/src/lib.rs"),
			format!(
				"//! Platform host registration generated from this runner's \
				 dependencies.\n#[cfg(target_os = \
				 \"{platform}\")]\ninclude!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \
				 \"/../.leptoskit/{platform}/registrant.rs\"));\n"
			),
		));
	}

	Ok(files)
}

/// OS dependencies are isolated in runner crates and never enter the Wasm app.
fn host_manifest(name: &str, platform: &str, sdk: Option<&Utf8Path>) -> Result<String> {
	let ffi = sdk_dependency("leptoskit_bridge_ffi", sdk, &["app"])?;
	let mut manifest = format!(
		"[package]\nname = \"{name}_{platform}\"\nversion = \"0.1.0\"\nedition = \
		 \"2024\"\npublish = false\n\n[lib]\nname = \"leptoskit_app\"\ncrate-type = \
		 [\"staticlib\"]\n\n[target.'cfg(target_os = \
		 \"{platform}\")'.dependencies]\nleptoskit_bridge_ffi = {ffi}\n"
	);

	if matches!(platform, "windows" | "linux") {
		let feature = if platform == "linux" {
			"gtk"
		} else {
			"windows"
		};
		let dependency = sdk_dependency("leptoskit", sdk, &["wasm", feature])?;
		writeln!(manifest, "leptoskit = {dependency}")?;
		writeln!(
			manifest,
			"\n[[bin]]\nname = \"{}-{platform}\"\npath = \"src/main.rs\"\n",
			name.replace('_', "-")
		)?;
	}

	Ok(manifest)
}

/// The CSR document also documents the browser bootstrap for custom web hosts.
fn index_html(name: &str) -> String {
	let lib = format!("{}_app", name.replace('-', "_"));
	format!(
		r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <link rel="icon" href="data:," />
    <title>{name}</title>
    <link rel="stylesheet" href="/assets/app.css" />
  </head>
  <body>
    <script type="module">
      import init from "/pkg/{lib}.js";
      await init({{ module_or_path: "/pkg/{lib}_bg.wasm" }});
    </script>
  </body>
</html>
"#
	)
}

/// Explains source ownership, platform overrides, and normal build commands.
fn readme(name: &str) -> String {
	format!(
		r#"# {name}

Write the application once in `{name}_app`. The browser, web SSR server, and native runners render the same screen and counter behavior. `{name}_backend` implements RPC endpoints; `{name}_shared` holds their serializable contracts. The root server imports the app and backend, while application builds never import backend implementation.

```sh
leptoskit build
leptoskit dev
leptoskit build --platform ios
leptoskit build --platform android
leptoskit build --platform macos
leptoskit build --platform windows
leptoskit build --platform linux
```

All six destinations are enabled in `leptoskit.toml`. Each build activates exactly one platform feature in `{name}_app`; add optional dependencies to that feature when they support the app's actual compilation target. Native app code compiles to Wasm, so UIKit, JNI, and other OS dependencies belong in the corresponding runner's Cargo manifest or a native plugin.

`src/main.rs` starts the SSR server, serves browser files, and registers the backend's `GET /api/health` endpoint. Shared assets live in `assets/`. `web/index.html` supplies the document for CSR mode (`app.render = "csr"`); SSR uses the shared app's `shell` function.

## Platform overrides

`{name}_app/src/routes.rs` declares the home module once:

```rust
leptoskit::platform_module!(pub mod home = "routes/home");
```

The module path is relative to the app crate's `src/` directory. Add `routes/home.ios.rs`, `routes/home.android.rs`, `routes/home.web.rs`, `routes/home.macos.rs`, `routes/home.windows.rs`, or `routes/home.linux.rs` to replace `routes/home.rs` on that platform. Imports stay `crate::routes::home`. The shared file is the fallback; replacements must expose the same interface. Use this declaration for any module that needs platform overrides. The build helper watches additions and removals of override files.

`ios/`, `android/`, `macos/`, `windows/`, and `linux/` are app-owned platform runners. Install the matching SDK when building a destination. All generated registrants, SDK integration files, and build intermediates stay under `.leptoskit/<platform>/`, which is ignored by Git. Prepare a native target before opening its generated Xcode project or the app-owned `android/` Gradle project.

The starter requests no native plugin capabilities. Add native plugins to the runner's direct Cargo dependencies and grant them explicitly in `native.plugins`.
"#
	)
}

/// Stable root template files, independent of the chosen application name.
pub fn template_files() -> &'static [&'static str] {
	&[
		leptoskit_config::CONFIG_FILE,
		".gitignore",
		"Cargo.toml",
		"src/main.rs",
		"web/index.html",
		"assets/app.css",
		"README.md",
	]
}

/// Ensures the workspace contains its shared crates and every platform runner.
///
/// # Errors
///
/// Returns an error naming the first missing source file.
pub fn verify_scaffold(dir: &Path) -> Result<()> {
	for file in template_files() {
		anyhow::ensure!(dir.join(file).is_file(), "scaffold is missing {file}");
	}

	let config = LeptoskitConfig::load_from(dir)?;
	let name = config.app.name;

	for file in [
		format!("{name}_app/src/lib.rs"),
		format!("{name}_app/src/routes/home.rs"),
		format!("{name}_backend/src/rpc/mod.rs"),
		format!("{name}_shared/src/lib.rs"),
	] {
		anyhow::ensure!(dir.join(&file).is_file(), "scaffold is missing {file}");
	}

	for platform in NATIVE_PLATFORMS {
		let file = format!("{platform}/Cargo.toml");
		anyhow::ensure!(dir.join(&file).is_file(), "scaffold is missing {file}");
	}

	Ok(())
}
