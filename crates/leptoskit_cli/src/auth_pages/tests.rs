use std::fs;
use std::path::Path;

use leptoskit_config::LeptoskitConfig;

use super::*;

const LIB: &str = "//! The app.\n\npub mod routes;\n";

/// A workspace like the scaffold's: a server at the root and the client in
/// `app_app`, with `leptoskit.toml` saying so. `db` adds a `[db]` table.
fn workspace(db: bool) -> tempfile::TempDir {
	let root = tempfile::tempdir().unwrap();
	let path = root.path();
	let db = if db {
		"\n[db]\ndialect = \"sqlite\"\n"
	} else {
		""
	};

	fs::write(
		path.join("leptoskit.toml"),
		format!(
			"schema_version = 0\n\n[app]\nname = \"app\"\n\n[build]\nclient_package = \
			 \"app_app\"\n{db}"
		),
	)
	.unwrap();
	fs::write(
		path.join("Cargo.toml"),
		"[package]\nname = \"app\"\n\n[workspace]\nmembers = \
		 [\"app_app\"]\n\n[dependencies]\nleptoskit = { version = \"0.1\", features = \
		 [\"server\"] }\n",
	)
	.unwrap();
	fs::create_dir_all(path.join("src")).unwrap();
	fs::write(path.join("src/main.rs"), "fn main() {}\n").unwrap();

	fs::create_dir_all(path.join("app_app/src")).unwrap();
	fs::write(
		path.join("app_app/Cargo.toml"),
		"[package]\nname = \"app_app\"\n\n[dependencies]\nleptoskit = \"0.1\"\n",
	)
	.unwrap();
	fs::write(path.join("app_app/src/lib.rs"), LIB).unwrap();
	root
}

fn run(root: &Path, force: bool) -> Report {
	let config = LeptoskitConfig::load_from(root).unwrap();
	generate(root, &config, false, force).unwrap()
}

fn read(root: &Path, path: &str) -> String {
	fs::read_to_string(root.join(path)).unwrap()
}

#[test]
fn the_screens_and_config_land_in_the_client_crate() {
	let root = workspace(false);
	let report = run(root.path(), false);

	assert_eq!(
		report.changed,
		[
			"created app_app/src/auth/mod.rs",
			"created app_app/src/auth/sign_in.rs",
			"created app_app/src/auth/devices.rs",
			"created app_app/src/auth/security.rs",
			"updated app_app/src/lib.rs: declared `pub mod auth;`",
			"updated app_app/Cargo.toml: added the leptoskit features auth, leptos, query",
			"updated leptoskit.toml: added [auth]",
		]
	);
	assert_eq!(report.kept, [] as [String; 0]);
	assert_eq!(
		read(root.path(), "app_app/src/auth/sign_in.rs"),
		pages(false)[1].1
	);
	assert!(read(root.path(), "app_app/src/lib.rs").ends_with(
		"pub mod routes;\n\n/// Written by `leptoskit generate auth`.\npub mod auth;\n"
	));
	assert!(read(root.path(), "app_app/Cargo.toml").contains(
		"leptoskit = { version = \"0.1\", features = [\"auth\", \"leptos\", \"query\"] }"
	));

	let config = LeptoskitConfig::load_from(root.path()).unwrap();
	assert_eq!(config.auth.unwrap().keys_env, "AUTH_ENCRYPTION_KEYS");

	// No database: the server's side waits, and says why.
	assert!(!root.path().join("src/sign_in.rs").exists());
	assert!(
		report
			.next
			.iter()
			.any(|step| step.contains("Add a `[db]` table"))
	);
	assert!(
		report
			.next
			.iter()
			.any(|step| step.contains("auth::provide()"))
	);
	let printed = report.to_string();
	assert!(
		printed.starts_with("created app_app/src/auth/mod.rs\n"),
		"{printed}"
	);
	assert!(
		printed.contains("\nnext:\n  1. In app_app/src/lib.rs, call"),
		"{printed}"
	);
}

