//! Configuration loading tests.

use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use leptoskit_config::AttributeBraces;
use leptoskit_config::ClosingTags;
use leptoskit_config::ConfigError;
use leptoskit_config::FormatConfig;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Optimize;
use leptoskit_config::Platform;
use leptoskit_config::RenderMode;
use leptoskit_config::default_package_id;

/// A unique scratch directory removed on drop.
struct Scratch {
	path: PathBuf,
}

impl Scratch {
	fn new(tag: &str) -> Self {
		static COUNTER: AtomicU64 = AtomicU64::new(0);
		let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
		let path = std::env::temp_dir().join(format!(
			"leptoskit_config_test_{tag}_{}_{}",
			std::process::id(),
			unique
		));
		std::fs::create_dir_all(&path).expect("create scratch dir");
		Self { path }
	}
}

impl Drop for Scratch {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.path);
	}
}

#[test]
fn parses_a_full_file() {
	let scratch = Scratch::new("full");
	std::fs::write(
		scratch.path.join("leptoskit.toml"),
		r#"
[app]
name = "counter"
version = "1.2.3"
default_platform = "native"

[serve]
port = 4200

[native]
package_id = "com.example.counter"

[codepush]
enabled = true
server = "https://cp.example.com"
channel = "staging"
public_keys = ["a0d1b6b0b6a5f6cbe4f0e2f6dbb2a1e7b0c3d8f9e1a2b3c4d5e6f708192a3b4c"]

[[plugins]]
name = "size_report"

[[plugins]]
name = "leptos_meta"
leptos = true
"#,
	)
	.expect("write config");
	let config = LeptoskitConfig::load_from(&scratch.path).expect("parse");
	assert_eq!(config.app.name, "counter");
	assert_eq!(config.app.version.to_string(), "1.2.3");
	assert_eq!(config.app.default_platform, Platform::Native);
	assert_eq!(config.serve.port, 4200);
	assert_eq!(config.native.package_id, "com.example.counter");
	assert!(config.codepush.enabled);
	assert_eq!(config.codepush.public_keys.len(), 1);
	assert_eq!(config.plugins.len(), 2);
	assert!(config.plugins[1].leptos);
}

#[test]
fn defaults_fill_from_the_directory() {
	let scratch = Scratch::new("defaults");
	std::fs::write(scratch.path.join("leptoskit.toml"), "").expect("write config");
	let config = LeptoskitConfig::load_from(&scratch.path).expect("parse");
	assert_eq!(
		config.app.name,
		scratch
			.path
			.file_name()
			.and_then(|name| name.to_str())
			.unwrap_or_default()
	);
	assert!(config.native.package_id.starts_with("com.leptoskit."));
	assert_eq!(config.app.default_platform, Platform::Web);
	assert_eq!(config.app.platforms, Platform::APPLICATION);
}

#[test]
fn platform_selection_rejects_disabled_defaults_duplicates_and_empty_lists() {
	let mut config = LeptoskitConfig::default();
	config.app.platforms = vec![Platform::Ios];
	assert!(matches!(
		config.validate(),
		Err(ConfigError::DisabledPlatform(Platform::Web))
	));
	config.app.default_platform = Platform::Ios;
	config.validate().unwrap();
	assert!(matches!(
		config.ensure_platform_enabled(Platform::Android),
		Err(ConfigError::DisabledPlatform(Platform::Android))
	));
	config.app.platforms.push(Platform::Ios);
	assert!(matches!(
		config.validate(),
		Err(ConfigError::DuplicatePlatform(Platform::Ios))
	));
	config.app.platforms.clear();
	assert!(matches!(config.validate(), Err(ConfigError::NoPlatforms)));
}

#[test]
fn concrete_platforms_round_trip_and_select_distinct_features() {
	for platform in Platform::APPLICATION
		.into_iter()
		.chain([Platform::Harmonyos, Platform::Tui])
	{
		let name = platform.to_string();
		assert_eq!(name.parse::<Platform>().unwrap(), platform);
		assert_eq!(platform.feature(), Some(name.as_str()));
		assert_eq!(platform.is_native(), platform != Platform::Web);
	}
}

