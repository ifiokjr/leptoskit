//! `leptoskit doctor`: checks the environment for the tooling the CLI runs.

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Result;
use leptoskit_config::ConfigError;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Platform;

use crate::ui::Outcome;
use crate::wasm::BindgenCompatibility;
use crate::wasm::WASM_TARGET;

/// One environment check.
struct Check {
	name: &'static str,
	/// Whether a failure stops builds (as opposed to some builds).
	required: bool,
	/// The detail line on success, or what is wrong.
	outcome: Result<String, String>,
}

/// The Android platform the generated Gradle project compiles against.
const ANDROID_PLATFORM: &str = "android-36";

/// Runs the checks and prints a report: the tools every build uses, then one
/// group per native platform the project targets.
///
/// # Errors
///
/// Returns an error when any required check fails, so scripts and CI can
/// gate on the exit status.
pub fn run(all: bool) -> Result<()> {
	// A malformed config is the project check's report, not a reason to
	// skip the rest of the tools.
	let project = LeptoskitConfig::load().ok();
	let mut checks = vec![
		Check {
			name: "cargo",
			required: true,
			outcome: version_of("cargo"),
		},
		Check {
			name: "rustc",
			required: true,
			outcome: version_of("rustc"),
		},
		Check {
			name: WASM_TARGET,
			required: true,
			outcome: rust_target_installed(WASM_TARGET),
		},
		Check {
			name: "wasm-opt",
			required: false,
			outcome: version_of("wasm-opt").map_err(|error| {
				format!("{error}; release builds need it unless `bundle.wasm_opt = false`")
			}),
		},
	];

	if let Some(check) = project_check() {
		checks.push(check);
	}

	if let Some(check) = extension_check() {
		checks.push(check);
	}

	if project
		.as_ref()
		.is_some_and(|(config, _)| config.extension.is_some())
	{
		for (name, path, hint) in [
			(
				"Firefox",
				crate::extension::browser::firefox(),
				"install Firefox or set LEPTOSKIT_FIREFOX",
			),
			(
				"geckodriver",
				crate::extension::browser::geckodriver(),
				"install geckodriver on PATH or set LEPTOSKIT_GECKODRIVER",
			),
		] {
			checks.push(Check {
				name,
				required: false,
				outcome: path
					.map(|path| path.display().to_string())
					.ok_or_else(|| hint.to_owned()),
			});
		}
	}

	if let Some(check) = devtools_check() {
		checks.push(check);
	}

	if let Some((config, root)) = &project
		&& let Some(check) = connections_check(config, root)
	{
		checks.push(check);
	}

	let config = project.as_ref().map(|(config, _)| config);

	for platform in selected_platforms(all, config) {
		checks.extend(match platform {
			Platform::Ios => ios_checks(project.as_ref()),
			Platform::Macos => {
				vec![Check {
					name: "macOS: swift",
					required: true,
					outcome: version_of("swift").map_err(|error| {
						format!("{error}; install Xcode or its command-line tools")
					}),
				}]
			}
			Platform::Android => android_checks(),
			Platform::Linux => {
				vec![
					Check {
						name: "Linux: pkg-config",
						required: true,
						outcome: version_of("pkg-config"),
					},
					Check {
						name: "Linux: GTK 4",
						required: true,
						outcome: gtk4_version(),
					},
				]
			}
			Platform::Harmonyos => harmonyos_checks(),
			_ => Vec::new(),
		});
	}

	crate::ui::heading("doctor", "the tools leptoskit builds with");
	let mut summary = crate::ui::Summary::default();
	let mut failed = 0;

	for check in &checks {
		let (outcome, details) = match &check.outcome {
			Ok(details) => (Outcome::Passed, details.as_str()),

			Err(problem) if check.required => {
				failed += 1;
				(Outcome::Failed, problem.as_str())
			}

			Err(problem) => (Outcome::Warned, problem.as_str()),
		};

		summary.row(outcome, check.name, details, None);
	}

	summary.print();
	let until = if failed == 1 {
		"it passes"
	} else {
		"they pass"
	};

	anyhow::ensure!(
		failed == 0,
		"{} failed; builds will not work until {until}",
		crate::ui::count(failed, "required check"),
	);
	Ok(())
}