#[test]
fn running_again_changes_nothing() {
	let root = workspace(true);
	run(root.path(), false);
	let before = [
		"app_app/src/lib.rs",
		"app_app/Cargo.toml",
		"Cargo.toml",
		"leptoskit.toml",
		"src/main.rs",
		"src/sign_in.rs",
	]
	.map(|path| read(root.path(), path));

	let report = run(root.path(), false);

	assert!(report.changed.is_empty(), "{:?}", report.changed);
	assert!(report.kept.is_empty(), "{:?}", report.kept);
	assert!(
		report
			.to_string()
			.starts_with("nothing changed: the sign-in screens are already in place\n")
	);
	let after = [
		"app_app/src/lib.rs",
		"app_app/Cargo.toml",
		"Cargo.toml",
		"leptoskit.toml",
		"src/main.rs",
		"src/sign_in.rs",
	]
	.map(|path| read(root.path(), path));
	assert_eq!(before, after);
}

#[test]
fn a_server_with_a_database_gets_its_sign_in_module() {
	let root = workspace(true);
	let report = run(root.path(), false);

	assert!(
		report
			.changed
			.contains(&"created src/sign_in.rs".to_owned())
	);
	assert!(
		report
			.changed
			.contains(&"updated src/main.rs: declared `mod sign_in;`".to_owned())
	);
	assert!(
		report
			.changed
			.contains(&"updated Cargo.toml: added the leptoskit feature auth".to_owned())
	);
	assert_eq!(
		read(root.path(), "src/sign_in.rs"),
		server_module("../leptoskit.toml", false)
	);
	assert!(read(root.path(), "src/sign_in.rs").contains("include_str!(\"../leptoskit.toml\")"));
	assert!(read(root.path(), "Cargo.toml").contains("features = [\"server\", \"auth\"]"));
	assert!(
		report
			.next
			.iter()
			.any(|step| step.contains("sign_in::service::<User>"))
	);
	assert!(
		report
			.next
			.iter()
			.any(|step| step.contains("AUTH_ENCRYPTION_KEYS"))
	);

	// Once main.rs builds the service, the steps stop asking for it.
	let main = read(root.path(), "src/main.rs");
	fs::write(
		root.path().join("src/main.rs"),
		main.replace(
			"fn main() {}",
			"fn main() { let _ = sign_in::service::<()>; }",
		),
	)
	.unwrap();
	let report = run(root.path(), false);
	assert!(
		!report
			.next
			.iter()
			.any(|step| step.contains("sign_in::service"))
	);
}

#[test]
fn a_server_with_passwords_on_hashes_them() {
	let root = workspace(true);
	let config = root.path().join("leptoskit.toml");
	let text = read(root.path(), "leptoskit.toml");
	fs::write(
		&config,
		format!("{text}\n[auth]\n\n[auth.passwords]\nenabled = true\n"),
	)
	.unwrap();
	let report = run(root.path(), false);
	assert!(
		report.changed.contains(
			&"updated Cargo.toml: added the leptoskit features auth, auth-passwords".to_owned()
		),
		"{report}"
	);
	assert!(
		!read(root.path(), "app_app/Cargo.toml").contains("auth-passwords"),
		"never in the browser's build"
	);
}

#[test]
fn edited_screens_are_kept_unless_forced() {
	let root = workspace(false);
	run(root.path(), false);
	let page = root.path().join("app_app/src/auth/devices.rs");
	fs::write(&page, "// mine\n").unwrap();

	let report = run(root.path(), false);
	assert_eq!(
		report.kept,
		["kept app_app/src/auth/devices.rs: it differs from the template; --force replaces it"]
	);
	assert_eq!(fs::read_to_string(&page).unwrap(), "// mine\n");

	let report = run(root.path(), true);
	assert_eq!(report.changed, ["replaced app_app/src/auth/devices.rs"]);
	assert_eq!(fs::read_to_string(&page).unwrap(), pages(false)[2].1);
}

#[test]
fn an_auth_module_of_the_apps_own_stops_it() {
	let root = workspace(false);
	fs::write(root.path().join("app_app/src/auth.rs"), "").unwrap();
	let config = LeptoskitConfig::load_from(root.path()).unwrap();
	let error = generate(root.path(), &config, false, false).unwrap_err();
	assert!(
		error
			.to_string()
			.contains("app_app/src/auth.rs already exists")
	);
	assert!(!root.path().join("app_app/src/auth").exists());
}

#[test]
fn a_directory_in_the_way_is_refused() {
	let root = workspace(false);
	fs::create_dir_all(root.path().join("app_app/src/auth/mod.rs")).unwrap();
	let config = LeptoskitConfig::load_from(root.path()).unwrap();
	let error = generate(root.path(), &config, false, false).unwrap_err();
	assert!(
		error.to_string().contains("isn't an ordinary file"),
		"{error}"
	);
}

