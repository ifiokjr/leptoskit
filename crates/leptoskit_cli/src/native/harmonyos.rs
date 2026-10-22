//! HarmonyOS Stage-model projects, Rust cross compilation, and HAP packaging.

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::LeptoskitConfig;

use super::Target;
use super::project;
use super::registry::Registry;

/// Reject capabilities the initial ArkUI host cannot service before doing work.
pub(super) fn validate(config: &LeptoskitConfig) -> Result<()> {
	let package = if config.native.package_id.is_empty() {
		leptoskit_config::default_package_id(&config.app.name)
	} else {
		config.native.package_id.clone()
	};
	anyhow::ensure!(
		(7..=128).contains(&package.len()),
		"HarmonyOS native.package_id must contain 7 to 128 ASCII bytes"
	);
	anyhow::ensure!(
		!config.codepush.enabled,
		"HarmonyOS does not yet support code push; set codepush.enabled = false"
	);
	Ok(())
}

pub(super) fn validate_plugins(registry: &Registry) -> Result<()> {
	for plugin in &registry.plugins {
		anyhow::ensure!(
			!plugin.rust_worker && (plugin.rust_type.is_some() || plugin.rust_factory.is_some()),
			"HarmonyOS currently supports synchronous Rust plugins only; plugin `{}` needs a \
			 HarmonyOS implementation",
			plugin.name
		);
	}
	Ok(())
}

/// Explicitly select the SDK's native directory, never the Android NDK.
pub(super) fn ndk() -> Result<PathBuf> {
	let path = std::env::var_os("OHOS_NDK_HOME").context(
		"HarmonyOS needs OHOS_NDK_HOME pointing to the SDK native directory (containing llvm/ and \
		 sysroot/); install the SDK with DevEco Studio",
	)?;
	let path = std::fs::canonicalize(path).context("resolve OHOS_NDK_HOME")?;
	path.to_str().context("OHOS_NDK_HOME must be UTF-8")?;
	validate_ndk(&path)?;
	Ok(path)
}

fn validate_ndk(path: &Path) -> Result<()> {
	for tool in ["clang", "clang++", "llvm-ar"] {
		let name = format!("{tool}{}", std::env::consts::EXE_SUFFIX);
		anyhow::ensure!(
			path.join("llvm/bin").join(name).is_file(),
			"OHOS_NDK_HOME lacks llvm/bin/{tool}: {}",
			path.display()
		);
	}
	anyhow::ensure!(
		path.join("sysroot/usr/include").is_dir(),
		"OHOS_NDK_HOME lacks sysroot/usr/include: {}",
		path.display()
	);
	Ok(())
}

/// Configure Rust's linker and cc-rs for exactly the selected OHOS
/// architecture.
pub(super) fn configure_cargo(command: &mut Command, ndk: &Path, target: Target) {
	let triple = target.rust();
	let clang_target = if matches!(target, Target::HarmonyosX86_64) {
		"x86_64-linux-ohos"
	} else {
		"aarch64-linux-ohos"
	};
	let tools = ndk.join("llvm/bin");
	let sysroot = ndk.join("sysroot");
	let escaped_sysroot = sysroot.to_string_lossy().replace('\'', "'\\''");
	let key = triple.replace('-', "_");
	let compiler = |name: &str| tools.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
	let mut flags = std::env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_else(|_| {
		std::env::var("RUSTFLAGS")
			.unwrap_or_default()
			.split_whitespace()
			.collect::<Vec<_>>()
			.join("\x1f")
	});
	for flag in [
		format!("link-arg=--target={clang_target}"),
		format!("link-arg=--sysroot={}", sysroot.display()),
	] {
		if !flags.is_empty() {
			flags.push('\x1f');
		}
		flags.push_str("-C\x1f");
		flags.push_str(&flag);
	}
	command
		.env("CC_SHELL_ESCAPED_FLAGS", "1")
		.env(
			format!("CARGO_TARGET_{}_LINKER", key.to_ascii_uppercase()),
			compiler("clang"),
		)
		.env("CARGO_ENCODED_RUSTFLAGS", flags)
		.env(format!("CC_{key}"), compiler("clang"))
		.env(format!("CXX_{key}"), compiler("clang++"))
		.env(format!("AR_{key}"), compiler("llvm-ar"))
		.env(
			format!("CFLAGS_{key}"),
			format!("--target={clang_target} --sysroot='{escaped_sysroot}' -D__MUSL__"),
		)
		.env(
			format!("CXXFLAGS_{key}"),
			format!("--target={clang_target} --sysroot='{escaped_sysroot}' -D__MUSL__"),
		);
}

