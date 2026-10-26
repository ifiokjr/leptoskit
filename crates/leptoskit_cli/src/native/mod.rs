//! Generated native hosts and dependency-driven plugin registration.

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;
use clap::Subcommand;
use clap::ValueEnum;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Optimize;
use leptoskit_config::Platform;
use sha2::Digest;
use sha2::Sha256;

pub(crate) mod config;
mod desktop;
mod harmonyos;
mod lock;
mod manifest;
mod project;
mod registry;
mod rust_desktop;
pub mod scaffold;
mod size;

/// Native host tooling. Generated files live in `.leptoskit`; platform
/// directories contain app-owned integration sources.
#[derive(Debug, Clone, Subcommand)]
pub enum NativeCommand {
	/// Compiles the native Wasm guest and compatibility descriptor for code
	/// push.
	Bundle {
		#[arg(long, default_value = "dist")]
		out: String,
		/// Destination whose platform feature the bundle enables.
		#[arg(long, value_enum)]
		target: Option<Target>,
	},
	/// Regenerates host projects and registrants and compiles their Rust
	/// inputs.
	Prepare {
		#[arg(value_enum)]
		target: Target,
		#[arg(long)]
		release: bool,
	},
	/// Produces an Apple, Windows, or Linux application, Android APK, or
	/// HarmonyOS HAP.
	Build {
		#[arg(value_enum)]
		target: Target,
		#[arg(long)]
		release: bool,
	},
}

/// The native compilation target, including simulator architecture.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Target {
	/// A native AppKit application for this Mac's architecture.
	Macos,
	IosSimulator,
	Ios,
	Android,
	/// Android x86-64 devices and hosted CI emulators.
	#[value(name = "android-x86_64")]
	AndroidX86_64,
	/// HarmonyOS arm64 devices and arm64 emulators (DevEco Studio).
	Harmonyos,
	/// HarmonyOS x86-64 emulators.
	#[value(name = "harmonyos-x86_64")]
	HarmonyosX86_64,
	/// A native Windows application for this machine's architecture.
	Windows,
	/// A native GTK application for this machine's architecture.
	Linux,
}

impl Target {
	fn rust(self) -> &'static str {
		match self {
			Self::Macos if cfg!(target_arch = "x86_64") => "x86_64-apple-darwin",
			Self::Macos => "aarch64-apple-darwin",
			Self::IosSimulator if cfg!(target_arch = "x86_64") => "x86_64-apple-ios",
			Self::IosSimulator => "aarch64-apple-ios-sim",
			Self::Ios => "aarch64-apple-ios",
			Self::Android => "aarch64-linux-android",
			Self::AndroidX86_64 => "x86_64-linux-android",
			Self::Harmonyos => "aarch64-unknown-linux-ohos",
			Self::HarmonyosX86_64 => "x86_64-unknown-linux-ohos",
			Self::Windows if cfg!(target_arch = "aarch64") => "aarch64-pc-windows-msvc",
			Self::Windows => "x86_64-pc-windows-msvc",
			Self::Linux if cfg!(target_arch = "aarch64") => "aarch64-unknown-linux-gnu",
			Self::Linux => "x86_64-unknown-linux-gnu",
		}
	}

	pub(super) fn platform(self) -> Platform {
		match self {
			Self::Macos => Platform::Macos,
			Self::Ios | Self::IosSimulator => Platform::Ios,
			Self::Android | Self::AndroidX86_64 => Platform::Android,
			Self::Harmonyos | Self::HarmonyosX86_64 => Platform::Harmonyos,
			Self::Windows => Platform::Windows,
			Self::Linux => Platform::Linux,
		}
	}

	pub(crate) fn from_platform(platform: Platform) -> Result<Self> {
		match platform {
			Platform::Ios => Ok(Self::Ios),
			Platform::Android => Ok(Self::Android),
			Platform::Macos => Ok(Self::Macos),
			Platform::Windows => Ok(Self::Windows),
			Platform::Linux => Ok(Self::Linux),
			Platform::Harmonyos => Ok(Self::Harmonyos),
			_ => anyhow::bail!("{platform} is not a concrete native platform"),
		}
	}
}

/// App-owned platform integration, with support for existing native projects.
pub(super) fn host_dir(root: &Path, target: Target) -> PathBuf {
	let platform = target.platform().to_string();
	let directory = root.join(&platform);

	if directory.join("Cargo.toml").is_file() {
		return directory;
	}

	root.join("hosts").join(platform)
}

/// Platform-local generated state prevents one destination replacing another.
pub(super) fn generated_dir(root: &Path, target: Target) -> PathBuf {
	let generated = root.join(".leptoskit");

	if root
		.join(target.platform().to_string())
		.join("Cargo.toml")
		.is_file()
	{
		return generated.join(target.platform().to_string());
	}

	generated
}