#[test]
fn the_file_router_gets_both_routes() {
	let root = workspace(false);
	let path = root.path();
	let mut config = read(path, "leptoskit.toml");
	config.push_str(
		"\n[[plugins]]\nname = \"router\"\n\n[plugins.options]\ndir = \"app_app/src/routes\"\n",
	);
	fs::write(path.join("leptoskit.toml"), config).unwrap();
	fs::create_dir_all(path.join("app_app/src/routes")).unwrap();
	fs::write(
		path.join("app_app/src/routes/index.rs"),
		"use leptoskit::prelude::*;\n\npub fn page() -> Element {\n\tView()\n}\n",
	)
	.unwrap();

	let report = run(path, false);

	assert!(
		report
			.changed
			.contains(&"created app_app/src/routes/sign-in.rs".to_owned())
	);
	assert!(
		report
			.changed
			.contains(&"created app_app/src/routes/devices.rs".to_owned())
	);
	assert!(
		report
			.changed
			.contains(&"created app_app/src/routes/security.rs".to_owned())
	);
	assert_eq!(
		read(path, "app_app/src/routes/sign-in.rs"),
		routes(false)[0].1
	);
	let module = read(path, "app_app/src/routes.rs");
	assert!(module.contains("SignIn"), "{module}");
	assert!(module.contains("Devices"), "{module}");
}

#[test]
fn routes_the_app_already_has_are_left_alone() {
	let root = workspace(false);
	let path = root.path();
	let mut config = read(path, "leptoskit.toml");
	config.push_str("\n[router]\ndir = \"app_app/src/routes\"\n");
	fs::write(path.join("leptoskit.toml"), config).unwrap();
	fs::create_dir_all(path.join("app_app/src/routes/(auth)")).unwrap();
	fs::write(path.join("app_app/src/routes/(auth)/sign-in.rs"), "").unwrap();
	// A routes directory the router can't read: regenerating fails, and the
	// report says how to retry.
	fs::write(path.join("app_app/src/routes/[broken.rs"), "").unwrap();

	let report = run(path, false);

	assert_eq!(
		report.kept,
		["kept app_app/src/routes/(auth)/sign-in.rs: the app already has that route"]
	);
	assert!(!path.join("app_app/src/routes/sign-in.rs").exists());
	assert!(path.join("app_app/src/routes/devices.rs").exists());
	assert!(path.join("app_app/src/routes/security.rs").exists());
	assert!(
		report
			.next
			.iter()
			.any(|step| step.contains("leptoskit plugin:router routes")),
		"{:?}",
		report.next
	);
}

#[test]
fn a_single_crate_app_is_its_own_client() {
	let root = tempfile::tempdir().unwrap();
	let path = root.path();
	fs::write(
		path.join("leptoskit.toml"),
		"schema_version = 0\n\n[app]\nname = \"solo\"\n\n[auth]\norigin = \"https://solo.example\"\n",
	)
	.unwrap();
	fs::write(
		path.join("Cargo.toml"),
		"[package]\nname = \"solo\"\n\n[dependencies.leptoskit]\nworkspace = true\nfeatures = \
		 [\"query\"]\n",
	)
	.unwrap();
	fs::create_dir_all(path.join("src")).unwrap();
	fs::write(
		path.join("src/main.rs"),
		"mod auth;\nfn main() { auth::provide(); }",
	)
	.unwrap();

	let report = run(path, false);

	assert_eq!(
		report.changed,
		[
			"created src/auth/mod.rs",
			"created src/auth/sign_in.rs",
			"created src/auth/devices.rs",
			"created src/auth/security.rs",
			"updated Cargo.toml: added the leptoskit features auth, leptos",
		]
	);
	assert!(report.next.is_empty(), "{:?}", report.next);
	assert!(read(path, "Cargo.toml").contains("features = [\"query\", \"auth\", \"leptos\"]"));
	assert!(!read(path, "leptoskit.toml").contains("keys_env"));
}