/// Create user-owned DevEco sources once; only derived build inputs regenerate.
pub(super) fn generate(
	root: &Path,
	config: &LeptoskitConfig,
	registry: &Registry,
	target: Target,
) -> Result<()> {
	scaffold(root, config)?;
	let abi = if matches!(target, Target::HarmonyosX86_64) {
		"x86_64"
	} else {
		"arm64-v8a"
	};
	let profile =
		include_str!("templates/harmonyos/entry-build-profile.json5").replace("__ABI__", abi);
	project::write(
		&root.join("hosts/harmonyos/entry/build-profile.json5"),
		&profile,
	)?;
	// CMake bracket arguments preserve spaces, quotes, and Windows separators.
	let cmake_path = |path: &Path| -> Result<String> {
		let path = path.to_str().context("HarmonyOS path is not UTF-8")?;
		let value = path
			.strip_prefix(r"\\?\")
			.unwrap_or(path)
			.replace('\\', "/");
		anyhow::ensure!(!value.contains("]=]"), "unsupported CMake path");
		Ok(format!("[=[{value}]=]"))
	};
	project::write(
		&root.join(".leptoskit/harmonyos/paths.cmake"),
		&format!(
			"set(LEPTOSKIT_BRIDGE {})\nset(LEPTOSKIT_LIBRARY {})\n",
			cmake_path(&registry.bridge)?,
			cmake_path(&root.join(".leptoskit/lib/libleptoskit_bridge_ffi.a"))?
		),
	)?;
	Ok(())
}

/// The app shell is owned by the application, just like hosts/android.
pub(super) fn scaffold(root: &Path, config: &LeptoskitConfig) -> Result<()> {
	let host = root.join("hosts/harmonyos");
	let package = if config.native.package_id.is_empty() {
		leptoskit_config::default_package_id(&config.app.name)
	} else {
		config.native.package_id.clone()
	};
	let app = serde_json::json!({"app": {
		"bundleName": package, "vendor": "leptoskit", "versionCode": 1,
		"versionName": config.app.version.to_string(), "icon": "$media:app_icon", "label": "$string:app_name"
	}});
	let resources = serde_json::json!({"string": [{"name":"app_name", "value": config.app.name}]});
	let files = [
		("AppScope/app.json5", serde_json::to_string_pretty(&app)?),
		(
			"AppScope/resources/base/element/string.json",
			serde_json::to_string_pretty(&resources)?,
		),
		(
			"AppScope/resources/base/media/app_icon.svg",
			include_str!("templates/harmonyos/app_icon.svg").into(),
		),
		(
			"build-profile.json5",
			include_str!("templates/harmonyos/build-profile.json5").into(),
		),
		(
			"oh-package.json5",
			include_str!("templates/harmonyos/oh-package.json5").into(),
		),
		(
			"TESTING.md",
			include_str!("templates/harmonyos/testing.md").replace("__BUNDLE_NAME__", &package),
		),
		(
			"entry/src/ohosTest/module.json5",
			include_str!("templates/harmonyos/test-module.json5").into(),
		),
		(
			"entry/src/ohosTest/ets/testrunner/OpenHarmonyTestRunner.ets",
			include_str!("templates/harmonyos/OpenHarmonyTestRunner.ets").into(),
		),
		(
			"entry/src/ohosTest/ets/test/Counter.test.ets",
			include_str!("templates/harmonyos/Counter.test.ets")
				.replace("__BUNDLE_NAME__", &package),
		),
		(
			"hvigorfile.ts",
			"import { appTasks } from '@ohos/hvigor-ohos-plugin';\nexport default { system: \
			 appTasks, plugins: [] };\n"
				.into(),
		),
		(
			"hvigor/hvigor-config.json5",
			"{\"modelVersion\": \"5.0.0\", \"dependencies\": {}, \"execution\": {}, \"logging\": \
			 {}, \"debugging\": {}, \"nodeOptions\": {}}\n"
				.into(),
		),
		(
			"entry/hvigorfile.ts",
			"import { hapTasks } from '@ohos/hvigor-ohos-plugin';\nexport default { system: \
			 hapTasks, plugins: [] };\n"
				.into(),
		),
		(
			"entry/oh-package.json5",
			"{\"name\": \"entry\", \"version\": \"1.0.0\", \"dependencies\": \
			 {\"libleptoskit.so\": \"file:./src/main/cpp/types/libleptoskit\"}}\n"
				.into(),
		),
		(
			"entry/build-profile.json5",
			include_str!("templates/harmonyos/entry-build-profile.json5")
				.replace("__ABI__", "arm64-v8a"),
		),
		(
			"entry/src/main/module.json5",
			include_str!("templates/harmonyos/module.json5").into(),
		),
		(
			"entry/src/main/resources/base/profile/main_pages.json",
			"{\"src\": [\"pages/Index\"]}\n".into(),
		),
		(
			"entry/src/main/resources/base/element/color.json",
			"{\"color\": [{\"name\": \"start_window_background\", \"value\": \"#FFFFFF\"}]}\n"
				.into(),
		),
		(
			"entry/src/main/ets/entryability/EntryAbility.ets",
			include_str!("templates/harmonyos/EntryAbility.ets").into(),
		),
		(
			"entry/src/main/ets/pages/Index.ets",
			include_str!("templates/harmonyos/Index.ets").into(),
		),
		(
			"entry/src/main/ets/scene.ts",
			include_str!("templates/harmonyos/scene.ts").into(),
		),
		(
			"entry/src/main/cpp/CMakeLists.txt",
			include_str!("templates/harmonyos/CMakeLists.txt").into(),
		),
		(
			"entry/src/main/cpp/types/libleptoskit/index.d.ts",
			include_str!("templates/harmonyos/index.d.ts").into(),
		),
		(
			"entry/src/main/cpp/types/libleptoskit/oh-package.json5",
			"{\"name\":\"libleptoskit.so\",\"version\":\"1.0.0\",\"types\":\"./index.d.ts\"}\n"
				.into(),
		),
		(
			".gitignore",
			"/.hvigor\n/oh_modules\n/entry/oh_modules\n/entry/build\n/entry/.test\n/entry/.cxx\n/\
			 entry/build-profile.json5\n/local.properties\n/entry/src/main/resources/rawfile/app.\
			 wasm\n"
				.into(),
		),
	];
	for (name, content) in files {
		let path = host.join(name);
		if !path.try_exists()? {
			let path = camino::Utf8Path::from_path(&path)
				.context("HarmonyOS project path is not UTF-8")?;
			crate::new_project::write_new(path, &content)?;
		}
	}
	Ok(())
}

/// Package with the SDK-installed Hvigor command, without downloading a
/// wrapper.
pub(super) fn build(root: &Path, release: bool) -> Result<()> {
	let executable = std::env::var_os("HVIGORW").unwrap_or_else(|| "hvigorw".into());
	super::run_tool(
		Command::new(executable)
			.current_dir(root.join("hosts/harmonyos"))
			.args([
				"--mode",
				"module",
				"-p",
				"product=default",
				"-p",
				"module=entry@default",
				"-p",
				if release {
					"buildMode=release"
				} else {
					"buildMode=debug"
				},
				"assembleHap",
				"--no-daemon",
			]),
	)
	.context(
		"HarmonyOS HAP packaging requires DevEco's hvigorw on PATH (or set HVIGORW); sync the \
		 project with ohpm install first",
	)?;
	println!(
		"built HAP in {}/hosts/harmonyos/entry/build/default/outputs/default",
		root.display()
	);
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn preserves_host_edits_when_switching_abis_and_rejects_missing_plugins() {
		let root = std::env::temp_dir().join(format!("leptoskit-harmonyos-{}", std::process::id()));
		std::fs::create_dir_all(&root).unwrap();
		let root = root.canonicalize().unwrap();
		let config = LeptoskitConfig::default();
		let mut registry = Registry {
			plugins: Vec::new(),
			support: Vec::new(),
			bridge: root.join("bridge with spaces"),
			target: root.join("target"),
		};
		project::generate(&root, &config, &registry, Target::Harmonyos, false).unwrap();
		let source = root.join("hosts/harmonyos/entry/src/main/ets/pages/Index.ets");
		std::fs::write(&source, "// application-owned edit").unwrap();
		let signing = root.join("hosts/harmonyos/build-profile.json5");
		std::fs::write(&signing, "// application signing configuration").unwrap();
		let tests = root.join("hosts/harmonyos/entry/src/ohosTest/ets/test/Counter.test.ets");
		std::fs::write(&tests, "// application-owned test journey").unwrap();
		project::generate(&root, &config, &registry, Target::HarmonyosX86_64, false).unwrap();
		assert_eq!(
			std::fs::read_to_string(source).unwrap(),
			"// application-owned edit"
		);
		assert_eq!(
			std::fs::read_to_string(signing).unwrap(),
			"// application signing configuration"
		);
		assert_eq!(
			std::fs::read_to_string(tests).unwrap(),
			"// application-owned test journey"
		);
		let profile: serde_json::Value = serde_json::from_str(
			&std::fs::read_to_string(root.join("hosts/harmonyos/entry/build-profile.json5"))
				.unwrap(),
		)
		.unwrap();
		assert_eq!(
			profile["buildOption"]["externalNativeOptions"]["abiFilters"],
			serde_json::json!(["x86_64"])
		);
		assert_eq!(
			profile["targets"],
			serde_json::json!([{"name": "default"}, {"name": "ohosTest"}])
		);
		registry.bridge = PathBuf::from(r"\\?\C:\SDK with spaces\bridge");
		project::generate(&root, &config, &registry, Target::Harmonyos, false).unwrap();
		assert!(
			std::fs::read_to_string(root.join(".leptoskit/harmonyos/paths.cmake"))
				.unwrap()
				.contains("[=[C:/SDK with spaces/bridge]=]")
		);
		for metadata in [
			serde_json::json!({"name": "haptics", "android-type": "Haptics"}),
			serde_json::json!({"name": "store", "rust-type": "Store", "rust-worker": true}),
		] {
			registry.plugins = vec![serde_json::from_value(metadata).unwrap()];
			assert!(
				validate_plugins(&registry)
					.unwrap_err()
					.to_string()
					.contains("HarmonyOS implementation")
			);
		}
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn rejects_code_push_before_building() {
		let mut config = LeptoskitConfig::default();
		config.codepush.enabled = true;
		assert!(
			validate(&config)
				.unwrap_err()
				.to_string()
				.contains("code push")
		);
	}

	#[test]
	fn reports_an_incomplete_sdk() {
		let error = validate_ndk(Path::new("/missing-harmonyos-sdk")).unwrap_err();
		assert!(error.to_string().contains("llvm/bin/clang"));
	}

	#[test]
	fn rejects_bundle_names_outside_the_harmonyos_size_limits() {
		let mut config = LeptoskitConfig::default();
		config.native.package_id = "a.b".into();
		assert!(
			validate(&config)
				.unwrap_err()
				.to_string()
				.contains("7 to 128")
		);
	}
}