/// Runs native generation and optionally the platform build tool.
pub fn run(command: NativeCommand) -> Result<()> {
	let (target, release, build) = match command {
		NativeCommand::Bundle { out, target } => {
			let (config, root) = LeptoskitConfig::load()?;
			let target = target
				.map_or_else(
					|| {
						match config.app.default_platform {
							Platform::Native => Ok(legacy_target(&root, &config)),
							platform => Target::from_platform(platform),
						}
					},
					Ok,
				)
				.context(
					"native bundle requires --target when app.default_platform is web or server",
				)?;
			ensure_target_enabled(&config, target)?;
			return bundle(&root, &config, target, true, None, &root.join(out));
		}
		NativeCommand::Prepare { target, release } => (target, release, false),
		NativeCommand::Build { target, release } => (target, release, true),
	};
	let (config, root) = LeptoskitConfig::load()?;
	ensure_target_enabled(&config, target)?;
	anyhow::ensure!(
		!matches!(target, Target::Macos | Target::Ios | Target::IosSimulator)
			|| cfg!(target_os = "macos"),
		"Apple applications require macOS and the Xcode command-line tools"
	);
	anyhow::ensure!(
		!matches!(target, Target::Windows) || cfg!(target_os = "windows"),
		"Windows applications require Windows and the MSVC build tools"
	);
	anyhow::ensure!(
		!matches!(target, Target::Linux) || cfg!(target_os = "linux"),
		"Linux applications require Linux and GTK 4 development libraries"
	);
	let harmonyos = matches!(target, Target::Harmonyos | Target::HarmonyosX86_64);
	let ndk = if harmonyos {
		harmonyos::validate(&config)?;
		Some(harmonyos::ndk()?)
	} else {
		None
	};
	if config.codepush.enabled {
		let server = &config.codepush.server;
		anyhow::ensure!(
			server.starts_with("https://")
				|| (!release && config.codepush.allow_http && server.starts_with("http://")),
			"native code push requires HTTPS; debug builds may opt into codepush.allow_http"
		);
	}
	let _guard = lock::acquire(&root)?;
	crate::routes::sync(&root, &config)?;
	crate::l10n::sync(&root, &config)?;
	let registry = registry::resolve(&root, &config, target)?;
	let build_dir = native_build_directory(&root, &registry.target, target.rust())?;
	project::generate(&root, &config, &registry, target, release)?;
	let profile = profile_directory(&config, true, release);
	let generated = generated_dir(&root, target);
	let bundle_dir = generated.join("bundle");
	bundle(
		&root,
		&config,
		target,
		release,
		Some(&build_dir),
		&bundle_dir,
	)?;
	if matches!(target, Target::Linux | Target::Windows) {
		write_assets(&root, &config, target, &bundle_dir, release)?;
		rust_desktop::build(&root, &config, target, release)?;

		return Ok(());
	}
	let mut host = Command::new("cargo");
	host.current_dir(&root).args([
		"build",
		"--package",
		&registry.package,
		"--lib",
		"--target",
		target.rust(),
	]);
	host.arg("--target-dir").arg(&build_dir);
	if release {
		// A debug build takes cargo's default profile; only release builds
		// pick up `bundle.optimize`'s profile.
		let (_, host_arguments) = release_arguments(&config, true);
		host.args(&host_arguments);
	}
	if let Some(ndk) = &ndk {
		harmonyos::configure_cargo(&mut host, ndk, target);
	}
	if !registry.host_features.is_empty() {
		host.args(["--features", &registry.host_features.join(",")]);
	}
	if matches!(target, Target::Android | Target::AndroidX86_64) {
		let ndk = android_ndk()?;
		let architecture = if matches!(target, Target::Android) {
			"aarch64"
		} else {
			"x86_64"
		};
		let host_os = if cfg!(target_os = "macos") {
			"darwin-x86_64"
		} else if cfg!(target_os = "windows") {
			"windows-x86_64"
		} else {
			"linux-x86_64"
		};
		let suffix = if cfg!(target_os = "windows") {
			".cmd"
		} else {
			""
		};
		let tools = ndk.join(format!("toolchains/llvm/prebuilt/{host_os}/bin"));
		let compiler = tools.join(format!("{architecture}-linux-android26-clang{suffix}"));
		anyhow::ensure!(
			compiler.is_file(),
			"selected NDK does not contain a compiler for this host: {}",
			compiler.display()
		);
		project::write(
			&root.join(".leptoskit/android/ndk-path.txt"),
			&gradle_ndk_path(&ndk)?,
		)?;
		// C dependencies of the host (ring, through the code push transport's
		// TLS) compile with cc-rs, which needs the NDK compiler and archiver
		// too.
		let target_key = target.rust().replace('-', "_");
		host.env(
			format!(
				"CARGO_TARGET_{}_LINUX_ANDROID_LINKER",
				architecture.to_ascii_uppercase()
			),
			&compiler,
		)
		.env(format!("CC_{target_key}"), compiler)
		.env(
			format!("CXX_{target_key}"),
			tools.join(format!("{architecture}-linux-android26-clang++{suffix}")),
		)
		.env(
			format!("AR_{target_key}"),
			tools.join(if cfg!(windows) {
				"llvm-ar.exe"
			} else {
				"llvm-ar"
			}),
		);
	} else if matches!(target, Target::Ios | Target::IosSimulator | Target::Macos) {
		for (key, value) in apple_cc_environment(target) {
			host.env(key, value);
		}
	}
	run_tool(&mut host)?;
	let lib_dir = generated.join("lib");
	project::copy(
		&build_dir
			.join(target.rust())
			.join(profile)
			.join("libleptoskit_app.a"),
		&lib_dir.join("libleptoskit_bridge_ffi.a"),
	)?;
	write_assets(&root, &config, target, &bundle_dir, release)?;
	if build {
		match target {
			Target::Macos => desktop::build(&root, &config, release)?,
			Target::Windows | Target::Linux => unreachable!("desktop hosts built above"),
			Target::Harmonyos | Target::HarmonyosX86_64 => {
				harmonyos::build(&root, release)?;
			}
			Target::Ios | Target::IosSimulator => {
				let sdk = if matches!(target, Target::Ios) {
					"iphoneos"
				} else {
					"iphonesimulator"
				};
				let architecture =
					if matches!(target, Target::IosSimulator) && cfg!(target_arch = "x86_64") {
						"ARCHS=x86_64"
					} else {
						"ARCHS=arm64"
					};
				let mut command = Command::new("xcodebuild");
				command.current_dir(&root).args([
					"-project",
					".leptoskit/ios/Host.xcodeproj",
					"-scheme",
					"App",
					"-sdk",
					sdk,
					"-configuration",
					if release { "Release" } else { "Debug" },
					"-derivedDataPath",
					".leptoskit/ios/build",
					architecture,
					"build",
				]);
				if matches!(target, Target::IosSimulator) {
					// Simulator Keychain access needs an ad-hoc signed app.
					// Disabling signing produces a launchable app without this
					// capability.
					command.args(["CODE_SIGNING_ALLOWED=YES", "CODE_SIGN_IDENTITY=-"]);
				} else {
					command.arg("CODE_SIGNING_ALLOWED=NO");
				}
				run_tool(&mut command)?;
			}
			Target::Android | Target::AndroidX86_64 => {
				let android = host_dir(&root, target);
				run_tool(
					Command::new(android.join(if cfg!(windows) {
						"gradlew.bat"
					} else {
						"gradlew"
					}))
					.current_dir(&android)
					.args([
						"--no-daemon",
						if release {
							"assembleRelease"
						} else {
							"assembleDebug"
						},
					]),
				)?;
			}
		}
		report_application_size(&root, target, release);
	}
	Ok(())
}