#[test]
fn the_client_crate_must_be_found() {
	let root = workspace(false);
	let path = root.path();
	let mut config = read(path, "leptoskit.toml");
	config = config.replace("app_app", "missing_app");
	fs::write(path.join("leptoskit.toml"), config).unwrap();
	let config = LeptoskitConfig::load_from(path).unwrap();
	let error = generate(path, &config, false, false).unwrap_err();
	assert!(
		error
			.to_string()
			.contains("couldn't find the client package `missing_app`"),
		"{error}"
	);

	let bare = tempfile::tempdir().unwrap();
	fs::write(
		bare.path().join("leptoskit.toml"),
		"schema_version = 0\n\n[app]\nname = \"bare\"\n",
	)
	.unwrap();
	let config = LeptoskitConfig::load_from(bare.path()).unwrap();
	let error = generate(bare.path(), &config, false, false).unwrap_err();
	assert!(error.to_string().contains("there's no package"), "{error}");
}

#[test]
fn the_client_crate_needs_a_root_file_and_leptoskit() {
	let root = workspace(false);
	let path = root.path();
	fs::remove_file(path.join("app_app/src/lib.rs")).unwrap();
	let config = LeptoskitConfig::load_from(path).unwrap();
	let error = generate(path, &config, false, false).unwrap_err();
	assert!(
		error
			.to_string()
			.contains("has no src/lib.rs or src/main.rs")
	);

	fs::write(path.join("app_app/src/lib.rs"), LIB).unwrap();
	fs::write(
		path.join("app_app/Cargo.toml"),
		"[package]\nname = \"app_app\"\n",
	)
	.unwrap();
	let error = generate(path, &config, false, false).unwrap_err();
	assert!(
		error
			.to_string()
			.contains("app_app/Cargo.toml doesn't depend on leptoskit"),
		"{error:#}"
	);
}

#[test]
fn a_config_the_table_would_break_is_left_alone() {
	let root = workspace(false);
	let path = root.path();
	let config = LeptoskitConfig::load_from(path).unwrap();
	// Another `[auth]` table that the loaded configuration didn't see.
	let mut raw = read(path, "leptoskit.toml");
	raw.push_str("\n[auth]\nkeys_env = \"OTHER\"");
	fs::write(path.join("leptoskit.toml"), &raw).unwrap();
	let error = generate(path, &config, false, false).unwrap_err();
	assert!(
		error.to_string().contains("adding [auth] would break"),
		"{error}"
	);

	assert_eq!(read(path, "leptoskit.toml"), raw);
}

#[test]
fn features_join_every_dependency_shape() {
	let mut document: toml_edit::DocumentMut = "[dependencies]\nleptoskit = { version = \"0.1\", \
	                                            features = [\"auth\"] }\n"
		.parse()
		.unwrap();
	assert_eq!(
		add_dependency_features(&mut document, "leptoskit", &CLIENT_FEATURES).unwrap(),
		["leptos", "query"]
	);
	assert_eq!(
		add_dependency_features(&mut document, "leptoskit", &CLIENT_FEATURES).unwrap(),
		Vec::<String>::new()
	);

	let mut document: toml_edit::DocumentMut = "[dependencies]\nleptoskit = { version = \"0.1\", \
	                                            default-features = false }\n"
		.parse()
		.unwrap();
	add_dependency_features(&mut document, "leptoskit", &CLIENT_FEATURES).unwrap();
	assert_eq!(
		document.to_string(),
		"[dependencies]\nleptoskit = { version = \"0.1\", default-features = false, features = \
		 [\"auth\", \"leptos\", \"query\"] }\n"
	);

	let mut document: toml_edit::DocumentMut = "[dependencies]\nserde = \"1\"\n".parse().unwrap();
	assert!(add_dependency_features(&mut document, "leptoskit", &CLIENT_FEATURES).is_none());

	let mut document: toml_edit::DocumentMut = "[package]\nname = \"x\"\n".parse().unwrap();
	assert!(add_dependency_features(&mut document, "leptoskit", &CLIENT_FEATURES).is_none());
}

#[test]
fn paths_print_from_the_root() {
	assert_eq!(
		relative(Path::new("/project"), Path::new("/project/app/src/lib.rs")),
		"app/src/lib.rs"
	);
	assert_eq!(
		relative(Path::new("/project"), Path::new("/elsewhere/lib.rs")),
		"/elsewhere/lib.rs"
	);
}

