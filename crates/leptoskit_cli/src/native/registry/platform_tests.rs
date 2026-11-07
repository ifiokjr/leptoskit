// Per-platform registration tables and what each destination resolves to.

use leptoskit_config::MissingPlugins;

use super::*;

fn runtime_metadata(runtime: &serde_json::Value) -> Metadata {
	let mut value = metadata("Plugin");
	value.packages[2].metadata = json!({"leptoskit": {"runtime": runtime}});
	value
}

fn echo_config() -> LeptoskitConfig {
	let mut config = LeptoskitConfig::default();
	config.app.name = "app".into();
	config.native.plugins = vec!["echo".into()];
	config
}

/// A host that runs every Rust flavour, as desktop runners will.
const RUNS_RUST: HostCapabilities = HostCapabilities {
	sync: true,
	worker: true,
	asynchronous: true,
	service: true,
	native: None,
};

#[test]
fn a_platform_table_wins_over_shared_rust_and_shared_rust_wins_over_native_types() {
	let registry = resolve_metadata(
		runtime_metadata(&json!({
			"name": "echo", "rust-type": "Shared",
			"apple-module": "Echo", "apple-type": "EchoPlugin",
			"android-type": "dev.example.EchoPlugin",
			"ios": {"rust-type": "ios::Echo"},
			"linux": {"rust-factory": "desktop::from_context", "rust-worker": true}
		})),
		&echo_config(),
	)
	.unwrap();
	let plugin = &registry.plugins[0];
	let rust = |constructor, flavour| {
		Ok(Implementation::Rust(RustRegistration {
			constructor,
			flavour,
		}))
	};

	assert_eq!(
		plugin.implementation(Target::Ios, HostCapabilities::of(Target::Ios)),
		rust(Constructor::Type("ios::Echo"), Flavour::Sync)
	);

	for target in [Target::Macos, Target::Android] {
		assert_eq!(
			plugin.implementation(target, HostCapabilities::of(target)),
			rust(Constructor::Type("Shared"), Flavour::Sync),
			"{target:?}"
		);
	}

	assert_eq!(
		plugin.implementation(Target::Linux, RUNS_RUST),
		rust(
			Constructor::Factory("desktop::from_context"),
			Flavour::Worker
		)
	);
	// The Linux runner doesn't run plugins yet.
	assert_eq!(
		plugin.implementation(Target::Linux, HostCapabilities::of(Target::Linux)),
		Err(Missing::HostCannotRun(Capability::Worker))
	);

	// Without shared Rust fields, a platform's native type is next.
	let registry = resolve_metadata(
		runtime_metadata(&json!({
			"name": "echo", "android-type": "dev.example.EchoPlugin",
			"windows": {"rust-type": "desktop::Echo"}
		})),
		&echo_config(),
	)
	.unwrap();
	let plugin = &registry.plugins[0];
	assert_eq!(
		plugin.implementation(Target::Android, HostCapabilities::of(Target::Android)),
		Ok(Implementation::Android {
			ty: "dev.example.EchoPlugin"
		})
	);
	assert_eq!(
		plugin.implementation(Target::Windows, RUNS_RUST),
		rust(Constructor::Type("desktop::Echo"), Flavour::Sync)
	);
	assert_eq!(
		plugin.implementation(Target::Linux, RUNS_RUST),
		Err(Missing::NoImplementation)
	);
}

#[test]
fn a_host_that_cannot_run_the_rust_flavour_falls_back_to_its_native_type() {
	let registry = resolve_metadata(
		runtime_metadata(&json!({
			"name": "echo", "rust-type": "Echo", "rust-worker": true,
			"harmonyos-type": "EchoPlugin"
		})),
		&echo_config(),
	)
	.unwrap();
	assert_eq!(
		registry.plugins[0]
			.implementation(Target::Harmonyos, HostCapabilities::of(Target::Harmonyos)),
		Ok(Implementation::ArkTs { ty: "EchoPlugin" })
	);
}

#[test]
fn platform_tables_reject_unknown_names_keys_and_incomplete_registration() {
	for runtime in [
		json!({"name": "echo", "web": {"rust-type": "Echo"}}),
		json!({"name": "echo", "linux": {"rust-type": "Echo", "apple-type": "Echo"}}),
		json!({"name": "echo", "linux": {"rust-service-factory": "service"}}),
		json!({"name": "echo", "linux": {}}),
		json!({"name": "echo", "linux": {"rust-worker": true}}),
		json!({"name": "echo", "linux": {"rust-type": "Echo", "rust-factory": "new"}}),
		json!({"name": "echo", "linux": {"rust-type": "Echo", "unavailable": true}}),
		json!({"name": "echo", "linux": {"rust-type": "Echo", "rust-async": true}}),
		json!({"name": "echo", "linux": {"rust-factory": "new", "rust-async": true, "rust-worker": true}}),
		json!({"name": "echo", "linux": {"rust-type": "Echo(); panic!()"}}),
		json!({"name": "echo", "linux": {"rust-factory": "crate::new"}}),
		json!({"name": "echo", "rust-service-factory": "service", "linux": {"rust-type": "Echo"}}),
	] {
		assert!(
			resolve_metadata(runtime_metadata(&runtime), &echo_config()).is_err(),
			"{runtime}"
		);
	}

	assert!(
		resolve_metadata(
			runtime_metadata(&json!({
				"name": "echo", "rust-service-factory": "service", "linux": {"unavailable": true}
			})),
			&echo_config()
		)
		.is_ok()
	);
}

