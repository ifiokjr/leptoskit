//! User-owned native application sources; generated build state stays separate.

use anyhow::Context;
use anyhow::Result;
use camino::Utf8PathBuf;
use leptoskit_config::AppConfig;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::NativeConfig;
use leptoskit_config::Platform;

/// Generates an application with a Rust guest, host library, UIKit/AppKit
/// scenes and Android activity. Existing destinations are never overwritten.
pub fn run(path: &str, name: Option<&str>, sdk: Option<&str>) -> Result<()> {
	run_for_target(path, name, sdk, None)
}

/// Generates one selected native target, or the default Apple/Android set.
pub fn run_for_target(
	path: &str,
	name: Option<&str>,
	sdk: Option<&str>,
	target: Option<leptoskit_config::NativeTarget>,
) -> Result<()> {
	anyhow::ensure!(
		!matches!(
			target,
			Some(leptoskit_config::NativeTarget::Windows | leptoskit_config::NativeTarget::Linux)
		),
		"application scaffolding currently supports macos, ios, android, and harmonyos"
	);
	let harmonyos = target == Some(leptoskit_config::NativeTarget::Harmonyos);
	let destination = std::path::Path::new(path);
	let name = name
		.or_else(|| destination.file_name().and_then(|name| name.to_str()))
		.context("pass --name when the destination has no package name")?;
	crate::new_project::ensure_package_name(name)?;
	anyhow::ensure!(
		name.bytes().all(|byte| {
			byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
		}),
		"native application names must be lowercase"
	);
	anyhow::ensure!(
		!destination.exists(),
		"{} already exists",
		destination.display()
	);
	let sdk = sdk
		.map(std::fs::canonicalize)
		.transpose()
		.context("resolve --sdk-path")?;
	let dependency = |package: &str, features: &str| -> Result<String> {
		let mut value = toml::Table::new();
		value.insert(
			"version".into(),
			format!("={}", env!("CARGO_PKG_VERSION")).into(),
		);
		if let Some(sdk) = &sdk {
			let path = sdk.join("crates").join(package);
			anyhow::ensure!(
				path.join("Cargo.toml").is_file(),
				"--sdk-path lacks {package}"
			);
			value.insert(
				"path".into(),
				path.to_str().context("SDK path is not UTF-8")?.into(),
			);
		}
		if !features.is_empty() {
			value.insert("features".into(), vec![features].into());
		}
		Ok(format!(
			"[dependencies.{package}]\n{}\n",
			toml::to_string(&value)?
		))
	};
	let mut config = LeptoskitConfig {
		app: AppConfig {
			name: name.into(),
			default_platform: Platform::Native,
			..AppConfig::default()
		},
		native: NativeConfig {
			package_id: leptoskit_config::default_package_id(name),
			plugins: [
				"os",
				"crypto",
				"clipboard",
				"haptics",
				"battery",
				"device",
				"secure-store",
				"filesystem",
			]
			.map(str::to_owned)
			.to_vec(),
			targets: vec![
				leptoskit_config::NativeTarget::Ios,
				leptoskit_config::NativeTarget::Android,
				leptoskit_config::NativeTarget::Macos,
			],
			..NativeConfig::default()
		},
		..LeptoskitConfig::default()
	};
	if let Some(target) = target {
		config.native.targets = vec![target];
	}
	if harmonyos {
		config.native.plugins.clear();
		super::harmonyos::validate(&config)?;
	}
	let mut manifest = format!(
		"[package]\nname = {name:?}\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = \
		 false\n\n[lib]\nname = \"leptoskit_app\"\ncrate-type = \
		 [\"staticlib\"]\n\n[workspace]\nmembers = [\"guest\"]\n\n[profile.release]\nlto = \
		 \"thin\"\ncodegen-units = 1\n\n# Keep host proc-macros loadable on macOS 27 \
		 (rust-lang/rust#157750).\n[profile.release.build-override]\nstrip = \"none\"\n\n"
	);
	for (package, features) in [
		("leptoskit_bridge_ffi", "app"),
		("leptoskit_bridge", ""),
		("leptoskit_plugin_os", ""),
		("leptoskit_plugin_crypto", ""),
		("leptoskit_plugin_clipboard", ""),
		("leptoskit_plugin_haptics", ""),
		("leptoskit_plugin_battery", ""),
		("leptoskit_plugin_device", ""),
		("leptoskit_plugin_secure_store", ""),
		("leptoskit_plugin_filesystem", ""),
	] {
		if harmonyos && !matches!(package, "leptoskit_bridge_ffi" | "leptoskit_bridge") {
			continue;
		}
		manifest.push_str(&dependency(package, features)?);
	}
	let mut guest_manifest = format!(
		"[package]\nname = \"{name}_guest\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = \
		 false\n\n[lib]\ncrate-type = [\"cdylib\"]\n\n{}[dependencies]\nserde_json = \
		 \"1\"\nfutures = {{ version = \"0.3\", features = [\"executor\"] }}\n",
		dependency("leptoskit_bridge", "guest")?
	);
	for package in [
		"leptoskit_plugin_haptics",
		"leptoskit_plugin_battery",
		"leptoskit_plugin_device",
		"leptoskit_plugin_secure_store",
		"leptoskit_plugin_filesystem",
	] {
		if harmonyos {
			continue;
		}
		guest_manifest.push_str(&dependency(package, "guest")?);
	}
	let package = &config.native.package_id;
	let files = vec![
		("Cargo.toml", manifest),
		(
			"hosts/android/gradlew",
			include_str!("templates/gradle/gradlew").into(),
		),
		(
			"hosts/android/gradlew.bat",
			include_str!("templates/gradle/gradlew.bat").into(),
		),
		(
			"hosts/android/gradle/wrapper/gradle-wrapper.properties",
			include_str!("templates/gradle/gradle-wrapper.properties").into(),
		),
		(
			"hosts/android/gradle/LICENSE",
			include_str!("templates/gradle/LICENSE").into(),
		),
		(
			"src/lib.rs",
			"//! The installed native runtime. Registration is generated from Cargo \
			 metadata.\ninclude!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \
			 \"/.leptoskit/registrant.rs\"));\n"
				.into(),
		),
		("guest/Cargo.toml", guest_manifest),
		(
			"guest/src/lib.rs",
			if harmonyos {
				include_str!("templates/harmonyos/guest.rs")
			} else {
				include_str!("templates/guest.rs")
			}
			.into(),
		),
		("leptoskit.toml", config.to_toml_string()?),
		(
			".gitignore",
			"/target\n/.leptoskit\n/hosts/android/.gradle\n/hosts/android/**/build\n/hosts/\
			 android/local.properties\n"
				.into(),
		),
		(
			"hosts/ios/App.swift",
			include_str!("templates/App.swift").into(),
		),
		(
			"hosts/macos/App.swift",
			include_str!("templates/MacApp.swift").into(),
		),
		(
			"hosts/macos/Info.plist",
			include_str!("templates/MacInfo.plist")
				.replace("__PACKAGE__", package)
				.replace("__NAME__", name)
				.replace("__VERSION__", "0.1.0"),
		),
		(
			"hosts/ios/AppUITests.swift",
			include_str!("templates/AppUITests.swift").into(),
		),
		(
			"hosts/android/app/src/androidTest/java/NativeAppTest.kt",
			include_str!("templates/NativeAppTest.kt").replace("dev.leptoskit.generated", package),
		),
		(
			"hosts/ios/Info.plist",
			include_str!("templates/Info.plist")
				.replace("__PACKAGE__", package)
				.replace("__NAME__", name),
		),
		(
			"hosts/android/app/src/main/java/MainActivity.kt",
			include_str!("templates/MainActivity.kt").replace("dev.leptoskit.generated", package),
		),
		(
			"hosts/android/settings.gradle.kts",
			"pluginManagement { repositories { google(); mavenCentral(); gradlePluginPortal() } \
			 }\ndependencyResolutionManagement { \
			 repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS); repositories { \
			 google(); mavenCentral() } }\nrootProject.name = \
			 \"LeptosKitApp\"\ninclude(\":app\")\n"
				.into(),
		),
		(
			"hosts/android/build.gradle.kts",
			"plugins { id(\"com.android.application\") version \"9.0.1\" apply false }\n".into(),
		),
		(
			"hosts/android/gradle.properties",
			"org.gradle.jvmargs=-Xmx2g\norg.gradle.workers.max=2\nandroid.useAndroidX=true\n"
				.into(),
		),
		(
			"hosts/android/app/build.gradle.kts",
			include_str!("templates/build.gradle.kts").replace("__PACKAGE__", package),
		),
		(
			"hosts/android/app/src/main/res/values/styles.xml",
			include_str!("templates/styles.xml").into(),
		),
		(
			"hosts/android/app/src/main/AndroidManifest.xml",
			include_str!("templates/AndroidManifest.xml")
				.replace("__PACKAGE__", package)
				.replace("__NAME__", name),
		),
		(
			"README.md",
			format!(
				"# {name}\n\nThis app runs a Rust/Wasm interface on UIKit, AppKit and Android \
				 controls. Selected plugins in `native.plugins` are registered from direct Cargo \
				 dependency metadata. OS permission dialogs remain separate.\n\n```sh\nleptoskit \
				 native build ios-simulator\nleptoskit native build android\nleptoskit native \
				 build macos\n```\n\nInstall Xcode and the iOS Rust targets, or Android SDK/NDK, \
				 Java 17+ and the Android Rust target. Set `ANDROID_HOME` for Android builds. The \
				 pinned Gradle wrapper verifies the distribution checksum on first \
				 download.\n\nEdit `guest/src/lib.rs` for the interface and `hosts/ios`, \
				 `hosts/macos` or `hosts/android` for native app configuration. `.leptoskit` is \
				 regenerated; never edit its registrants. Re-run `leptoskit native prepare \
				 ios-simulator` after changing dependencies, then open \
				 `.leptoskit/ios/Host.xcodeproj` in Xcode. For Android, prepare the Android \
				 target before opening `hosts/android` in Android Studio.\n\nThe CLI produces \
				 ad-hoc signed macOS and iOS simulator applications for local development. iOS \
				 device bundles remain unsigned; select your development team/provisioning in \
				 Xcode for a device or distribution build. Android debug builds are locally \
				 signed by Gradle; configure release signing in your app's Gradle project before \
				 distribution.\n"
			),
		),
	];
	std::fs::create_dir_all(destination)?;
	let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(destination)?)
		.map_err(|_| anyhow::anyhow!("destination path is not UTF-8"))?;
	for (path, content) in files {
		if harmonyos && (path.starts_with("hosts/") || path.eq_ignore_ascii_case("readme.md")) {
			continue;
		}
		crate::new_project::write_new(&root.join(path), &content)?;
	}
	if harmonyos {
		super::harmonyos::scaffold(root.as_std_path(), &config)?;
		crate::new_project::write_new(
			&root.join("readme.md"),
			include_str!("templates/harmonyos/readme.md"),
		)?;
		println!(
			"created HarmonyOS native app in {root}\nnext: cd {root} && leptoskit native build \
			 harmonyos"
		);
		return Ok(());
	}
	{
		use std::io::Write as _;
		let mut file = std::fs::OpenOptions::new()
			.write(true)
			.create_new(true)
			.open(root.join("hosts/android/gradle/wrapper/gradle-wrapper.jar"))?;
		file.write_all(include_bytes!("templates/gradle/gradle-wrapper.jar"))?;
	}
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		std::fs::set_permissions(
			root.join("hosts/android/gradlew"),
			std::fs::Permissions::from_mode(0o755),
		)?;
	}
	println!(
		"created native app in {root}\nnext: cd {root} && leptoskit native build ios-simulator"
	);
	Ok(())
}