#[test]
fn enabled_platforms_have_one_authoritative_list_in_new_configuration() {
	let mut config = LeptoskitConfig::default();
	let serialized = config.to_toml_string().unwrap();
	let document: toml::Value = toml::from_str(&serialized).unwrap();
	assert_eq!(
		document["app"]["platforms"],
		toml::Value::try_from(&config.app.platforms).unwrap()
	);
	assert!(document["native"].get("targets").is_none());
	config.native.targets = vec![leptoskit_config::NativeTarget::Harmonyos];
	let serialized = config.to_toml_string().unwrap();
	let restored: LeptoskitConfig = toml::from_str(&serialized).unwrap();
	assert_eq!(restored.native.targets, config.native.targets);
}

#[test]
fn native_plugin_options_roundtrip_and_require_an_explicit_grant() {
	let input = r#"
[native]
plugins = ["http"]
[native.plugin-options.http]
origins = ["https://api.example.com"]
"#;
	let config: LeptoskitConfig = toml::from_str(input).unwrap();
	config.validate().unwrap();
	assert_eq!(
		config.native.plugin_options["http"]["origins"]
			.as_array()
			.unwrap()[0]
			.as_str(),
		Some("https://api.example.com")
	);
	let roundtrip: LeptoskitConfig = toml::from_str(&config.to_toml_string().unwrap()).unwrap();
	assert_eq!(roundtrip, config);
	let mut unselected = config;
	unselected.native.plugins.clear();
	assert!(
		matches!(unselected.validate(), Err(ConfigError::UnselectedPluginOptions(name)) if name == "http")
	);
	assert!(toml::from_str::<LeptoskitConfig>("[native.plugin-options]\nhttp = \"all\"").is_err());
}

#[test]
fn unknown_fields_are_rejected() {
	let scratch = Scratch::new("unknown");
	std::fs::write(
		scratch.path.join("leptoskit.toml"),
		"[app]\nname = \"x\"\nnonexistent = true\n",
	)
	.expect("write config");
	let error = LeptoskitConfig::load_from(&scratch.path).unwrap_err();
	assert!(matches!(error, ConfigError::Parse { .. }), "got {error:?}");
}

#[test]
fn missing_file_reports_read_error() {
	let scratch = Scratch::new("missing");
	let error = LeptoskitConfig::load_from(&scratch.path).unwrap_err();
	assert!(matches!(error, ConfigError::Read { .. }), "got {error:?}");
}

#[test]
fn round_trips_through_toml() {
	let scratch = Scratch::new("roundtrip");
	std::fs::write(
		scratch.path.join("leptoskit.toml"),
		"[app]\nname = \"counter\"\n",
	)
	.expect("write config");
	let config = LeptoskitConfig::load_from(&scratch.path).expect("parse");
	let serialized = config.to_toml_string().expect("serialize");
	let reparsed: LeptoskitConfig = toml::from_str(&serialized).expect("reparse");
	assert_eq!(config, reparsed);
}

fn load(tag: &str, toml: &str) -> Result<LeptoskitConfig, ConfigError> {
	let scratch = Scratch::new(tag);
	std::fs::write(scratch.path.join("leptoskit.toml"), toml).expect("write config");
	LeptoskitConfig::load_from(&scratch.path)
}

#[test]
fn default_package_ids_are_valid_on_ios_and_android() {
	assert_eq!(default_package_id("counter"), "com.leptoskit.counter");
	assert_eq!(default_package_id("my-app"), "com.leptoskit.myapp");
	assert_eq!(default_package_id("my.cool_App"), "com.leptoskit.mycoolapp");
	assert_eq!(default_package_id("1app"), "com.leptoskit.app1app");
	assert_eq!(default_package_id("---"), "com.leptoskit.app");
	assert_eq!(default_package_id(""), "com.leptoskit.app");
	assert_eq!(default_package_id("café"), "com.leptoskit.caf");
}

#[test]
fn derived_package_id_uses_the_app_name() {
	let config = load("derived_id", "[app]\nname = \"my-app\"\n").expect("load");
	assert_eq!(config.native.package_id, "com.leptoskit.myapp");
}