#[test]
fn an_unavailable_platform_is_skipped_without_a_warning() {
	let runtime = json!({
		"name": "echo", "apple-module": "Echo", "apple-type": "EchoPlugin",
		"linux": {"unavailable": true}
	});
	let config = echo_config();

	let registry =
		resolve_for_target(runtime_metadata(&runtime), &config, Some(Target::Linux)).unwrap();
	assert!(registry.plugins.is_empty());
	assert_eq!(registry.missing[0].reason, Missing::Unavailable);
	assert_eq!(registry.warnings().count(), 0);

	let registry =
		resolve_for_target(runtime_metadata(&runtime), &config, Some(Target::Windows)).unwrap();
	assert_eq!(
		registry.warnings().collect::<Vec<_>>(),
		["warning: echo has no Windows implementation; calls fail with Unavailable"]
	);

	let registry =
		resolve_for_target(runtime_metadata(&runtime), &config, Some(Target::Macos)).unwrap();
	assert_eq!(registry.plugins.len(), 1);
	assert_eq!(registry.missing, [] as [MissingPlugin; 0]);

	// Strict apps still learn that the plugin is left out.
	let mut strict = config;
	strict.native.missing_plugins = MissingPlugins::Error;
	let error = resolve_for_target(runtime_metadata(&runtime), &strict, Some(Target::Linux))
		.err()
		.unwrap();
	assert_eq!(
		error.to_string(),
		"native.missing-plugins = \"error\" stops the build: echo is unavailable on Linux"
	);
}

#[test]
fn async_host_features_follow_the_destinations_that_register_the_plugin() {
	let mut config = echo_config();
	config
		.native
		.plugin_options
		.insert("echo".into(), toml::Table::new());
	let runtime = json!({"name": "echo", "rust-factory": "from_config", "rust-async": true});

	let android =
		resolve_for_target(runtime_metadata(&runtime), &config, Some(Target::Android)).unwrap();
	assert_eq!(
		android.host_features,
		["renamed_echo/async-host", "ffi/async-host"]
	);

	// HarmonyOS can't dispatch async calls, so it neither registers the
	// plugin nor compiles its async host.
	let harmonyos =
		resolve_for_target(runtime_metadata(&runtime), &config, Some(Target::Harmonyos)).unwrap();
	assert!(harmonyos.plugins.is_empty());
	assert_eq!(harmonyos.host_features, [] as [String; 0]);
	assert_eq!(
		harmonyos.warnings().collect::<Vec<_>>(),
		[
			"warning: echo can't run on HarmonyOS: the HarmonyOS host doesn't run async plugins \
			 yet; calls fail with Unavailable"
		]
	);

	// An async platform table needs host options like a shared declaration.
	let table = json!({
		"name": "echo", "android-type": "dev.example.Echo",
		"linux": {"rust-factory": "desktop::from_config", "rust-async": true}
	});
	assert!(resolve_metadata(runtime_metadata(&table), &echo_config()).is_err());
	let registry =
		resolve_for_target(runtime_metadata(&table), &config, Some(Target::Android)).unwrap();
	assert_eq!(registry.host_features, [] as [String; 0]);
}