/// The inferred destination used by pre-universal native projects.
pub(crate) fn legacy_target(root: &Path, config: &LeptoskitConfig) -> Target {
	use leptoskit_config::NativeTarget;

	match config.native.targets.as_slice() {
		[NativeTarget::Harmonyos] => Target::Harmonyos,
		[NativeTarget::Macos] => Target::Macos,
		[NativeTarget::Ios] => Target::IosSimulator,
		[NativeTarget::Android] => Target::Android,
		[NativeTarget::Windows] => Target::Windows,
		[NativeTarget::Linux] => Target::Linux,
		_ if cfg!(target_os = "macos") && host_dir(root, Target::Macos).is_dir() => Target::Macos,
		_ if cfg!(target_os = "macos") => Target::IosSimulator,
		_ => Target::Android,
	}
}

/// Prints the size of the artifact a finished platform build produced, so a
/// size change surfaces in the build that caused it. A missing artifact or a
/// measurement failure never fails the build: the report is information, not
/// a gate.
fn report_application_size(root: &Path, target: Target, release: bool) {
	let report = match target {
		Target::Macos => size::macos(root).map(Some),
		Target::Ios | Target::IosSimulator => size::ios(root, release),
		Target::Android | Target::AndroidX86_64 => size::android(root, release),
		// The desktop hosts and the harmonyos packaging report through their
		// own tools; a measurement for them is a follow-up.
		Target::Windows | Target::Linux | Target::Harmonyos | Target::HarmonyosX86_64 => Ok(None),
	};
	match report {
		Ok(Some(report)) => {
			println!("application size ({}):\n{}", report.label, report.render());
		}
		Ok(None) => println!("built; no artifact found to measure"),
		Err(error) => eprintln!("warning: could not measure the artifact: {error}"),
	}
}

/// The cc-rs compiler variables for an Apple target, pointing at the SDK's
/// own clang: C dependencies of the host (ring, through the code push
/// transport's TLS) compile with cc-rs, and a wrapped cc on PATH (nix, a
/// sysroot prefix) rejects Apple targets. Empty off macOS, where there is no
/// SDK to find.
fn apple_cc_environment(target: Target) -> Vec<(String, String)> {
	let sdk = match target {
		Target::Ios => "iphoneos",
		Target::IosSimulator => "iphonesimulator",
		_ => "macosx",
	};
	let target_key = target.rust().replace('-', "_");
	let mut environment = Vec::new();
	if let Some(compiler) = xcrun_find(sdk, "clang") {
		environment.push((format!("CC_{target_key}"), compiler.display().to_string()));
		if let Some(plusplus) = xcrun_find(sdk, "clang++") {
			environment.push((format!("CXX_{target_key}"), plusplus.display().to_string()));
		}
	}
	environment
}

/// Writes what the target's shell bundles from the built guest. Desktop gets
/// the raw Wasm. Mobile gets the guest inside its store container, which the
/// update store verifies, and the update configuration, but never the Wasm a
/// second time.
fn write_assets(
	root: &Path,
	config: &LeptoskitConfig,
	target: Target,
	bundle_dir: &Path,
	release: bool,
) -> Result<()> {
	let generated = generated_dir(root, target);
	if matches!(target, Target::Macos | Target::Windows | Target::Linux) {
		return project::copy(
			&bundle_dir.join("app.wasm"),
			&generated.join("assets/app.wasm"),
		);
	}
	if matches!(target, Target::Harmonyos | Target::HarmonyosX86_64) {
		return project::copy(
			&bundle_dir.join("app.wasm"),
			&root.join("hosts/harmonyos/entry/src/main/resources/rawfile/app.wasm"),
		);
	}
	let assets = generated.join("mobile");
	std::fs::create_dir_all(&assets)?;
	let channel = config::channel(root, &config.codepush.channel, target.platform());
	leptoskit_codepush::manifest::bundle_directory_as(
		bundle_dir,
		&assets.join("app.lkb"),
		&config.app.name,
		&channel,
		config.app.version.clone(),
		0,
	)?;
	let update_config = serde_json::json!({
		"enabled": config.codepush.enabled, "app": config.app.name,
		"channel": channel, "server": config.codepush.server,
		"public_keys": config.codepush.public_keys, "runtime": runtime_descriptor(root, config, target),
		"allow_http": config.codepush.allow_http && !release,
	});
	std::fs::write(
		assets.join("codepush.json"),
		serde_json::to_vec_pretty(&update_config)?,
	)?;
	Ok(())
}