/// The native platforms worth checking, in display order: the project's own
/// inside a project, or every platform this machine can build with `--all`.
/// HarmonyOS only appears when the project selects it, because its SDK is
/// rarely installed alongside the rest.
fn selected_platforms(all: bool, config: Option<&LeptoskitConfig>) -> Vec<Platform> {
	const ORDER: [Platform; 5] = [
		Platform::Ios,
		Platform::Macos,
		Platform::Android,
		Platform::Harmonyos,
		Platform::Linux,
	];

	if all {
		// Hoisted out of the arms: clippy reads a match whose arms are all
		// literal booleans as a `matches!`, and `cfg!` collapses to literals
		// on every platform but this one.
		let apple = cfg!(target_os = "macos");
		let linux = cfg!(target_os = "linux");
		return ORDER
			.into_iter()
			.filter(|platform| {
				match platform {
					Platform::Ios | Platform::Macos => apple,
					Platform::Linux => linux,
					Platform::Harmonyos => false,
					_ => true,
				}
			})
			.collect();
	}

	let Some(config) = config else {
		return Vec::new();
	};

	ORDER
		.into_iter()
		.filter(|platform| config.app.platforms.contains(platform))
		.collect()
}

fn ios_checks(project: Option<&(LeptoskitConfig, PathBuf)>) -> Vec<Check> {
	if !cfg!(target_os = "macos") {
		return vec![Check {
			name: "iOS: Xcode",
			required: true,
			outcome: Err("iOS applications require macOS and Xcode".to_owned()),
		}];
	}

	let mut checks = vec![
		Check {
			name: "iOS: Xcode",
			required: true,
			outcome: xcodebuild_version(),
		},
		Check {
			name: "iOS: simulator runtime",
			required: true,
			outcome: ios_simulator_runtime(),
		},
		Check {
			name: "iOS: aarch64-apple-ios",
			required: true,
			outcome: rust_target_installed("aarch64-apple-ios"),
		},
		Check {
			name: "iOS: aarch64-apple-ios-sim",
			required: true,
			outcome: rust_target_installed("aarch64-apple-ios-sim"),
		},
	];

	// CocoaPods is required only while a plugin declares pods; apps that use
	// Swift packages alone never need it.
	if let Some((config, root)) = project
		&& crate::native::ios_plugins_use_pods(root, config)
	{
		checks.push(Check {
			name: "iOS: CocoaPods",
			required: true,
			outcome: version_of("pod").map_err(|error| {
				format!("{error}; a plugin declares apple-pods, so builds run `pod install`")
			}),
		});
	}

	checks
}

/// An installed iOS simulator runtime, which `leptoskit dev` and simulator
/// builds run in.
fn ios_simulator_runtime() -> Result<String, String> {
	let output = Command::new("xcrun")
		.args(["simctl", "list", "runtimes"])
		.output()
		.map_err(|error| format!("could not run xcrun: {error}"))?;

	if !output.status.success() {
		return Err(format!(
			"`xcrun simctl list runtimes` exited with {}",
			output.status
		));
	}

	let runtimes = String::from_utf8_lossy(&output.stdout);

	match runtimes.lines().find(|line| line.contains("iOS")) {
		Some(runtime) => Ok(runtime.trim().to_owned()),
		None => Err("none installed; add one in Xcode under Settings > Platforms".to_owned()),
	}
}

fn android_checks() -> Vec<Check> {
	vec![
		Check {
			name: "Android: Java",
			required: true,
			outcome: java_version(),
		},
		Check {
			name: "Android: SDK",
			required: true,
			outcome: android_sdk(),
		},
		Check {
			name: "Android: platform",
			required: true,
			outcome: android_platform(),
		},
		Check {
			name: "Android: NDK",
			required: true,
			outcome: crate::native::android_ndk()
				.map(|ndk| ndk.display().to_string())
				.map_err(|error| format!("{error:#}")),
		},
		Check {
			name: "Android: CMake",
			required: true,
			outcome: android_cmake(),
		},
		Check {
			name: "Android: aarch64-linux-android",
			required: true,
			outcome: rust_target_installed("aarch64-linux-android"),
		},
		Check {
			name: "Android: x86_64-linux-android",
			required: false,
			outcome: rust_target_installed("x86_64-linux-android")
				.map_err(|error| format!("{error}; the emulator and 64-bit Intel devices need it")),
		},
		Check {
			name: "Android: platform tools",
			required: false,
			outcome: android_platform_tools(),
		},
	]
}