#[test]
fn experimental_rust_plugins_fall_back_to_native_unless_opted_in() {
	let runtime = json!({
		"name": "echo",
		"apple-module": "Echo",
		"apple-type": "EchoPlugin",
		"android-type": "dev.example.EchoPlugin",
		"ios": {"rust-factory": "from_context", "rust-context": true, "rust-main-thread": true, "experimental": true},
		"android": {"rust-factory": "from_context", "rust-context": true, "rust-main-thread": true, "experimental": true},
		"macos": {"rust-factory": "from_context", "rust-context": true, "rust-main-thread": true, "experimental": true}
	});
	let default_config = echo_config();

	// By default `experimental-rust-plugins` is false, so stable Swift/Kotlin
	// implementations are selected.
	for (target, expected) in [
		(
			Target::Ios,
			Implementation::Apple {
				module: "Echo",
				ty: "EchoPlugin",
			},
		),
		(
			Target::Macos,
			Implementation::Apple {
				module: "Echo",
				ty: "EchoPlugin",
			},
		),
		(
			Target::Android,
			Implementation::Android {
				ty: "dev.example.EchoPlugin",
			},
		),
	] {
		let registry =
			resolve_for_target(runtime_metadata(&runtime), &default_config, Some(target)).unwrap();
		assert_eq!(
			registry.plugins[0].implementation(target, HostCapabilities::of(target)),
			Ok(expected),
			"{target:?}"
		);
	}

	// Opting into `native.experimental-rust-plugins = true` selects the pure
	// Rust `ContextFactory` implementation on each target.
	let mut opted_in = echo_config();
	opted_in.native.experimental_rust_plugins = true;

	for target in [Target::Ios, Target::Macos, Target::Android] {
		let registry =
			resolve_for_target(runtime_metadata(&runtime), &opted_in, Some(target)).unwrap();
		assert_eq!(
			registry.plugins[0].implementation(target, HostCapabilities::of(target)),
			Ok(Implementation::Rust(RustRegistration {
				constructor: Constructor::ContextFactory("from_context"),
				flavour: Flavour::MainThread,
			})),
			"{target:?}"
		);
	}
}

#[test]
fn experimental_only_rust_plugin_reports_missing_flag_when_not_opted_in() {
	let runtime = json!({
		"name": "echo",
		"ios": {"rust-factory": "from_context", "rust-context": true, "experimental": true}
	});
	let config = echo_config();
	let registry =
		resolve_for_target(runtime_metadata(&runtime), &config, Some(Target::Ios)).unwrap();
	assert!(registry.plugins.is_empty());
	assert_eq!(registry.missing[0].reason, Missing::ExperimentalRustPlugin);
	assert_eq!(
		registry.warnings().collect::<Vec<_>>(),
		[
			"warning: echo requires native.experimental-rust-plugins = true on iOS; calls fail \
			 with Unavailable"
		]
	);

	// Rejects `rust-context`, `rust-main-thread`, or `experimental` without a
	// valid Rust constructor or with conflicting flavours.
	for invalid in [
		json!({"name": "echo", "rust-type": "Echo", "rust-context": true}),
		json!({"name": "echo", "apple-module": "Echo", "apple-type": "EchoPlugin", "experimental": true}),
		json!({"name": "echo", "apple-module": "Echo", "apple-type": "EchoPlugin", "rust-main-thread": true}),
		json!({"name": "echo", "rust-type": "Echo", "rust-worker": true, "rust-main-thread": true}),
		json!({"name": "echo", "rust-factory": "new", "rust-async": true, "rust-main-thread": true}),
		json!({"name": "echo", "ios": {"rust-type": "Echo", "rust-context": true}}),
		json!({"name": "echo", "ios": {"rust-type": "Echo", "rust-worker": true, "rust-main-thread": true}}),
		json!({"name": "echo", "ios": {"rust-factory": "new", "rust-async": true, "rust-main-thread": true}}),
	] {
		assert!(
			resolve_metadata(runtime_metadata(&invalid), &echo_config()).is_err(),
			"{invalid}"
		);
	}
}

#[test]
fn watch_targets_consult_platform_tables_without_native_hosts() {
	let with_android = resolve_metadata(
		runtime_metadata(&json!({
			"name": "echo", "android-type": "dev.example.EchoPlugin"
		})),
		&echo_config(),
	)
	.unwrap();
	// Wear OS reads the Android table but has no native host kind yet, so the
	// Android type does not bind; watchOS has no table at all.
	assert_eq!(
		with_android.plugins[0].implementation(Target::Wearos, RUNS_RUST),
		Err(Missing::NoImplementation)
	);
	assert_eq!(
		with_android.plugins[0].implementation(Target::Watchos, RUNS_RUST),
		Err(Missing::NoImplementation)
	);

	let with_rust = resolve_metadata(
		runtime_metadata(&json!({
			"name": "echo", "rust-type": "Shared"
		})),
		&echo_config(),
	)
	.unwrap();
	assert_eq!(
		with_rust.plugins[0].implementation(Target::Wearos, HostCapabilities::of(Target::Wearos)),
		Err(Missing::HostCannotRun(Capability::Sync))
	);
}

#[test]
fn missing_plugin_messages_name_wear_and_watch_hosts() {
	for (target, label) in [(Target::Wearos, "Wear OS"), (Target::Watchos, "watchOS")] {
		let missing = MissingPlugin {
			name: "echo".into(),
			target,
			reason: Missing::NoImplementation,
		};
		assert!(missing.describe().contains(label), "{}", missing.describe());
		assert!(
			missing.warning().unwrap().contains(label),
			"{:?}",
			missing.warning()
		);
	}
}