/// Builds the native ABI guest without a platform shell or JavaScript glue.
/// The guest always compiles for release — it is interpreted by wasmi, where a
/// debug build is both slower and larger — with [`bundle.optimize`] picking
/// the profile exactly as it does for the web client. `wasm-opt -Oz` runs
/// only when the enclosing build is a release build, like the web client, and
/// warns rather than fails when binaryen is missing: a machine that only
/// builds for devices still prepares, bundles, and debugs native apps.
fn bundle(
	root: &Path,
	config: &LeptoskitConfig,
	target: Target,
	release: bool,
	// Isolates same-named applications that share a Cargo target directory.
	target_dir: Option<&Path>,
	out: &Path,
) -> Result<()> {
	anyhow::ensure!(
		!config.native.update_contract.trim().is_empty(),
		"native.update_contract cannot be empty"
	);
	let metadata = crate::wasm::cargo_metadata(root)?;
	let portable = root
		.join(target.platform().to_string())
		.join("Cargo.toml")
		.is_file();
	let package = if portable {
		config
			.build
			.client_package
			.clone()
			.context("portable native builds require build.client_package")?
	} else {
		format!("{}_guest", config.app.name)
	};
	let client = crate::wasm::detect_client_with(Some(&package), None, &metadata)?
		.context("native application has no cdylib")?;
	let (plugins, context) = crate::build::plugin_context(config, target.platform(), release)?;
	let mut rustflags = config.build.rustflags.clone();
	rustflags.extend(context.rustflags);
	let mut options = crate::wasm::CargoOptions {
		release,
		features: context.cargo_features,
		rustflags,
		env: context.env,
		..Default::default()
	};

	if portable {
		let feature = target
			.platform()
			.feature()
			.context("native platform feature missing")?;
		let package = metadata
			.package(&package)
			.context("native application package missing")?;
		anyhow::ensure!(
			package.declares_feature(feature),
			"native app package `{}` must declare feature `{feature}`",
			package.name
		);
		options = options.with_platform(target.platform())?;
		options.target_directory = Some(generated_dir(root, target).join("target"));
	} else if let Some(target_dir) = target_dir {
		options.target_directory = Some(target_dir.to_path_buf());
	}

	// `bundle.optimize` reaches the guest exactly as it does the web client.
	let options = match (&config.bundle.cargo_profile, config.bundle.optimize) {
		(Some(profile), _) => options.with_profile(profile, Vec::new()),
		(None, Optimize::Smallest) => {
			options.with_profile(
				crate::wasm::SMALLEST_PROFILE,
				crate::wasm::smallest_profile_config(),
			)
		}
		(None, Optimize::Fast) => options,
	};

	let wasm = crate::wasm::compile_wasm(root, &client, &options)?;
	std::fs::create_dir_all(out)?;
	let module = out.join("app.wasm");
	project::copy(&wasm, &module)?;
	if release && config.bundle.wasm_opt {
		// Native builds cannot require binaryen the way the web client does:
		// `native prepare` and `native bundle` also run on machines that only
		// build for devices. Without it the guest ships unoptimized, with a
		// warning saying so; the size report makes the difference visible.
		match crate::wasm::wasm_opt_version() {
			Ok(_) => crate::wasm::optimize_guest(&module)?,
			Err(_) => {
				eprintln!(
					"warning: wasm-opt is not installed, so the guest wasm ships unoptimized; \
					 install binaryen for the smaller build"
				);
			}
		}
	}
	std::fs::write(
		out.join("runtime.json"),
		serde_json::to_vec_pretty(&runtime_descriptor(root, config, target))?,
	)?;
	plugins.after_build(&mut leptoskit_plugin::PluginPostBuildContext {
		config: config.clone(),
		platform: target.platform(),
		release,
		output_dir: out.to_path_buf(),
		wasm_bundle: Some(out.join("app.wasm")),
	})?;
	println!("native Wasm bundle: {}", out.display());
	Ok(())
}

/// The profile arguments a release cargo build takes: `--release`, or
/// `--profile <name>` with the `--config` entries defining the profile.
/// `host` picks between the two size profiles: the wasm one can
/// `panic = "abort"`, while the host's `catch_unwind` boundary cannot.
fn release_arguments(config: &LeptoskitConfig, host: bool) -> (String, Vec<String>) {
	match &config.bundle.cargo_profile {
		Some(profile) => (profile.clone(), vec!["--profile".into(), profile.clone()]),
		None if config.bundle.optimize == Optimize::Smallest && host => {
			(
				crate::wasm::HOST_SMALLEST_PROFILE.into(),
				profile_arguments(crate::wasm::HOST_SMALLEST_PROFILE),
			)
		}
		None if config.bundle.optimize == Optimize::Smallest => {
			(
				crate::wasm::SMALLEST_PROFILE.into(),
				profile_arguments(crate::wasm::SMALLEST_PROFILE),
			)
		}
		_ => ("release".into(), vec!["--release".into()]),
	}
}

