//! Configuration loading tests.

use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use leptoskit_config::AttributeBraces;
use leptoskit_config::ClosingTags;
use leptoskit_config::ConfigError;
use leptoskit_config::FormatConfig;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::LintConfig;
use leptoskit_config::LintLevel;
use leptoskit_config::MarkupStyle;
use leptoskit_config::MissingPlugins;
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
fn missing_plugins_warn_by_default_and_can_stop_the_build() {
	let config: LeptoskitConfig = toml::from_str("").unwrap();
	assert_eq!(config.native.missing_plugins, MissingPlugins::Warn);
	assert!(
		!toml::to_string(&config)
			.unwrap()
			.contains("missing-plugins")
	);

	let config: LeptoskitConfig = toml::from_str("[native]\nmissing-plugins = \"error\"").unwrap();
	assert_eq!(config.native.missing_plugins, MissingPlugins::Error);
	assert!(
		toml::to_string(&config)
			.unwrap()
			.contains("missing-plugins = \"error\"")
	);
	assert!(toml::from_str::<LeptoskitConfig>("[native]\nmissing-plugins = \"ignore\"").is_err());
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
fn lint_table_sets_markup_rules_and_exclusions() {
	let config = load("lint-defaults", "").expect("parse");
	assert_eq!(config.lint, LintConfig::default());
	assert_eq!(config.lint.markup, MarkupStyle::View);
	assert!(config.lint.clippy);
	assert!(
		!config
			.to_toml_string()
			.expect("serialize")
			.contains("[lint]"),
		"defaults stay out of the file"
	);

	let config = load(
		"lint-set",
		"[lint]\nmarkup = \"any\"\nclippy = false\nexclude = \
		 [\"tests/**\"]\n[lint.rules]\ncomponent_attribute = \"deny\"\nmarkup_style = \"allow\"\n",
	)
	.expect("parse");
	assert_eq!(config.lint.markup, MarkupStyle::Any);
	assert!(!config.lint.clippy);
	assert_eq!(config.lint.exclude, ["tests/**"]);
	assert_eq!(config.lint.rules.component_attribute, LintLevel::Deny);
	assert_eq!(config.lint.rules.markup_style, LintLevel::Allow);

	for invalid in [
		"[lint]\nmarkup = \"html\"\n",
		"[lint.rules]\nunknown_rule = \"warn\"\n",
		"[lint.rules]\nmarkup_style = \"error\"\n",
	] {
		let error = load("lint-invalid", invalid).expect_err("rejects");
		assert!(matches!(error, ConfigError::Parse { .. }), "{error}");
	}
}

#[test]
fn lint_table_loads_without_the_rest_of_the_file() {
	let scratch = Scratch::new("lint-only");
	let path = scratch.path.join("leptoskit.toml");
	std::fs::write(&path, "[app]\nname = 7\n[lint]\nmarkup = \"builder\"\n").expect("write config");
	let lint = LintConfig::load_from_file(&path).expect("parse");
	assert_eq!(lint.markup, MarkupStyle::Builder);
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

#[test]
fn the_database_table_turns_on_migrations() {
	let config = load("db_off", "").expect("load");
	assert!(config.db.is_none());

	let config = load("db_on", "[db]\n").expect("load");
	let db = config.db.expect("an empty table turns the database on");
	assert_eq!(db.dialect, leptoskit_config::DbDialect::Postgres);
	assert_eq!(db.migrations, PathBuf::from("migrations"));
	assert_eq!(db.url_env, "DATABASE_URL");
	assert_eq!(db.features, ["server"]);
	assert_eq!(
		db.package(&config.app),
		format!("{}_db", config.app.name.replace('-', "_"))
	);
	assert_eq!(db.dialect.as_str(), "postgres");

	for dialect in [
		leptoskit_config::DbDialect::Postgres,
		leptoskit_config::DbDialect::Sqlite,
	] {
		assert_eq!(
			dialect
				.to_string()
				.parse::<leptoskit_config::DbDialect>()
				.unwrap(),
			dialect
		);
	}
	assert_eq!(
		"mysql"
			.parse::<leptoskit_config::DbDialect>()
			.unwrap_err()
			.to_string(),
		"unknown database `mysql`; expected postgres or sqlite"
	);

	let config = load(
		"db_custom",
		"[db]\ndialect = \"sqlite\"\ncrate = \"store\"\nfeatures = [\"server\", \
		 \"sqlite\"]\nmigrations = \"db/migrations\"\nurl_env = \"STORE_URL\"\n",
	)
	.expect("load");
	let db = config.db.expect("db");
	assert_eq!(db.dialect.as_str(), "sqlite");
	assert_eq!(db.package(&config.app), "store");
	assert_eq!(db.features, ["server", "sqlite"]);
	assert_eq!(db.migrations, PathBuf::from("db/migrations"));
	assert_eq!(db.url_env, "STORE_URL");
}

#[test]
fn database_settings_are_checked() {
	for migrations in ["", "/abs/migrations", "../migrations"] {
		let error = load(
			"db_bad_migrations",
			&format!("[db]\nmigrations = \"{migrations}\"\n"),
		)
		.unwrap_err();
		assert!(
			matches!(error, ConfigError::InvalidDbMigrations(_)),
			"{migrations:?}: got {error:?}"
		);
	}

	for package in ["", "my app", "app::db"] {
		let error = load("db_bad_crate", &format!("[db]\ncrate = \"{package}\"\n")).unwrap_err();
		assert!(
			matches!(error, ConfigError::InvalidDbPackage(_)),
			"{package:?}: got {error:?}"
		);
	}

	let error = load("db_bad_env", "[db]\nurl_env = \"\"\n").unwrap_err();
	assert!(matches!(error, ConfigError::InvalidDbUrlEnv));
	assert!(error.to_string().contains("DATABASE_URL"));

	let error = load("db_bad_dialect", "[db]\ndialect = \"mysql\"\n").unwrap_err();
	assert!(format!("{error:?}").contains("mysql"), "{error:?}");
}

#[test]
fn the_auth_table_names_secrets_by_variable() {
	let config = load("auth_off", "").expect("load");
	assert!(config.auth.is_none());

	let config = load("auth_on", "[auth]\n").expect("load");
	let auth = config.auth.expect("an empty table turns sign-in on");
	assert_eq!(auth, leptoskit_config::AuthConfig::default());
	assert_eq!(auth.keys_env, "AUTH_ENCRYPTION_KEYS");
	assert_eq!(
		(auth.idle_days, auth.absolute_days, auth.access_minutes),
		(14, 90, 15)
	);

	let config = load(
		"auth_custom",
		"[auth]\napp_name = \"Notes\"\norigin = \"https://notes.example\"\nclient_ip_header = \
		 \"fly-client-ip\"\nidle_days = 7\nabsolute_days = 30\n\n[auth.mail]\nfrom = \"Notes \
		 <hi@notes.example>\"\nsmtp_url_env = \"SMTP_URL\"\n",
	)
	.expect("load");
	let auth = config.auth.expect("auth");
	assert_eq!(auth.app_name.as_deref(), Some("Notes"));
	assert_eq!(auth.mail.smtp_url_env.as_deref(), Some("SMTP_URL"));

	for origin in [
		"http://localhost:3000",
		"http://127.0.0.1",
		"https://notes.example/",
	] {
		load("auth_origin", &format!("[auth]\norigin = \"{origin}\"\n")).expect(origin);
	}

	let embedded =
		LeptoskitConfig::from_toml("[auth]\n", std::path::Path::new("notes")).expect("embedded");
	assert_eq!(embedded.app.name, "notes");
	assert!(embedded.auth.is_some());

	let providers = load(
		"auth_providers",
		"[auth]\nmfa = \"required\"\nrequired_for = [\"Admin\"]\n\n[auth.providers.google]\nclient_id_env \
		 = \"GOOGLE_CLIENT_ID\"\nclient_secret_env = \"GOOGLE_CLIENT_SECRET\"\n\n[auth.providers.\
		 apple]\nclient_id_env = \"A\"\nteam_id_env = \"T\"\nkey_id_env = \"K\"\nprivate_key_env = \
		 \"P\"\n\n[auth.providers.company]\nname = \"Company SSO\"\nissuer = \
		 \"https://sso.example\"\nclient_id_env = \"SSO_ID\"\n\n[auth.providers.microsoft]\
		 \nclient_id_env = \"M\"\nclient_secret_env = \"S\"\ntenant = \"consumers\"\n",
	)
	.expect("providers");
	let auth = providers.auth.expect("auth");
	assert_eq!(auth.mfa, leptoskit_config::AuthMfa::Required);
	assert_eq!(auth.required_for, ["Admin"]);
	assert_eq!(auth.providers.len(), 4);
	assert_eq!(auth.providers["company"].kind("company"), "oidc");
	assert_eq!(auth.providers["google"].kind("google"), "google");
	assert_eq!(
		auth.providers["google"].env_names(),
		[
			("client_id_env", "GOOGLE_CLIENT_ID"),
			("client_secret_env", "GOOGLE_CLIENT_SECRET")
		]
	);

	let webhook =
		"[auth.mail]\nwebhook_url_env = \"MAIL_URL\"\nwebhook_secret_env = \"MAIL_SECRET\"\n";
	load("auth_webhook", webhook).expect("a webhook with its secret");

	let solana = load(
		"auth_solana",
		"[auth]\n\n[auth.solana]\nsign_in = true\nchain_id = \"devnet\"\n",
	)
	.expect("solana");
	let auth = solana.auth.expect("auth");
	assert!(auth.solana.sign_in);
	assert_eq!(auth.solana.chain_id.as_deref(), Some("devnet"));
}

#[test]
fn auth_settings_are_checked() {
	for (settings, message) in [
		(
			"origin = \"http://notes.example\"",
			"only localhost may use `http://`",
		),
		(
			"origin = \"https://notes.example/sign-in\"",
			"must be `https://` and a host",
		),
		("origin = \"https://\"", "must be `https://`"),
		("keys_env = \"\"", "keys_env"),
		("idle_days = 0", "idle_days"),
		("idle_days = 100", "idle_days"),
		("access_minutes = 0", "access_minutes"),
		("access_minutes = 61", "access_minutes"),
		("[auth.mail]\nsmtp_url_env = \"SMTP_URL\"", "from is needed"),
		(
			"[auth.mail]\nwebhook_url_env = \"URL\"",
			"webhook_secret_env is needed",
		),
		(
			"[auth.mail]\nfrom = \"a@b.c\"\nsmtp_url_env = \"A\"\nwebhook_url_env = \
			 \"B\"\nwebhook_secret_env = \"C\"",
			"not both",
		),
		(
			"[auth.mail]\nsmtp_url_env = \"\"",
			"must name environment variables",
		),
		("smtp_password = \"hunter2\"", "could not parse"),
		("mfa = \"sometimes\"", "could not parse"),
		("required_for = [\"\"]", "required_for"),
		(
			"[auth.providers.Google]\nclient_id_env = \"A\"",
			"lowercase letters",
		),
		(
			"[auth.providers.google]\nclient_id_env = \"A\"",
			"needs `client_secret_env`",
		),
		(
			"[auth.providers.github]\nclient_id_env = \"\"\nclient_secret_env = \"B\"",
			"must name environment variables",
		),
		(
			"[auth.providers.apple]\nclient_id_env = \"A\"",
			"needs `team_id_env`",
		),
		(
			"[auth.providers.sso]\nissuer = \"http://sso.example\"\nclient_id_env = \"A\"",
			"must be an `https://` URL",
		),
		(
			"[auth.providers.sso]\nissuer = \"https://sso.example\"",
			"needs `client_id_env`",
		),
		(
			"[auth.providers.x]\nkind = \"myspace\"",
			"isn't one of google",
		),
		(
			"[auth.providers.google]\nclient_id_env = \"A\"\nclient_secret = \"inline\"",
			"could not parse",
		),
		("[auth.solana]\nchain_id = \"main net\"", "chain_id"),
		("[auth.solana]\nchain_id = \"\"", "chain_id"),
		("[auth.solana]\nrpc_url = \"https://x\"", "could not parse"),
	] {
		let error = load("auth_bad", &format!("[auth]\n{settings}\n")).unwrap_err();
		assert!(error.to_string().contains(message), "{settings}: {error}");
	}
}