/// Adds the hosts a scaffold has: iOS with its bridge under a target table
/// from a checkout, Android with a plain version, and Linux, which can't
/// host plugins.
fn hosts(root: &Path) {
	for (platform, manifest) in [
		(
			"ios",
			"[package]\nname = \"app_ios\"\n\n[target.'cfg(target_os = \
			 \"ios\")'.dependencies]\nleptoskit_bridge_ffi = { version = \"=0.1.0\", \
			 default-features = false, path = \"/sdk/crates/leptoskit_bridge_ffi\", features = \
			 [\"app\"] }\n",
		),
		(
			"android",
			"[package]\nname = \"app_android\"\n\n[dependencies]\nleptoskit_bridge_ffi = \"0.1\"\n",
		),
		(
			"linux",
			"[package]\nname = \"app_linux\"\n\n[dependencies]\nleptoskit = \"0.1\"\n",
		),
	] {
		fs::create_dir_all(root.join(platform)).unwrap();
		fs::write(root.join(platform).join("Cargo.toml"), manifest).unwrap();
	}
}

#[test]
fn plugin_hosts_get_the_sign_in_plugins_and_the_config_grants_them() {
	let root = workspace(false);
	hosts(root.path());
	let report = run(root.path(), false);

	assert!(
		report.changed.contains(
			&"updated ios/Cargo.toml: added leptoskit_plugin_http, leptoskit_plugin_secure_store, \
			  leptoskit_plugin_passkeys, and leptoskit_plugin_web_auth"
				.to_owned()
		),
		"{:?}",
		report.changed
	);
	assert!(
		report.changed.contains(
			&"updated leptoskit.toml: granted http, granted secure-store, granted passkeys, \
			  granted web-auth, let the HTTP plugin reach the development server"
				.to_owned()
		),
		"{:?}",
		report.changed
	);
	assert!(
		report
			.next
			.iter()
			.any(|step| step.contains("plugin-options.http")),
		"{:?}",
		report.next
	);

	let ios = read(root.path(), "ios/Cargo.toml");
	assert!(
		ios.contains(
			"leptoskit_plugin_http = { version = \"=0.1.0\", default-features = false, path = \
			 \"/sdk/crates/leptoskit_plugin_http\" }"
		),
		"{ios}"
	);
	let android = read(root.path(), "android/Cargo.toml");
	assert!(
		android.contains(
			"leptoskit_plugin_secure_store = { version = \"0.1\", default-features = false }"
		),
		"{android}"
	);
	assert_eq!(
		read(root.path(), "linux/Cargo.toml"),
		"[package]\nname = \"app_linux\"\n\n[dependencies]\nleptoskit = \"0.1\"\n"
	);

	assert!(
		android.contains(
			"leptoskit_plugin_web_auth = { version = \"0.1\", default-features = false }"
		),
		"{android}"
	);

	let config = LeptoskitConfig::load_from(root.path()).unwrap();
	assert_eq!(
		config.native.plugins,
		["http", "secure-store", "passkeys", "web-auth"]
	);
	assert_eq!(
		config.native.plugin_options["http"]["origins"],
		toml::Value::Array(vec![
			"http://localhost:3000".into(),
			"http://10.0.2.2:3000".into()
		])
	);

	// A second run finds everything in place.
	let again = run(root.path(), false);
	assert!(again.changed.is_empty(), "{:?}", again.changed);
}

#[test]
fn plugin_paths_keep_the_separators_the_manifest_uses() {
	assert_eq!(
		sibling("/sdk/crates/leptoskit_bridge_ffi", "p"),
		"/sdk/crates/p"
	);
	assert_eq!(
		sibling(r"C:\sdk\crates\leptoskit_bridge_ffi", "p"),
		r"C:\sdk\crates\p"
	);
	assert_eq!(sibling("leptoskit_bridge_ffi", "p"), "p");
}

#[test]
fn the_apps_own_plugin_choices_are_kept() {
	let root = workspace(false);
	hosts(root.path());
	let config = read(root.path(), "leptoskit.toml");
	fs::write(
		root.path().join("leptoskit.toml"),
		format!(
			"{config}\n[native]\nplugins = [\"http\"]\n\n[native.plugin-options.http]\norigins = \
			 [\"https://notes.example\"]\n"
		),
	)
	.unwrap();
	let report = run(root.path(), false);

	assert!(
		report.changed.contains(
			&"updated leptoskit.toml: granted secure-store, granted passkeys, granted web-auth"
				.to_owned()
		),
		"{:?}",
		report.changed
	);
	let config = LeptoskitConfig::load_from(root.path()).unwrap();
	assert_eq!(
		config.native.plugins,
		["http", "secure-store", "passkeys", "web-auth"]
	);
	assert_eq!(
		config.native.plugin_options["http"]["origins"],
		toml::Value::Array(vec!["https://notes.example".into()])
	);
}