/// `--profile <name>` plus the `--config` entries defining one of leptoskit's
/// size profiles, taken from the wasm module that owns them.
fn profile_arguments(profile: &str) -> Vec<String> {
	let config = if profile == crate::wasm::HOST_SMALLEST_PROFILE {
		crate::wasm::host_smallest_profile_config()
	} else {
		crate::wasm::smallest_profile_config()
	};
	let mut arguments = vec!["--profile".to_owned(), profile.to_owned()];
	for entry in config {
		arguments.push("--config".into());
		arguments.push(entry);
	}
	arguments
}

/// The directory under `target/<triple>` cargo writes a profile into: the
/// profile's own name, except the dev profile's `debug`.
fn profile_directory(config: &LeptoskitConfig, host: bool, release: bool) -> String {
	if !release {
		return "debug".into();
	}
	let (profile, _) = release_arguments(config, host);
	match profile.as_str() {
		"release" => "release".into(),
		name => name.to_owned(),
	}
}

/// The ordinary bundled file makes compatibility part of the signed content.
fn runtime_descriptor(root: &Path, config: &LeptoskitConfig, target: Target) -> serde_json::Value {
	let shell = config::shell(root, config, target.platform());

	serde_json::json!({
		"abi": leptoskit_wasm::ABI_VERSION,
		"shell": shell,
		"plugins": config.native.plugins,
	})
}

fn ensure_target_enabled(config: &LeptoskitConfig, target: Target) -> Result<()> {
	// Legacy HarmonyOS projects selected their only destination through
	// native.targets before app.platforms existed.
	if target.platform() == Platform::Harmonyos
		&& config.app.default_platform == Platform::Native
		&& config.native.targets == [leptoskit_config::NativeTarget::Harmonyos]
		&& config.app.platforms == Platform::APPLICATION
	{
		return Ok(());
	}

	config.ensure_platform_enabled(target.platform())?;

	Ok(())
}

fn native_build_directory(root: &Path, target_dir: &Path, target: &str) -> Result<PathBuf> {
	let root = root
		.canonicalize()
		.context("canonicalize native application directory")?;
	let identity = hex::encode(Sha256::digest(root.as_os_str().as_encoded_bytes()));
	Ok(target_dir
		.join("leptoskit-native")
		.join(identity)
		.join(target))
}

fn run_tool(command: &mut Command) -> Result<()> {
	let status = command
		.status()
		.with_context(|| format!("could not execute {command:?}"))?;
	anyhow::ensure!(status.success(), "{command:?} failed: {status}");
	Ok(())
}

/// The path of `tool` inside an SDK, from the `xcrun` on `PATH`: Xcode's own
/// compiler, not whatever `cc` resolves to on the surrounding machine.
fn xcrun_find(sdk: &str, tool: &str) -> Option<PathBuf> {
	xcrun_find_in(
		std::env::var_os("PATH").as_deref().unwrap_or_default(),
		sdk,
		tool,
	)
}

/// [`xcrun_find`] over an explicit `PATH`, so the lookup is testable without
/// mutating the environment.
fn xcrun_find_in(search: &std::ffi::OsStr, sdk: &str, tool: &str) -> Option<PathBuf> {
	let mut xcrun = None;
	for directory in std::env::split_paths(&search) {
		let candidate = directory.join("xcrun");
		if candidate.is_file() {
			xcrun = Some(candidate);
			break;
		}
	}
	let output = Command::new(xcrun?)
		.args(["--sdk", sdk, "--find", tool])
		.output()
		.ok()?;
	if !output.status.success() {
		return None;
	}
	let path = String::from_utf8(output.stdout).ok()?;
	let path = PathBuf::from(path.trim());
	path.is_file().then_some(path)
}

fn android_ndk() -> Result<PathBuf> {
	if let Some(path) = std::env::var_os("ANDROID_NDK_HOME") {
		return checked_ndk(Path::new(&path));
	}
	let sdk = std::env::var_os("ANDROID_HOME")
		.or_else(|| std::env::var_os("ANDROID_SDK_ROOT"))
		.context("set ANDROID_HOME and install an NDK, or set ANDROID_NDK_HOME")?;
	let mut versions = std::fs::read_dir(Path::new(&sdk).join("ndk"))?
		.map(|entry| entry.map(|entry| entry.path()))
		.collect::<std::io::Result<Vec<_>>>()?;
	versions.retain(|path| path.join("source.properties").is_file());
	versions.sort_by_cached_key(|path| {
		path.file_name()
			.and_then(|name| name.to_str())
			.unwrap_or_default()
			.split('.')
			.map(|part| part.parse::<u64>().unwrap_or(0))
			.collect::<Vec<_>>()
	});
	checked_ndk(&versions.pop().context("no Android NDK is installed")?)
}

fn checked_ndk(path: &Path) -> Result<PathBuf> {
	let path = path.canonicalize().context("resolve Android NDK path")?;
	anyhow::ensure!(
		path.join("source.properties").is_file(),
		"Android NDK is missing source.properties: {}",
		path.display()
	);
	Ok(path)
}

