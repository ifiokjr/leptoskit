//! Generated native hosts and dependency-driven plugin registration.

use std::path::Path;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;
use clap::Subcommand;
use clap::ValueEnum;

mod project;
mod registry;
pub mod scaffold;

/// Native host tooling. Generated files live in `.leptoskit`; app sources in
/// `hosts` remain user-owned.
#[derive(Debug, Clone, Subcommand)]
pub enum NativeCommand {
	/// Compiles the native Wasm guest and compatibility descriptor for code
	/// push.
	Bundle {
		#[arg(long, default_value = "dist")]
		out: String,
	},
	/// Regenerates host projects and registrants and compiles their Rust
	/// inputs.
	Prepare {
		#[arg(value_enum)]
		target: Target,
		#[arg(long)]
		release: bool,
	},
	/// Produces an iOS application bundle or an Android APK.
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
	IosSimulator,
	Ios,
	Android,
	/// Android x86-64 devices and hosted CI emulators.
	#[value(name = "android-x86_64")]
	AndroidX86_64,
}

impl Target {
	fn rust(self) -> &'static str {
		match self {
			Self::IosSimulator if cfg!(target_arch = "x86_64") => "x86_64-apple-ios",
			Self::IosSimulator => "aarch64-apple-ios-sim",
			Self::Ios => "aarch64-apple-ios",
			Self::Android => "aarch64-linux-android",
			Self::AndroidX86_64 => "x86_64-linux-android",
		}
	}
}

/// Runs native generation and optionally the platform build tool.
pub fn run(command: NativeCommand) -> Result<()> {
	let (target, release, build) = match command {
		NativeCommand::Bundle { out } => {
			let (config, root) = leptoskit_config::LeptoskitConfig::load()?;
			return bundle(&root, &config, &root.join(out));
		}
		NativeCommand::Prepare { target, release } => (target, release, false),
		NativeCommand::Build { target, release } => (target, release, true),
	};
	let (config, root) = leptoskit_config::LeptoskitConfig::load()?;
	if config.codepush.enabled {
		let server = &config.codepush.server;
		anyhow::ensure!(
			server.starts_with("https://")
				|| (!release && config.codepush.allow_http && server.starts_with("http://")),
			"native code push requires HTTPS; debug builds may opt into codepush.allow_http"
		);
	}
	let registry = registry::resolve(&root, &config, target.rust())?;
	project::generate(&root, &config, &registry, target, release)?;
	let profile = if release { "release" } else { "debug" };
	let bundle_dir = root.join(".leptoskit/bundle");
	bundle(&root, &config, &bundle_dir)?;
	let mut host = Command::new("cargo");
	host.current_dir(&root).args([
		"build",
		"--package",
		&config.app.name,
		"--target",
		target.rust(),
	]);
	if release {
		host.arg("--release");
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
		let bin = ndk.join(format!("toolchains/llvm/prebuilt/{host_os}/bin"));
		let clang = bin.join(format!("{architecture}-linux-android26-clang{suffix}"));
		host.env(
			format!(
				"CARGO_TARGET_{}_LINUX_ANDROID_LINKER",
				architecture.to_ascii_uppercase()
			),
			&clang,
		);
		// C dependencies of the host (ring, through the code push transport's
		// TLS) compile with cc-rs, which needs the NDK compiler and archiver
		// too.
		host.env(format!("CC_{architecture}_linux_android"), &clang)
			.env(
				format!("AR_{architecture}_linux_android"),
				bin.join(format!(
					"llvm-ar{}",
					if cfg!(target_os = "windows") {
						".exe"
					} else {
						""
					}
				)),
			);
	}
	run_tool(&mut host)?;
	let lib_dir = root.join(".leptoskit/lib");
	project::copy(
		&registry
			.target
			.join(target.rust())
			.join(profile)
			.join("libleptoskit_app.a"),
		&lib_dir.join("libleptoskit_bridge_ffi.a"),
	)?;
	let assets = root.join(".leptoskit/assets");
	project::copy(&bundle_dir.join("app.wasm"), &assets.join("app.wasm"))?;
	leptoskit_codepush::manifest::bundle_directory_as(
		&bundle_dir,
		&assets.join("app.lkb"),
		&config.app.name,
		&config.codepush.channel,
		config.app.version.clone(),
		0,
	)?;
	let update_config = serde_json::json!({
		"enabled": config.codepush.enabled, "app": config.app.name,
		"channel": config.codepush.channel, "server": config.codepush.server,
		"public_keys": config.codepush.public_keys, "runtime": runtime_descriptor(&config),
		"allow_http": config.codepush.allow_http && !release,
	});
	std::fs::write(
		assets.join("codepush.json"),
		serde_json::to_vec_pretty(&update_config)?,
	)?;
	if build {
		match target {
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
				run_tool(Command::new("xcodebuild").current_dir(&root).args([
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
					"CODE_SIGNING_ALLOWED=NO",
					architecture,
					"build",
				]))?;
				println!(
					"built application in {}/.leptoskit/ios/build/Build/Products",
					root.display()
				);
			}
			Target::Android | Target::AndroidX86_64 => {
				run_tool(
					Command::new(root.join(if cfg!(windows) {
						"hosts/android/gradlew.bat"
					} else {
						"hosts/android/gradlew"
					}))
					.current_dir(root.join("hosts/android"))
					.args([
						"--no-daemon",
						if release {
							"assembleRelease"
						} else {
							"assembleDebug"
						},
					]),
				)?;
				println!(
					"built APK in {}/hosts/android/app/build/outputs/apk",
					root.display()
				);
			}
		}
	}
	Ok(())
}

/// Builds the native ABI guest without a platform shell or JavaScript glue.
fn bundle(root: &Path, config: &leptoskit_config::LeptoskitConfig, out: &Path) -> Result<()> {
	anyhow::ensure!(
		!config.native.update_contract.trim().is_empty(),
		"native.update_contract cannot be empty"
	);
	let metadata = crate::wasm::cargo_metadata(root)?;
	let mut command = Command::new("cargo");
	command.current_dir(root).args([
		"build",
		"--package",
		&format!("{}_guest", config.app.name),
		"--target",
		"wasm32-unknown-unknown",
		"--release",
	]);
	run_tool(&mut command)?;
	let guest = metadata
		.target_directory
		.join("wasm32-unknown-unknown/release")
		.join(format!("{}_guest.wasm", config.app.name.replace('-', "_")));
	std::fs::create_dir_all(out)?;
	project::copy(&guest, &out.join("app.wasm"))?;
	std::fs::write(
		out.join("runtime.json"),
		serde_json::to_vec_pretty(&runtime_descriptor(config))?,
	)?;
	println!("native Wasm bundle: {}", out.display());
	Ok(())
}

/// The ordinary bundled file makes compatibility part of the signed content.
fn runtime_descriptor(config: &leptoskit_config::LeptoskitConfig) -> serde_json::Value {
	serde_json::json!({
		"abi": leptoskit_wasm::ABI_VERSION,
		"shell": config.native.update_contract,
		"plugins": config.native.plugins,
	})
}

fn run_tool(command: &mut Command) -> Result<()> {
	let status = command
		.status()
		.with_context(|| format!("could not execute {command:?}"))?;
	anyhow::ensure!(status.success(), "{command:?} failed: {status}");
	Ok(())
}

fn android_ndk() -> Result<std::path::PathBuf> {
	if let Some(path) = std::env::var_os("ANDROID_NDK_HOME") {
		return Ok(path.into());
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
	versions.pop().context("no Android NDK is installed")
}