#[test]
fn enabled_codepush_requires_a_server() {
	let error = load("codepush_no_server", "[codepush]\nenabled = true\n").unwrap_err();
	assert!(
		matches!(error, ConfigError::CodePushServerMissing),
		"got {error:?}"
	);
	let error = load(
		"codepush_blank_server",
		"[codepush]\nenabled = true\nserver = \"  \"\n",
	)
	.unwrap_err();
	assert!(
		matches!(error, ConfigError::CodePushServerMissing),
		"got {error:?}"
	);
	assert!(load("codepush_disabled", "[codepush]\nenabled = false\n").is_ok());
}

#[test]
fn enabled_codepush_requires_valid_public_keys() {
	let server = "[codepush]\nenabled = true\nserver = \"https://cp.example.com\"\n";
	let error = load("codepush_no_keys", server).unwrap_err();
	assert!(
		matches!(error, ConfigError::CodePushPublicKeysMissing),
		"got {error:?}"
	);
	for invalid in ["abc", &"g".repeat(64), &"a".repeat(66)] {
		let error = load(
			"codepush_bad_key",
			&format!("{server}public_keys = [\"{invalid}\"]\n"),
		)
		.unwrap_err();
		assert!(
			matches!(&error, ConfigError::InvalidCodePushPublicKey(key) if key == invalid),
			"{invalid:?}: got {error:?}"
		);
	}
	let valid = "a".repeat(64);
	assert!(
		load(
			"codepush_keys",
			&format!("{server}public_keys = [\"{valid}\"]\n")
		)
		.is_ok()
	);
}

#[test]
fn explicit_package_ids_are_validated() {
	for valid in ["com.example.counter", "com.Example.App2", "io.app"] {
		let toml = format!("[native]\npackage_id = \"{valid}\"\n");
		assert!(load("valid_id", &toml).is_ok(), "{valid}");
	}
	for invalid in [
		"counter",
		"com.my_app",
		"com.my-app",
		"com.1app",
		"com..app",
		"com.app.",
	] {
		let toml = format!("[native]\npackage_id = \"{invalid}\"\n");
		let error = load("invalid_id", &toml).unwrap_err();
		assert!(
			matches!(&error, ConfigError::InvalidPackageId { package_id, .. } if package_id == invalid),
			"{invalid}: got {error:?}"
		);
	}
}

#[test]
fn render_mode_defaults_to_csr_and_parses_every_mode() {
	assert_eq!(
		load("render-default", "").expect("parse").app.render,
		RenderMode::Csr
	);
	for (name, mode) in [
		("csr", RenderMode::Csr),
		("ssr", RenderMode::Ssr),
		("islands", RenderMode::Islands),
	] {
		let config = load("render", &format!("[app]\nrender = \"{name}\"\n")).expect("parse");
		assert_eq!(config.app.render, mode);
		assert_eq!(config.app.render.to_string(), name);
	}
	assert!(!RenderMode::Csr.renders_on_server());
	assert!(RenderMode::Ssr.renders_on_server());
	assert!(RenderMode::Islands.renders_on_server());
}

#[test]
fn unknown_render_modes_and_the_old_ssr_flags_are_rejected() {
	for toml in [
		"[app]\nrender = \"spa\"\n",
		"[app]\nssr = true\n",
		"[app]\nhydration = true\n",
	] {
		let error = load("render-bad", toml).unwrap_err();
		assert!(
			matches!(error, ConfigError::Parse { .. }),
			"{toml}: {error:?}"
		);
	}
}

#[test]
fn bundle_settings_default_to_fast_and_parse_smallest() {
	let defaults = load("bundle-default", "").expect("parse").bundle;
	assert_eq!(defaults.optimize, Optimize::Fast);
	assert!(defaults.wasm_opt && defaults.precompress);
	assert!(defaults.split);
	assert_eq!((defaults.cargo_profile, defaults.budget_kib), (None, None));

	let bundle = load(
		"bundle",
		"[bundle]\noptimize = \"smallest\"\ncargo_profile = \"wasm-release\"\nwasm_opt = \
		 false\nprecompress = false\nbudget_kib = 80\nsplit = false\n",
	)
	.expect("parse")
	.bundle;
	assert_eq!(bundle.optimize, Optimize::Smallest);
	assert_eq!(bundle.cargo_profile.as_deref(), Some("wasm-release"));
	assert!(!bundle.wasm_opt && !bundle.precompress);
	assert!(!bundle.split);
	assert_eq!(bundle.budget_kib, Some(80));

	assert_eq!(
		"smallest".parse::<Optimize>().expect("parse"),
		Optimize::Smallest
	);
	assert_eq!(Optimize::Fast.to_string(), "fast");
	assert!("tiny".parse::<Optimize>().is_err());
}

