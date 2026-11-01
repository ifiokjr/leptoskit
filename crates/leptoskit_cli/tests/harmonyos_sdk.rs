//! Real SDK compatibility builds. Run explicitly with `OHOS_SDK_HOME`,
//! `HVIGORW`, and `LEPTOSKIT_HYPIUM` pointing to the pinned CI tools.
//! Artifacts survive failures under `target/harmonyos-sdk`.

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

/// Build the real native host and instrumentation package for both supported
/// ABIs.
#[test]
#[ignore = "requires the public API 12 SDK, Hvigor, and Hypium; run by HarmonyOS CI"]
fn packages_counter_and_instrumentation_for_both_abis() {
	let sdk = required_directory("OHOS_SDK_HOME");
	let hypium = required_directory("LEPTOSKIT_HYPIUM");
	let hvigor = std::env::var_os("HVIGORW").expect("HVIGORW must name the pinned wrapper");
	let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.join("../..")
		.canonicalize()
		.unwrap();
	let stamp = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.unwrap()
		.as_nanos();
	let artifacts = repository
		.join("target/harmonyos-sdk")
		.join(format!("{}-{stamp}", std::process::id()));
	fs::create_dir_all(&artifacts).unwrap();
	let app = artifacts.join("counter");

	let host = app.join("hosts/harmonyos");
	let log = artifacts.join("commands.log");
	println!("HarmonyOS SDK artifacts: {}", artifacts.display());
	checked(
		Command::new(env!("CARGO_BIN_EXE_leptoskit"))
			.args(["new"])
			.arg(&app)
			.args([
				"--platform",
				"native",
				"--native-target",
				"harmonyos",
				"--sdk-path",
			])
			.arg(&repository),
		&log,
	);

	// The public SDK uses OpenHarmony's product schema. Keep these edits in
	// the generated fixture, so production HarmonyOS templates stay unchanged.
	let profile = host.join("build-profile.json5");
	let mut product: serde_json::Value =
		serde_json::from_slice(&fs::read(&profile).unwrap()).unwrap();
	product["app"]["products"][0]["runtimeOS"] = "OpenHarmony".into();
	product["app"]["products"][0]["compileSdkVersion"] = 12.into();
	product["app"]["products"][0]["compatibleSdkVersion"] = 12.into();
	fs::write(&profile, serde_json::to_vec_pretty(&product).unwrap()).unwrap();

	for source in ["main", "ohosTest"] {
		let path = host.join(format!("entry/src/{source}/module.json5"));
		let mut module: serde_json::Value =
			serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
		module["module"]["deviceTypes"] = serde_json::json!(["default"]);
		fs::write(path, serde_json::to_vec_pretty(&module).unwrap()).unwrap();
	}

	// Select the ArkTS posture plugin, so the SDK compiles a generated
	// registrant and plugin against the real `@ohos.display` declarations.
	let manifest = app.join("Cargo.toml");
	let mut cargo: toml::Table = fs::read_to_string(&manifest).unwrap().parse().unwrap();
	let mut posture = toml::Table::new();
	posture.insert(
		"path".into(),
		repository
			.join("crates/leptoskit_plugin_posture")
			.to_str()
			.unwrap()
			.into(),
	);
	cargo["dependencies"]
		.as_table_mut()
		.unwrap()
		.insert("leptoskit_plugin_posture".into(), posture.into());
	fs::write(&manifest, toml::to_string(&cargo).unwrap()).unwrap();
	let settings = app.join("leptoskit.toml");
	let mut config: toml::Table = fs::read_to_string(&settings).unwrap().parse().unwrap();

	config["native"]
		.as_table_mut()
		.unwrap()
		.insert("plugins".into(), vec!["posture"].into());
	fs::write(&settings, toml::to_string(&config).unwrap()).unwrap();
	fs::write(
		host.join("local.properties"),
		format!("sdk.dir={}\n", sdk.display()),
	)
	.unwrap();
	// Match ohpm's installed source layout without needing an account or a
	// package-manager bootstrap. The workflow verifies the official HAR hash.
	let installed = host.join("oh_modules/@ohos/hypium");
	copy_directory(&hypium, &installed);
	let native = host.join("entry/oh_modules/libleptoskit.so");
	copy_directory(&host.join("entry/src/main/cpp/types/libleptoskit"), &native);
	for (target, abi, mode) in [
		("harmonyos", "arm64-v8a", "debug"),
		("harmonyos-x86_64", "x86_64", "release"),
	] {
		let mut command = Command::new(env!("CARGO_BIN_EXE_leptoskit"));
		command
			.current_dir(&app)
			.env("OHOS_NDK_HOME", sdk.join("12/native"))
			.env(
				"CARGO_TARGET_DIR",
				repository.join("target/harmonyos-sdk/cargo"),
			)
			.args(["native", "build", target]);

		if mode == "release" {
			command.arg("--release");
		}

		checked(&mut command, &log);
		let hap = host.join("entry/build/default/outputs/default/entry-default-unsigned.hap");
		let listing = checked(Command::new("unzip").arg("-t").arg(&hap), &log);
		let listing = String::from_utf8(listing.stdout).unwrap();
		for entry in [
			format!("libs/{abi}/libleptoskit.so"),
			"ets/modules.abc".into(),
			"resources/rawfile/app.wasm".into(),
		] {
			assert!(
				listing.contains(&entry),
				"missing {entry} from {}",
				hap.display()
			);
		}
		fs::copy(hap, artifacts.join(format!("{target}-{mode}.hap"))).unwrap();
		checked(
			Command::new(&hvigor).current_dir(&host).args([
				"--mode",
				"module",
				"-p",
				"product=default",
				"-p",
				"module=entry@ohosTest",
				"-p",
				&format!("buildMode={mode}"),
				"assembleHap",
				"--no-daemon",
			]),
			&log,
		);
		let test = host.join("entry/build/default/outputs/ohosTest/entry-ohosTest-unsigned.hap");
		let listing = checked(Command::new("unzip").arg("-t").arg(&test), &log);
		assert!(
			String::from_utf8(listing.stdout)
				.unwrap()
				.contains("ets/modules.abc")
		);
		fs::copy(
			test,
			artifacts.join(format!("{target}-{mode}-instrumentation.hap")),
		)
		.unwrap();
	}
}

/// Validate SDK inputs before generating the fixture.
fn required_directory(name: &str) -> PathBuf {
	let path =
		PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} is required")));
	let path = path.canonicalize().unwrap();
	assert!(path.is_dir(), "{name} must be a directory");
	path
}

/// Preserve subprocess evidence before propagating a build failure.
fn checked(command: &mut Command, log: &Path) -> Output {
	use std::io::Write;
	let output = command
		.output()
		.unwrap_or_else(|error| panic!("{command:?}: {error}"));
	let mut log = fs::OpenOptions::new()
		.create(true)
		.append(true)
		.open(log)
		.unwrap();
	writeln!(log, "{command:?}\n{}", output.status).unwrap();
	log.write_all(&output.stdout).unwrap();
	log.write_all(&output.stderr).unwrap();
	assert!(
		output.status.success(),
		"{command:?} failed; see commands.log"
	);
	output
}

/// Install package sources into the fixture's actual `oh_modules` path.
fn copy_directory(source: &Path, destination: &Path) {
	fs::create_dir_all(destination).unwrap();

	for entry in fs::read_dir(source).unwrap() {
		let entry = entry.unwrap();
		let destination = destination.join(entry.file_name());

		if entry.file_type().unwrap().is_dir() {
			copy_directory(&entry.path(), &destination);
		} else {
			fs::copy(entry.path(), destination).unwrap();
		}
	}
}