/// Java 17 or newer, which Gradle and the Android Gradle plugin require.
/// `java -version` prints to standard error.
fn java_version() -> Result<String, String> {
	let output = Command::new("java")
		.arg("-version")
		.output()
		.map_err(|error| format!("missing: {error}"))?;

	let text = format!(
		"{}{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
	let line = text
		.lines()
		.find(|line| line.contains("version"))
		.unwrap_or_default()
		.trim()
		.to_owned();

	if !output.status.success() {
		return Err(format!("`java -version` exited with {}", output.status));
	}

	// `openjdk version "21.0.2"`: the quoted version's first number is the
	// major, except legacy `1.8.0` schemes, whose second is.
	let quoted = line.split('"').nth(1).unwrap_or_default();

	if java_major(quoted) >= 17 {
		Ok(line)
	} else {
		Err(format!("Java 17 or newer is required; this is {quoted}"))
	}
}

/// The major version of a Java version string: `21.0.2` is 21, and the
/// legacy `1.8.0_392` scheme's second number is the major.
fn java_major(quoted: &str) -> u64 {
	let mut numbers = quoted.split('.');
	let leading = numbers
		.next()
		.and_then(|number| number.parse::<u64>().ok())
		.unwrap_or_default();

	if leading == 1 {
		numbers
			.next()
			.and_then(|number| number.parse::<u64>().ok())
			.unwrap_or_default()
	} else {
		leading
	}
}

fn android_sdk() -> Result<String, String> {
	match android_sdk_path() {
		Some(path) => Ok(path.display().to_string()),
		None => Err("missing: set ANDROID_HOME (or ANDROID_SDK_ROOT) to the SDK".to_owned()),
	}
}

fn android_sdk_path() -> Option<PathBuf> {
	let path = std::env::var_os("ANDROID_HOME")
		.or_else(|| std::env::var_os("ANDROID_SDK_ROOT"))
		.map(PathBuf::from);
	path.filter(|path| path.is_dir())
}

/// The Android platform the generated Gradle project compiles against.
fn android_platform() -> Result<String, String> {
	let platform = android_sdk_path().map(|sdk| sdk.join("platforms").join(ANDROID_PLATFORM));

	match platform {
		Some(path) if path.is_dir() => Ok(ANDROID_PLATFORM.to_owned()),
		_ => {
			Err(format!(
				"missing {ANDROID_PLATFORM}; install it with `sdkmanager \
				 \"platforms;{ANDROID_PLATFORM}\"`"
			))
		}
	}
}

/// The SDK's `CMake`, which the generated `CMakeLists.txt` builds through.
fn android_cmake() -> Result<String, String> {
	let directory = android_sdk_path().map(|sdk| sdk.join("cmake"));
	let Some(directory) = directory.filter(|directory| directory.is_dir()) else {
		return Err("missing; install it with `sdkmanager \"cmake;3.31.11\"`".to_owned());
	};

	let mut versions: Vec<_> = std::fs::read_dir(&directory)
		.map(|entries| {
			entries
				.filter_map(std::io::Result::ok)
				.map(|entry| entry.file_name().to_string_lossy().into_owned())
				.collect()
		})
		.unwrap_or_default();
	versions.sort();

	versions
		.pop()
		.ok_or_else(|| "the SDK's cmake directory is empty".to_owned())
}

fn android_platform_tools() -> Result<String, String> {
	match android_sdk_path().map(|sdk| sdk.join("platform-tools/adb")) {
		Some(path) if path.is_file() => Ok(path.display().to_string()),
		_ => Err("missing; `leptoskit dev` needs adb from platform-tools".to_owned()),
	}
}

/// GTK 4.6 or newer, which the Linux runner's crates discover through
/// pkg-config.
fn gtk4_version() -> Result<String, String> {
	let output = Command::new("pkg-config")
		.args(["--modversion", "gtk4"])
		.output()
		.map_err(|error| format!("missing: {error}"))?;

	if !output.status.success() {
		return Err(
			"not found; install the GTK 4 development packages (libgtk-4-dev on Debian and Ubuntu)"
				.to_owned(),
		);
	}

	let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
	let mut numbers = version.split('.');
	let (major, minor) = (
		numbers
			.next()
			.and_then(|number| number.parse::<u64>().ok())
			.unwrap_or_default(),
		numbers
			.next()
			.and_then(|number| number.parse::<u64>().ok())
			.unwrap_or_default(),
	);

	if (major, minor) >= (4, 6) {
		Ok(format!("gtk4 {version}"))
	} else {
		Err(format!("GTK 4.6 or newer is required; this is {version}"))
	}
}

fn harmonyos_checks() -> Vec<Check> {
	vec![
		Check {
			name: "HarmonyOS: hvigorw",
			required: false,
			outcome: hvigorw(),
		},
		Check {
			name: "HarmonyOS: ohpm",
			required: false,
			outcome: version_of("ohpm")
				.map_err(|error| format!("{error}; DevEco's package manager, for `ohpm install`")),
		},
	]
}

fn hvigorw() -> Result<String, String> {
	if let Some(path) = std::env::var_os("HVIGORW") {
		let path = PathBuf::from(path);

		return if path.is_file() {
			Ok(path.display().to_string())
		} else {
			Err(format!("{} does not exist", path.display()))
		};
	}

	match Command::new("hvigorw").arg("--version").output() {
		Ok(output) if output.status.success() => Ok("on PATH".to_owned()),
		_ => Err("missing: install DevEco Studio and put hvigorw on PATH or in HVIGORW".to_owned()),
	}
}

/// The first line a tool prints for `--version`, requiring a zero exit.
fn version_of(program: &str) -> Result<String, String> {
	tool_version(program, "--version")
}

/// Xcode's version, which unlike most tools spells the flag `-version`.
fn xcodebuild_version() -> Result<String, String> {
	tool_version("xcodebuild", "-version")
}

fn tool_version(program: &str, flag: &str) -> Result<String, String> {
	let output = Command::new(program)
		.arg(flag)
		.output()
		.map_err(|error| format!("missing: {error}"))?;

	if !output.status.success() {
		return Err(format!("`{program} {flag}` exited with {}", output.status));
	}

	Ok(String::from_utf8_lossy(&output.stdout)
		.lines()
		.next()
		.unwrap_or_default()
		.to_owned())
}

/// Whether the active toolchain's sysroot carries `target`'s standard
/// library. Asking the sysroot (rather than rustup) also works for
/// toolchains rustup does not manage, such as Nix-provided ones.
fn rust_target_installed(target: &str) -> Result<String, String> {
	let output = Command::new("rustc")
		.args(["--print", "sysroot"])
		.output()
		.map_err(|error| format!("could not run rustc: {error}"))?;

	if !output.status.success() {
		return Err(format!(
			"`rustc --print sysroot` exited with {}",
			output.status
		));
	}

	let sysroot = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
	let target_lib = sysroot.join("lib").join("rustlib").join(target);

	if target_lib.is_dir() {
		Ok("installed".to_owned())
	} else {
		Err(format!(
			"missing; install it with `rustup target add {target}`"
		))
	}
}

/// Chrome for Testing, which `leptoskit dev` and `leptoskit test extension`
/// run extensions in, when the project has one.
fn extension_check() -> Option<Check> {
	let (config, _) = LeptoskitConfig::load().ok()?;
	config.extension.as_ref()?;
	Some(Check {
		name: "Chrome for Testing",
		required: false,
		outcome: crate::extension::dev::chrome()
			.map(|path| path.display().to_string())
			.ok_or_else(|| crate::extension::dev::support().err().unwrap_or_default()),
	})
}

/// Whether the app's client crate can take the `DevTools` agents, which
/// `leptoskit dev` turns on through a `devtools` feature.
fn devtools_check() -> Option<Check> {
	let (config, root) = LeptoskitConfig::load().ok()?;
	let metadata = crate::wasm::cargo_metadata(&root).ok()?;
	let client = crate::wasm::detect_client_with(
		config.build.client_package.as_deref(),
		Some(&config.app.name),
		&metadata,
	)
	.ok()??;
	let package = metadata.package(&client.name)?;
	Some(Check {
		name: "devtools",
		required: false,
		outcome: devtools_outcome(package),
	})
}

fn devtools_outcome(package: &crate::wasm::MetadataPackage) -> Result<String, String> {
	if package.declares_feature("devtools") {
		Ok(format!("{} declares `devtools`", package.name))
	} else {
		Err(format!(
			"add `devtools = [\"leptoskit/devtools\"]` to {}'s [features] so `leptoskit dev` can \
			 inspect it",
			package.name
		))
	}
}

/// The project's wasm-bindgen compatibility, when run inside a project.
fn project_check() -> Option<Check> {
	let outcome = match LeptoskitConfig::load() {
		Err(ConfigError::NotFound) => return None,
		Err(error) => Err(format!("{:#}", anyhow::Error::from(error))),
		Ok((_, root)) => {
			match find_lockfile(&root) {
				None => Ok("no Cargo.lock yet; `leptoskit build` creates one".to_owned()),
				Some(lockfile) => lockfile_compatibility(&lockfile),
			}
		}
	};

	Some(Check {
		name: "project wasm-bindgen",
		required: true,
		outcome,
	})
}

fn lockfile_compatibility(lockfile: &Path) -> Result<String, String> {
	let contents = std::fs::read_to_string(lockfile)
		.map_err(|error| format!("could not read {}: {error}", lockfile.display()))?;

	match crate::wasm::wasm_bindgen_compatibility(&contents) {
		Ok(BindgenCompatibility::NotLocked) => Ok("no wasm-bindgen in Cargo.lock".to_owned()),
		Ok(BindgenCompatibility::Compatible) => {
			Ok(format!(
				"binds with the embedded wasm-bindgen {}",
				crate::wasm::embedded_wasm_bindgen()
			))
		}
		Ok(BindgenCompatibility::Mismatch { locked }) => {
			Err(crate::wasm::mismatch_message(&locked))
		}
		Err(error) => Err(format!("{error:#}")),
	}
}

/// The nearest `Cargo.lock` at or above the project root: cargo keeps it at
/// the workspace root.
fn find_lockfile(root: &Path) -> Option<PathBuf> {
	root.ancestors()
		.map(|dir| dir.join("Cargo.lock"))
		.find(|path| path.is_file())
}

/// Whether the app's and its services' database pools fit in Postgres's
/// connection limit, for an app on Postgres with services or managed hosts.
fn connections_check(config: &LeptoskitConfig, root: &Path) -> Option<Check> {
	let postgres = config
		.db
		.as_ref()
		.is_some_and(|db| db.dialect == leptoskit_config::DbDialect::Postgres);

	if !postgres || (config.services.is_empty() && config.infra.is_none()) {
		return None;
	}

	let settings = config
		.infra
		.as_ref()
		.map(|infra| (infra.postgres, infra.instances))
		.unwrap_or_default();
	let (limits, hosts) = (settings.0, settings.1.max(1));
	let outcome = crate::deploy::Project::read(root, config)
		.map_err(|error| format!("could not read the services: {error:#}"))
		.and_then(|project| {
			let budget = crate::deploy::services::Budget::new(
				limits.app_pool,
				u32::from(hosts),
				&project.services,
				limits.max_connections,
			);

			if budget.fits() {
				Ok(format!(
					"{} of {} connections at most",
					budget.total, budget.limit
				))
			} else {
				Err(format!(
					"the database pools need more connections than Postgres allows:{}\nraise \
					 infra.postgres.max_connections, or lower infra.postgres.app_pool and the \
					 services' `pool`",
					budget.explain()
				))
			}
		});
	Some(Check {
		name: "Postgres connections",
		required: true,
		outcome,
	})
}
#[cfg(test)]
mod tests {
	use leptoskit_config::LeptoskitConfig;
	use leptoskit_config::Platform;

	use super::java_major;
	use super::selected_platforms;

	#[test]
	fn a_project_selects_its_native_platforms_in_display_order() {
		let mut config = LeptoskitConfig::default();
		config.app.platforms = vec![
			Platform::Linux,
			Platform::Android,
			Platform::Web,
			Platform::Ios,
		];
		assert_eq!(
			selected_platforms(false, Some(&config)),
			[Platform::Ios, Platform::Android, Platform::Linux]
		);
		assert_eq!(selected_platforms(false, None), Vec::<Platform>::new());

		// HarmonyOS appears only when the project selects it.
		config.app.platforms.push(Platform::Harmonyos);
		assert_eq!(
			selected_platforms(false, Some(&config)),
			[
				Platform::Ios,
				Platform::Android,
				Platform::Harmonyos,
				Platform::Linux
			]
		);

		let all = selected_platforms(true, None);
		assert!(all.contains(&Platform::Android));
		assert!(
			!all.contains(&Platform::Harmonyos),
			"--all leaves DevEco's tools out"
		);
		assert_eq!(
			all.contains(&Platform::Ios),
			cfg!(target_os = "macos"),
			"iOS checks only run where Xcode can"
		);
	}

	#[test]
	fn java_s_major_version_comes_from_either_numbering_scheme() {
		assert_eq!(java_major("21.0.2"), 21);
		assert_eq!(java_major("17.0.11"), 17);
		assert_eq!(java_major("1.8.0_392"), 8);
		assert_eq!(java_major(" nonsense "), 0);
	}
}