#[test]
fn bundle_settings_live_in_bundle_not_build() {
	for toml in [
		"[build]\nbudget_kib = 80\n",
		"[bundle]\noptimize = \"tiny\"\n",
	] {
		let error = load("bundle-bad", toml).unwrap_err();
		assert!(
			matches!(error, ConfigError::Parse { .. }),
			"{toml}: {error:?}"
		);
	}
}

#[test]
fn a_key_in_its_old_table_names_where_it_moved() {
	let error = load("moved", "[build]\nwasm_opt = false\n").unwrap_err();
	assert!(matches!(error, ConfigError::MovedKey { .. }), "{error:?}");
	let message = error.to_string();
	assert!(message.contains("build.wasm_opt"), "{message}");
	assert!(message.contains("bundle.wasm_opt"), "{message}");
	assert!(message.contains("[bundle]"), "{message}");
}

#[test]
fn the_router_is_off_until_its_table_appears() {
	let config = load("router_off", "[app]\nname = \"field\"\n").expect("load");
	assert_eq!(config.router, None);
	assert!(
		!config
			.to_toml_string()
			.expect("serialize")
			.contains("[router]"),
		"an absent router stays absent"
	);

	let config = load("router_on", "[router]\n").expect("load");
	let router = config.router.expect("an empty table turns the router on");
	assert_eq!(router.dir, PathBuf::from("src/routes"));
	assert_eq!(router.output(), PathBuf::from("src/routes.rs"));

	let config = load("router_dir", "[router]\ndir = \"client/src/pages\"\n").expect("load");
	assert_eq!(
		config.router.expect("router").output(),
		PathBuf::from("client/src/pages.rs")
	);
}

#[test]
fn the_router_dir_stays_inside_the_project() {
	for dir in [
		"",
		"/abs/routes",
		"../routes",
		"src/../routes",
		"./src/routes",
		"src/my-routes",
		"src/routes.v2",
	] {
		let error = load("router_bad_dir", &format!("[router]\ndir = \"{dir}\"\n")).unwrap_err();
		assert!(
			matches!(error, ConfigError::InvalidRouterDir(_)),
			"{dir:?}: got {error:?}"
		);
	}
	let error = load("router_unknown_key", "[router]\nout = \"src/routes.rs\"\n").unwrap_err();
	assert!(matches!(error, ConfigError::Parse { .. }), "got {error:?}");
}

#[test]
fn translations_are_off_until_their_table_appears() {
	let config = load("l10n_off", "[app]\nname = \"field\"\n").expect("load");
	assert_eq!(config.l10n, None);
	assert!(
		!config
			.to_toml_string()
			.expect("serialize")
			.contains("[l10n]"),
		"absent translations stay absent"
	);

	let config = load("l10n_on", "[l10n]\n").expect("load");
	let l10n = config.l10n.expect("an empty table turns translations on");
	assert_eq!(l10n.dir, PathBuf::from("locales"));
	assert_eq!(l10n.output, PathBuf::from("src/l10n.rs"));
	assert_eq!(l10n.template, "en");

	let config = load(
		"l10n_custom",
		"[l10n]\ndir = \"app/translations\"\noutput = \"app/src/messages.rs\"\ntemplate = \
		 \"pt-BR\"\n",
	)
	.expect("load");
	let l10n = config.l10n.expect("l10n");
	assert_eq!(l10n.dir, PathBuf::from("app/translations"));
	assert_eq!(l10n.output, PathBuf::from("app/src/messages.rs"));
	assert_eq!(l10n.template, "pt-BR");
}