#[test]
fn a_host_without_the_bridge_gets_a_next_step() {
	let root = workspace(false);
	fs::create_dir_all(root.path().join("macos")).unwrap();
	fs::write(
		root.path().join("macos/Cargo.toml"),
		"[package]\nname = \"app_macos\"\n",
	)
	.unwrap();
	let report = run(root.path(), false);

	assert!(
		report.next.iter().any(|step| {
			step.starts_with("macos/Cargo.toml doesn't depend on leptoskit_bridge_ffi")
		}),
		"{:?}",
		report.next
	);
}

/// Every class the screens use is one the native renderer knows: an unknown
/// utility only fails once a device renders it.
#[test]
fn the_screens_use_only_supported_classes() {
	let mut checked = 0;

	let templates = PAGES
		.iter()
		.map(|(file, source, _)| (*file, *source))
		.chain(ROUTES.iter().flat_map(|(file, base, idp)| {
			[*base, *idp]
				.into_iter()
				.flatten()
				.map(move |source| (*file, source))
		}));

	for (file, source) in templates {
		for literal in source.split(".class(\"").skip(1) {
			let classes = literal.split('"').next().unwrap();

			for class in classes.split_whitespace() {
				leptoskit_native::Element::new(leptoskit_native::NativeKind::View)
					.try_class(class)
					.unwrap_or_else(|error| panic!("{file}: {error}"));
				checked += 1;
			}
		}
	}

	assert!(checked > 100, "only {checked} classes found");
}

fn run_idp(root: &Path, force: bool) -> Report {
	let config = LeptoskitConfig::load_from(root).unwrap();
	generate(root, &config, true, force).unwrap()
}

/// A [`workspace`] whose client uses the file router.
fn with_router(db: bool) -> tempfile::TempDir {
	let root = workspace(db);
	let path = root.path();
	let mut config = read(path, "leptoskit.toml");
	config.push_str(
		"\n[[plugins]]\nname = \"router\"\n\n[plugins.options]\ndir = \"app_app/src/routes\"\n",
	);
	fs::write(path.join("leptoskit.toml"), config).unwrap();
	fs::create_dir_all(path.join("app_app/src/routes")).unwrap();
	fs::write(
		path.join("app_app/src/routes/index.rs"),
		"use leptoskit::prelude::*;\n\npub fn page() -> Element {\n\tView()\n}\n",
	)
	.unwrap();
	root
}

#[test]
fn templates_keep_their_idp_lines_only_for_idp() {
	let template = "a\n// leptoskit: idp\nb\n\t// leptoskit: end idp\nc\n";

	assert_eq!(render(template, false), "a\nc\n");
	assert_eq!(render(template, true), "a\nb\nc\n");

	for (file, contents) in pages(false).into_iter().chain(routes(false)) {
		assert!(!contents.contains("leptoskit: "), "{file}");
		assert!(!contents.contains("idp"), "{file} mentions the idp");
	}

	for (file, contents) in pages(true).into_iter().chain(routes(true)) {
		assert!(!contents.contains("// leptoskit: "), "{file}");
	}

	assert!(!server_module("x", false).contains("idp"));
	assert!(server_module("x", true).contains("pub fn provider("));
	assert_eq!(pages(false).len(), 4);
	assert_eq!(pages(true).len(), 8);
	assert_eq!(routes(false).len(), 3);
	assert_eq!(routes(true).len(), 7);
}