fn gradle_ndk_path(path: &Path) -> Result<String> {
	let path = path.to_str().context("Android NDK path must be UTF-8")?;
	// Rust canonicalization uses verbatim Windows prefixes; Gradle/Java expect
	// an ordinary drive or UNC path in their Android DSL.
	Ok(if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
		format!(r"\\{unc}")
	} else {
		path.strip_prefix(r"\\?\").unwrap_or(path).to_owned()
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn universal_platforms_isolate_generated_state_and_update_contracts() {
		let root =
			std::env::temp_dir().join(format!("leptoskit-platform-state-{}", std::process::id()));
		std::fs::create_dir_all(root.join("ios")).unwrap();
		std::fs::create_dir_all(root.join("android")).unwrap();
		std::fs::write(root.join("ios/Cargo.toml"), "").unwrap();
		std::fs::write(root.join("android/Cargo.toml"), "").unwrap();
		let mut config = LeptoskitConfig::default();

		assert_eq!(host_dir(&root, Target::IosSimulator), root.join("ios"));
		assert_ne!(
			generated_dir(&root, Target::Ios),
			generated_dir(&root, Target::Android)
		);
		assert_eq!(
			generated_dir(&root, Target::Ios),
			generated_dir(&root, Target::IosSimulator)
		);
		let ios: leptoskit_codepush::installed::RuntimeContract =
			serde_json::from_value(runtime_descriptor(&root, &config, Target::Ios)).unwrap();
		let android: leptoskit_codepush::installed::RuntimeContract =
			serde_json::from_value(runtime_descriptor(&root, &config, Target::Android)).unwrap();
		assert_ne!(ios.shell, android.shell);
		let bundle = root.join("bundle");
		std::fs::create_dir_all(&bundle).unwrap();
		std::fs::write(bundle.join("app.wasm"), b"\0asm\x01\0\0\0").unwrap();
		config.app.name = "platform_app".into();

		for target in [Target::Ios, Target::Android] {
			std::fs::write(
				bundle.join("runtime.json"),
				serde_json::to_vec(&runtime_descriptor(&root, &config, target)).unwrap(),
			)
			.unwrap();
			write_assets(&root, &config, target, &bundle, false).unwrap();
			let mobile = generated_dir(&root, target).join("mobile");
			let container = std::fs::read(mobile.join("app.lkb")).unwrap();
			let manifest = leptoskit_codepush::read_container(&container)
				.unwrap()
				.manifest;
			let updates: serde_json::Value =
				serde_json::from_slice(&std::fs::read(mobile.join("codepush.json")).unwrap())
					.unwrap();
			assert_eq!(
				manifest.channel,
				format!("production-{}", target.platform())
			);
			assert_eq!(updates["channel"].as_str(), Some(manifest.channel.as_str()));
		}

		config.app.platforms = vec![Platform::Web];
		assert!(ensure_target_enabled(&config, Target::Ios).is_err());
	}

	#[test]
	fn report_application_size_prints_the_artifact_it_found() {
		let root = std::env::temp_dir()
			.canonicalize()
			.unwrap()
			.join(format!("leptoskit-size-print-{}", std::process::id()));
		let contents = root.join(".leptoskit/macos/App.app/Contents/MacOS");
		std::fs::create_dir_all(&contents).unwrap();
		std::fs::write(contents.join("App"), vec![0_u8; 1024]).unwrap();
		report_application_size(&root, Target::Macos, true);
		// A target with no artifact to measure still reports cleanly.
		report_application_size(&root, Target::Android, true);
		report_application_size(&root, Target::Harmonyos, true);
		// An iOS build reports its application bundle; an unreadable
		// products directory surfaces as a warning, not a
		// failure.
		let app = root.join(".leptoskit/ios/build/Build/Products/Release-iphonesimulator/App.app");
		std::fs::create_dir_all(&app).unwrap();
		std::fs::write(app.join("App"), vec![0_u8; 512]).unwrap();
		report_application_size(&root, Target::IosSimulator, true);
		let blocked = root.join(".leptoskit/ios/build/Build/Products");
		std::fs::remove_dir_all(&blocked).unwrap();
		std::fs::write(&blocked, b"file").unwrap();
		report_application_size(&root, Target::IosSimulator, true);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn legacy_native_layouts_enforce_enabled_platforms_and_preserve_harmonyos_opt_in() {
		let mut config = LeptoskitConfig::default();
		config.app.default_platform = Platform::Native;
		config.app.platforms = vec![Platform::Web];
		assert!(ensure_target_enabled(&config, Target::Android).is_err());
		config.native.targets = vec![leptoskit_config::NativeTarget::Harmonyos];
		assert!(ensure_target_enabled(&config, Target::Harmonyos).is_err());
		config.app.platforms = Platform::APPLICATION.to_vec();
		ensure_target_enabled(&config, Target::Harmonyos).unwrap();
		config.app.default_platform = Platform::Web;
		assert!(ensure_target_enabled(&config, Target::Harmonyos).is_err());
	}

	#[test]
	fn apple_cc_environment_names_the_sdk_and_offers_nothing_without_one() {
		for target in [Target::Ios, Target::IosSimulator, Target::Macos] {
			// On macOS the SDK compilers resolve; elsewhere the build cannot
			// depend on Xcode, so the environment is simply empty.
			let environment = apple_cc_environment(target);
			if cfg!(target_os = "macos") {
				assert!(
					environment.iter().any(|(key, _)| {
						key.starts_with("CC_") && key.ends_with(&target.rust().replace('-', "_"))
					}),
					"the SDK compiler is named for {target:?}: {environment:?}"
				);
			} else {
				assert!(environment.is_empty(), "no SDK off macOS: {environment:?}");
			}
		}
	}

	#[test]
	fn release_arguments_follow_optimize_and_the_cargo_profile_escape_hatch() {
		let mut config = LeptoskitConfig::default();
		let (profile, arguments) = release_arguments(&config, true);
		assert_eq!(profile, "release");
		assert_eq!(arguments, vec!["--release".to_owned()]);

		config.bundle.optimize = Optimize::Smallest;
		let (profile, arguments) = release_arguments(&config, true);
		assert_eq!(profile, crate::wasm::HOST_SMALLEST_PROFILE);
		assert_eq!(arguments[0], "--profile");
		assert_eq!(arguments[1], profile);
		assert!(arguments.iter().any(|argument| {
			argument == "profile.leptoskit-host-smallest.build-override.strip = \"none\""
		}));
		assert!(
			!arguments
				.iter()
				.any(|argument| argument.contains("panic = \"abort\"")),
			"the host keeps unwinding for catch_unwind: {arguments:?}"
		);

		let (profile, arguments) = release_arguments(&config, false);
		assert_eq!(profile, crate::wasm::SMALLEST_PROFILE);
		assert!(
			arguments
				.iter()
				.any(|argument| argument == "profile.leptoskit-smallest.panic = \"abort\""),
			"the wasm guest may abort: {arguments:?}"
		);

		config.bundle.cargo_profile = Some("wasm-release".into());
		for host in [true, false] {
			let (profile, arguments) = release_arguments(&config, host);
			assert_eq!(profile, "wasm-release");
			assert_eq!(
				arguments,
				vec!["--profile".to_owned(), "wasm-release".to_owned()]
			);
		}
	}

	#[test]
	fn profile_directory_follows_the_build_and_its_profile() {
		let mut config = LeptoskitConfig::default();
		assert_eq!(profile_directory(&config, true, false), "debug");
		assert_eq!(profile_directory(&config, true, true), "release");
		config.bundle.optimize = Optimize::Smallest;
		assert_eq!(
			profile_directory(&config, true, true),
			crate::wasm::HOST_SMALLEST_PROFILE
		);
		config.bundle.cargo_profile = Some("wasm-release".into());
		assert_eq!(profile_directory(&config, true, true), "wasm-release");
	}

	#[test]
	fn same_named_cargo_apps_preserve_their_artifacts_in_a_shared_target_directory() {
		let temporary = tempfile::tempdir().unwrap();
		let root = temporary.path();
		let shared = root.join("shared-target");
		let mut executables = Vec::new();
		for name in ["first", "second"] {
			let app = root.join(name);
			std::fs::create_dir_all(app.join("src")).unwrap();
			std::fs::write(
				app.join("Cargo.toml"),
				"[package]\nname = \"same-name\"\nversion = \"0.1.0\"\nedition = \
				 \"2021\"\n[workspace]\n",
			)
			.unwrap();
			std::fs::write(
				app.join("src/main.rs"),
				format!("fn main() {{ println!({name:?}); }}"),
			)
			.unwrap();
			let destination = native_build_directory(&app, &shared, "host").unwrap();
			let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
				.current_dir(&app)
				.env("CARGO_TARGET_DIR", &shared)
				.env_remove("CARGO_BUILD_TARGET")
				.args(["build", "--offline", "--target-dir"])
				.arg(&destination)
				.output()
				.unwrap();
			assert!(
				output.status.success(),
				"{}",
				String::from_utf8_lossy(&output.stderr)
			);
			executables.push(
				destination
					.join("debug")
					.join(format!("same-name{}", std::env::consts::EXE_SUFFIX)),
			);
			assert_eq!(
				native_build_directory(&app.join("src/.."), &shared, "host").unwrap(),
				destination
			);
			assert_ne!(
				native_build_directory(&app, &shared, "another-target").unwrap(),
				destination
			);
		}
		// The first artifact is consumed only after the second same-named app
		// has built, reproducing the native host build/copy interleaving.
		for (executable, expected) in executables.iter().zip(["first", "second"]) {
			let output = Command::new(executable).output().unwrap();
			assert!(output.status.success());
			assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), expected);
		}
	}

	#[cfg(unix)]
	#[test]
	fn xcrun_find_resolves_through_a_fake_sdk() {
		use std::os::unix::fs::PermissionsExt as _;

		let root = std::env::temp_dir()
			.canonicalize()
			.unwrap()
			.join(format!("leptoskit-xcrun-find-{}", std::process::id()));
		std::fs::create_dir_all(&root).unwrap();
		std::fs::write(root.join("clang"), b"").unwrap();
		std::fs::write(
			root.join("xcrun"),
			format!("#!/bin/sh\nprintf '%s\\n' \"{}/clang\"\n", root.display()),
		)
		.unwrap();
		std::fs::set_permissions(root.join("xcrun"), std::fs::Permissions::from_mode(0o755))
			.unwrap();
		let found = xcrun_find_in(root.as_os_str(), "macosx", "clang");
		assert_eq!(found, Some(root.join("clang")));
		assert!(
			xcrun_find_in(std::ffi::OsStr::new("/nonexistent"), "macosx", "clang").is_none(),
			"no xcrun on the search path finds nothing"
		);
		std::fs::remove_dir_all(root).unwrap();
	}

	/// A directory alias: a symlink on Unix, a junction (which needs no
	/// privilege) on Windows.
	fn directory_alias(target: &Path, alias: &Path) {
		#[cfg(unix)]
		std::os::unix::fs::symlink(target, alias).unwrap();
		#[cfg(windows)]
		assert!(
			std::process::Command::new("cmd")
				.args(["/C", "mklink", "/J"])
				.arg(alias)
				.arg(target)
				.status()
				.unwrap()
				.success()
		);
	}

	#[cfg(any(unix, windows))]
	#[test]
	fn native_cache_identity_resolves_directory_aliases() {
		let temporary = tempfile::tempdir().unwrap();
		let root = temporary.path();
		let first = root.join("first");
		let second = root.join("second");
		std::fs::create_dir_all(&first).unwrap();
		std::fs::create_dir_all(&second).unwrap();
		let alias = root.join("alias");
		directory_alias(&first, &alias);
		let first_cache = native_build_directory(&first, root, "host").unwrap();
		assert_eq!(
			native_build_directory(&alias, root, "host").unwrap(),
			first_cache
		);
		assert_ne!(
			native_build_directory(&second, root, "host").unwrap(),
			first_cache
		);
	}

	// Each platform's own non-Unicode names: arbitrary bytes on Linux,
	// unpaired UTF-16 surrogates on Windows. APFS rejects non-UTF-8 names.
	#[cfg(any(target_os = "linux", windows))]
	#[test]
	fn native_cache_identity_preserves_non_utf8_paths() {
		let temporary = tempfile::tempdir().unwrap();
		let root = temporary.path();
		#[cfg(target_os = "linux")]
		let (first, second) = {
			use std::os::unix::ffi::OsStringExt;
			(
				root.join(std::ffi::OsString::from_vec(vec![b'a', 0xfe])),
				root.join(std::ffi::OsString::from_vec(vec![b'a', 0xff])),
			)
		};
		#[cfg(windows)]
		let (first, second) = {
			use std::os::windows::ffi::OsStringExt;
			(
				root.join(std::ffi::OsString::from_wide(&[u16::from(b'a'), 0xD800])),
				root.join(std::ffi::OsString::from_wide(&[u16::from(b'a'), 0xD801])),
			)
		};
		std::fs::create_dir_all(&first).unwrap();
		std::fs::create_dir_all(&second).unwrap();
		assert_ne!(
			native_build_directory(&first, root, "host").unwrap(),
			native_build_directory(&second, root, "host").unwrap()
		);
	}

	#[test]
	fn ndk_paths_are_canonical_and_require_an_installation_manifest() {
		let root = std::env::temp_dir().join(format!("leptoskit-ndk-path-{}", std::process::id()));
		std::fs::create_dir_all(root.join("child")).unwrap();
		assert!(checked_ndk(&root).is_err());
		std::fs::write(root.join("source.properties"), "Pkg.Revision = 28.0.0").unwrap();
		assert_eq!(
			checked_ndk(&root.join("child/..")).unwrap(),
			root.canonicalize().unwrap()
		);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn desktop_bundles_raw_wasm_and_mobile_only_the_verified_container() {
		// Generated paths refuse symlinks, and the macOS temporary directory is
		// one.
		let root = std::env::temp_dir()
			.canonicalize()
			.unwrap()
			.join(format!("leptoskit-assets-{}", std::process::id()));
		let bundle = root.join(".leptoskit/bundle");
		std::fs::create_dir_all(&bundle).unwrap();
		let wasm = b"\0asm\x01\0\0\0";
		std::fs::write(bundle.join("app.wasm"), wasm).unwrap();
		std::fs::write(bundle.join("runtime.json"), b"{}").unwrap();
		let mut config = LeptoskitConfig::default();
		config.app.name = "demo".into();
		config.codepush.enabled = true;
		config.codepush.allow_http = true;

		write_assets(&root, &config, Target::Android, &bundle, true).unwrap();
		let mobile = root.join(".leptoskit/mobile");
		let container = std::fs::read(mobile.join("app.lkb")).unwrap();
		assert_eq!(
			leptoskit_codepush::installed::entry_module(&container).unwrap(),
			wasm
		);
		let updates: serde_json::Value =
			serde_json::from_slice(&std::fs::read(mobile.join("codepush.json")).unwrap()).unwrap();
		assert_eq!(updates["enabled"], true);
		assert_eq!(updates["app"], "demo");
		assert_eq!(updates["channel"], "production");
		assert_eq!(
			updates["allow_http"], false,
			"release builds never allow HTTP"
		);
		assert!(!root.join(".leptoskit/assets/app.wasm").exists());

		write_assets(&root, &config, Target::Macos, &bundle, false).unwrap();
		assert_eq!(
			std::fs::read(root.join(".leptoskit/assets/app.wasm")).unwrap(),
			wasm
		);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn gradle_ndk_paths_preserve_spaces_and_normalize_windows_prefixes() {
		assert_eq!(
			gradle_ndk_path(Path::new("/opt/Android NDK")).unwrap(),
			"/opt/Android NDK"
		);
		assert_eq!(
			gradle_ndk_path(Path::new(r"\\?\C:\Android NDK")).unwrap(),
			r"C:\Android NDK"
		);
		assert_eq!(
			gradle_ndk_path(Path::new(r"\\?\UNC\server\Android NDK")).unwrap(),
			r"\\server\Android NDK"
		);
	}
}