#[test]
fn translation_paths_stay_inside_the_project() {
	for dir in [
		"",
		"/abs/locales",
		"../locales",
		"src/../locales",
		"./locales",
	] {
		let error = load("l10n_bad_dir", &format!("[l10n]\ndir = \"{dir}\"\n")).unwrap_err();
		assert!(
			matches!(error, ConfigError::InvalidL10nDir(_)),
			"{dir:?}: got {error:?}"
		);
	}
	for output in [
		"",
		"/abs/l10n.rs",
		"../l10n.rs",
		"src/l10n",
		"src/l10n.txt",
		"src/my-l10n.rs",
		"src/9l10n.rs",
	] {
		let error = load(
			"l10n_bad_output",
			&format!("[l10n]\noutput = \"{output}\"\n"),
		)
		.unwrap_err();
		assert!(
			matches!(error, ConfigError::InvalidL10nOutput(_)),
			"{output:?}: got {error:?}"
		);
	}
	for template in ["", "en_US", "pt-", "-", "en US"] {
		let error = load(
			"l10n_bad_template",
			&format!("[l10n]\ntemplate = \"{template}\"\n"),
		)
		.unwrap_err();
		assert!(
			matches!(error, ConfigError::InvalidL10nTemplate(_)),
			"{template:?}: got {error:?}"
		);
		assert!(error.to_string().contains("l10n.template"), "{error}");
	}
	let error = load("l10n_unknown_key", "[l10n]\nfallback = \"en\"\n").unwrap_err();
	assert!(matches!(error, ConfigError::Parse { .. }), "got {error:?}");
}

#[test]
fn format_table_is_optional_and_typed() {
	let config = load("format-default", "[app]\nname = \"demo\"\n").expect("parse");
	assert!(config.format.is_default());
	assert!(
		!config
			.to_toml_string()
			.expect("serialize")
			.contains("[format]"),
		"default formatter settings stay out of serialized configuration"
	);

	let config = load(
		"format-custom",
		"[format]\nmacros = [\"html\"]\nattribute_braces = \"always\"\nclosing_tags = \
		 \"explicit\"\nrustfmt_toolchain = \"stable\"\n",
	)
	.expect("parse");
	assert_eq!(config.format.macros, vec!["html".to_owned()]);
	assert_eq!(config.format.attribute_braces, AttributeBraces::Always);
	assert_eq!(config.format.closing_tags, ClosingTags::Explicit);
	assert_eq!(config.format.rustfmt_toolchain, "stable");

	let error = load("format-unknown", "[format]\nmax_width = 80\n").expect_err("rejects");
	assert!(matches!(error, ConfigError::Parse { .. }), "{error}");
}

#[test]
fn format_table_loads_without_the_rest_of_the_file() {
	let scratch = Scratch::new("format-only");
	let path = scratch.path.join("leptoskit.toml");
	std::fs::write(
		&path,
		"[codepush]\nenabled = true\n[format]\nclosing_tags = \"preserve\"\n",
	)
	.expect("write config");
	let format = FormatConfig::load_from_file(&path).expect("parse");
	assert_eq!(format.closing_tags, ClosingTags::Preserve);
	assert!(FormatConfig::load_from_file(&scratch.path.join("missing.toml")).is_err());
}

#[test]
fn explicit_contract_versions_must_match_the_supported_version() {
	assert_eq!(
		LeptoskitConfig::default().schema_version,
		leptoskit_config::SCHEMA_VERSION
	);
	assert!(load("current-version", "schema_version = 0\n").is_ok());

	for version in [1, 2] {
		let error = load(
			"unsupported-version",
			&format!("schema_version = {version}\n"),
		)
		.expect_err("unsupported version");
		assert!(matches!(error, ConfigError::SchemaVersion(found) if found == version));
	}
}

#[test]
fn rpc_targets_must_be_known() {
	let error = load(
		"rpc-bad-target",
		"[app]\nname = \"todos\"\n\n[[rpc.targets]]\nlanguage = \"cobol\"\n",
	)
	.expect_err("unknown target");
	assert!(
		matches!(error, ConfigError::UnknownRpcTarget(ref name) if name == "cobol"),
		"got {error}"
	);
}

#[test]
fn rpc_scaffolds_must_be_known() {
	let error = load(
		"rpc-bad-scaffold",
		"[app]\nname = \"todos\"\n\n[[rpc.targets]]\nlanguage = \"rust\"\nscaffold = \"maybe\"\n",
	)
	.expect_err("unknown scaffold");
	assert!(
		matches!(error, ConfigError::UnknownRpcScaffold(ref mode) if mode == "maybe"),
		"got {error}"
	);
	assert!(
		error
			.to_string()
			.contains("expected create, full, or files"),
		"got {error}"
	);
}