#[test]
fn idp_adds_its_screens_routes_config_and_features() {
	let root = with_router(true);
	let path = root.path();
	let report = run_idp(path, false);

	for file in [
		"app_app/src/auth/consent.rs",
		"app_app/src/auth/device_code.rs",
		"app_app/src/auth/connected_apps.rs",
		"app_app/src/auth/developer_apps.rs",
		"app_app/src/routes/consent.rs",
		"app_app/src/routes/device.rs",
		"app_app/src/routes/connected-apps.rs",
		"app_app/src/routes/developer-apps.rs",
	] {
		assert!(
			report.changed.contains(&format!("created {file}")),
			"{file}: {:?}",
			report.changed
		);
	}

	assert_eq!(read(path, "app_app/src/auth/mod.rs"), pages(true)[0].1);
	assert_eq!(
		read(path, "app_app/src/routes/sign-in.rs"),
		routes(true)[0].1
	);
	assert_eq!(
		read(path, "src/sign_in.rs"),
		server_module("../leptoskit.toml", true)
	);
	assert!(
		read(path, "app_app/Cargo.toml")
			.contains("features = [\"auth\", \"leptos\", \"query\", \"idp\"]")
	);
	assert!(read(path, "Cargo.toml").contains("features = [\"server\", \"auth\", \"idp\"]"));
	let module = read(path, "app_app/src/routes.rs");
	assert!(module.contains("Consent"), "{module}");
	assert!(module.contains("ConnectedApps"), "{module}");

	let config = LeptoskitConfig::load_from(path).unwrap();
	let idp = config.auth.unwrap().idp.unwrap();
	assert_eq!(idp.consent_page, "/consent");
	assert_eq!(idp.device_page, "/device");
	assert!(
		report
			.changed
			.contains(&"updated leptoskit.toml: added [auth.idp]".to_owned())
	);

	for marker in ["navigate_with(", "sign_in::provider(&db, &auth)"] {
		assert!(
			report.next.iter().any(|step| step.contains(marker)),
			"{marker}: {:?}",
			report.next
		);
	}

	// Once the app does both, the steps stop asking.
	fs::write(
		path.join("app_app/src/lib.rs"),
		format!(
			"{}\nfn start() {{ auth::provide(); auth::screens().navigate_with(|_| ()); }}\n",
			read(path, "app_app/src/lib.rs")
		),
	)
	.unwrap();
	fs::write(
		path.join("src/main.rs"),
		"mod sign_in;\nfn main() { let _ = (sign_in::service::<()>, sign_in::provider); }\n",
	)
	.unwrap();
	let again = run_idp(path, false);
	assert!(again.changed.is_empty(), "{:?}", again.changed);
	assert!(again.next.is_empty(), "{:?}", again.next);
}

#[test]
fn idp_brings_untouched_files_up_to_date_and_a_plain_run_keeps_them() {
	let root = with_router(true);
	let path = root.path();
	run(path, false);
	let edited = path.join("app_app/src/auth/devices.rs");
	fs::write(&edited, "// mine\n").unwrap();

	let report = run_idp(path, false);
	for file in [
		"app_app/src/auth/mod.rs",
		"app_app/src/routes/sign-in.rs",
		"src/sign_in.rs",
	] {
		assert!(
			report
				.changed
				.contains(&format!("updated {file}: added what --idp needs")),
			"{file}: {:?}",
			report.changed
		);
	}
	assert_eq!(read(path, "app_app/src/auth/mod.rs"), pages(true)[0].1);
	assert_eq!(fs::read_to_string(&edited).unwrap(), "// mine\n");
	assert_eq!(
		report.kept,
		["kept app_app/src/auth/devices.rs: it differs from the template; --force replaces it"]
	);

	// Without --idp, the --idp versions stay: a run never takes screens away.
	let plain = run(path, false);
	assert!(plain.changed.is_empty(), "{:?}", plain.changed);
	assert_eq!(read(path, "app_app/src/auth/mod.rs"), pages(true)[0].1);
	assert_eq!(
		read(path, "app_app/src/routes/sign-in.rs"),
		routes(true)[0].1
	);

	// An --idp file someone edited is theirs, either way.
	let module = path.join("app_app/src/auth/mod.rs");
	fs::write(&module, "// mine too\n").unwrap();
	let report = run_idp(path, false);
	assert!(
		report
			.kept
			.iter()
			.any(|line| line.starts_with("kept app_app/src/auth/mod.rs")),
		"{:?}",
		report.kept
	);
	run_idp(path, true);
	assert_eq!(read(path, "app_app/src/auth/mod.rs"), pages(true)[0].1);
}

#[test]
fn an_auth_table_without_the_idp_gains_one() {
	let root = workspace(false);
	let path = root.path();
	run(path, false);
	assert!(
		LeptoskitConfig::load_from(path)
			.unwrap()
			.auth
			.unwrap()
			.idp
			.is_none()
	);

	let report = run_idp(path, false);
	assert!(
		report
			.changed
			.contains(&"updated leptoskit.toml: added [auth.idp]".to_owned())
	);
	assert!(
		LeptoskitConfig::load_from(path)
			.unwrap()
			.auth
			.unwrap()
			.idp
			.is_some()
	);
	// No server here, so there's no provider step.
	assert!(
		!report
			.next
			.iter()
			.any(|step| step.contains("sign_in::provider"))
	);
}
