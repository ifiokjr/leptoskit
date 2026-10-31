//! Generates a workspace with one application crate and app-owned platform
//! runners.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use camino::Utf8Path;
use camino::Utf8PathBuf;
use leptoskit_config::AppConfig;
use leptoskit_config::AuthConfig;
use leptoskit_config::BuildConfig;
use leptoskit_config::DbConfig;
use leptoskit_config::DbDialect;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::NativeConfig;
use leptoskit_config::NativeTarget;
use leptoskit_config::Platform;
use leptoskit_config::RenderMode;

const NATIVE_PLATFORMS: [&str; 6] = ["ios", "android", "macos", "windows", "linux", "tui"];

/// Writes a new application workspace, depending on the published SDK.
///
/// # Errors
///
/// Returns an error for an invalid package name or an existing template file.
pub fn run(path: &str, platform: Platform, name: Option<&str>) -> Result<()> {
	run_with_sdk(path, platform, name, None)
}

/// Writes a new workspace, optionally depending on a local SDK checkout.
///
/// Standard workspaces enable all six platforms, with `platform` choosing
/// the default build destination. HarmonyOS uses its experimental standalone
/// starter. Existing files are checked before writing any part of the scaffold.
///
/// # Errors
///
/// Returns an error for an invalid package name, SDK checkout, or conflicting
/// file.
pub fn run_with_sdk(
	path: &str,
	platform: Platform,
	name: Option<&str>,
	sdk: Option<&str>,
) -> Result<()> {
	run_with_database(path, platform, name, sdk, None, false)
}

/// Writes a new workspace like [`run_with_sdk`], with a database when
/// `database` names a dialect: a `<name>_db` crate declaring the tables, an
/// empty `migrations/` directory, and a server that migrates at startup.
/// `extension` adds a browser extension: the `extension` platform, a popup
/// and background worker in `[extension]`, and the app's entry points.
///
/// # Errors
///
/// Returns an error for an invalid package name, SDK checkout, or conflicting
/// file, or a database on the standalone HarmonyOS starter.
pub fn run_with_database(
	path: &str,
	platform: Platform,
	name: Option<&str>,
	sdk: Option<&str>,
	database: Option<DbDialect>,
	extension: bool,
) -> Result<()> {
	run_with_options(path, platform, name, sdk, database, extension, false)
}

/// Writes a new workspace like [`run_with_database`], with sign-in wired up
/// when `auth` is set: the `[auth]` table, a user table created on sign-up,
/// the server's routes and layer, and the generated sign-in screens in front
/// of the app. Sign-in keeps its accounts in the database, so `auth` without
/// `database` picks SQLite.
///
/// # Errors
///
/// As [`run_with_database`].
pub fn run_with_options(
	path: &str,
	platform: Platform,
	name: Option<&str>,
	sdk: Option<&str>,
	database: Option<DbDialect>,
	extension: bool,
	auth: bool,
) -> Result<()> {
	if platform == Platform::Harmonyos {
		anyhow::ensure!(
			database.is_none() && !auth,
			"the HarmonyOS starter has no server; create a standard workspace for a database or \
			 sign-in"
		);
		anyhow::ensure!(
			!extension,
			"the HarmonyOS starter has no web client; create a standard workspace for an extension"
		);
		return crate::native::scaffold::run_for_target(
			path,
			name,
			sdk,
			Some(NativeTarget::Harmonyos),
		);
	}

	let database = database.or(auth.then_some(DbDialect::Sqlite));
	let sdk = sdk.map(resolve_sdk).transpose()?;
	std::fs::create_dir_all(path).with_context(|| format!("could not create {path}"))?;
	let dir = std::fs::canonicalize(path).with_context(|| format!("could not resolve {path}"))?;
	let dir = Utf8PathBuf::from_path_buf(dir)
		.map_err(|dir| anyhow::anyhow!("{} is not UTF-8", dir.display()))?;
	let name = name
		.or_else(|| dir.file_name())
		.context("the project directory has no name")?;
	ensure_package_name(name)?;
	let platform = concrete_platform(platform);
	// A requested destination beyond the defaults a new app enables, such as
	// the terminal, joins the enabled set; a default for a disabled platform
	// only fails the first build.
	let mut app = AppConfig {
		name: name.to_owned(),
		default_platform: platform,
		render: RenderMode::Ssr,
		..AppConfig::default()
	};
	if !app.platforms.contains(&platform) {
		app.platforms.push(platform);
	}
	if extension {
		app.platforms.push(Platform::Extension);
	}
	let config = LeptoskitConfig {
		app,
		build: BuildConfig {
			client_package: Some(format!("{name}_app")),
			..BuildConfig::default()
		},
		native: NativeConfig {
			package_id: leptoskit_config::default_package_id(name),
			..if auth {
				auth_native_config()
			} else {
				NativeConfig::default()
			}
		},
		db: database.map(|dialect| {
			DbConfig {
				dialect,
				..DbConfig::default()
			}
		}),
		extension: extension.then(|| extension_config(name)),
		auth: auth.then(AuthConfig::default),
		..LeptoskitConfig::default()
	};
	let files = template(&config, sdk.as_deref())?;

	for (relative, _) in &files {
		let path = dir.join(relative);
		preflight_file(&dir, &path)?;
	}

	for (relative, contents) in &files {
		write_new_bytes(&dir.join(relative), contents)?;
	}

	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		std::fs::set_permissions(
			dir.join("android/gradlew"),
			std::fs::Permissions::from_mode(0o755),
		)?;
	}

	let name = crate::cli::invoked_name();
	crate::ui::success(format_args!(
		"created {dir} (all platforms; default: {platform})"
	));
	crate::ui::hint("next", format_args!("cd {dir} && {name} build"));
	if database.is_some() {
		crate::ui::hint("then", format_args!("{name} db migration create init"));
	}
	if extension {
		println!(
			"build the extension with: {} build --platform extension",
			crate::cli::invoked_name()
		);
	}
	println!(
		"working with a coding agent? give it leptoskit's skill: {} skill install",
		crate::cli::invoked_name()
	);

	if auth {
		println!(
			"before starting the server, set AUTH_ENCRYPTION_KEYS: export \
			 AUTH_ENCRYPTION_KEYS=\"$(date +%Y-%m):$(openssl rand 32 | basenc --base64url | tr -d \
			 =)\""
		);
	}

	Ok(())
}

/// The runtime plugins the sign-in client needs in a native app: `http` to
/// reach the server, and `secure-store` for the refresh token.
pub(crate) const AUTH_PLUGINS: [(&str, &str); 2] = [
	("http", "leptoskit_plugin_http"),
	("secure-store", "leptoskit_plugin_secure_store"),
];

/// The origins a new app's HTTP plugin may reach: the development server,
/// as a simulator or desktop sees it and as the Android emulator does.
pub(crate) const AUTH_DEV_ORIGINS: [&str; 2] = ["http://localhost:3000", "http://10.0.2.2:3000"];

/// The platforms whose hosts run runtime plugins, and so depend on the
/// sign-in client's.
pub(crate) const PLUGIN_PLATFORMS: [&str; 3] = ["ios", "android", "macos"];

/// `[native]` for a new app with sign-in: the plugins granted, and the
/// development server as the HTTP plugin's only origins.
fn auth_native_config() -> NativeConfig {
	let mut http = toml::Table::new();
	http.insert(
		"origins".into(),
		toml::Value::Array(AUTH_DEV_ORIGINS.map(toml::Value::from).to_vec()),
	);
	NativeConfig {
		plugins: AUTH_PLUGINS.map(|(name, _)| name.to_owned()).to_vec(),
		plugin_options: [("http".to_owned(), http)].into(),
		..NativeConfig::default()
	}
}

/// Rejects existing files and redirected parents before writing the workspace.
fn preflight_file(root: &Utf8Path, path: &Utf8Path) -> Result<()> {
	for entry in path.ancestors().take_while(|entry| *entry != root) {
		let metadata = match std::fs::symlink_metadata(entry) {
			Ok(metadata) => metadata,
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
			Err(error) => return Err(error).with_context(|| format!("could not inspect {entry}")),
		};
		anyhow::ensure!(entry != path, "{path} already exists");
		anyhow::ensure!(
			metadata.is_dir() && !metadata.file_type().is_symlink(),
			"{entry} must be an ordinary directory"
		);
	}

	Ok(())
}

/// Converts legacy build selectors into a concrete application destination.
fn concrete_platform(platform: Platform) -> Platform {
	if platform != Platform::Native {
		return platform;
	}

	match std::env::consts::OS {
		"macos" => Platform::Macos,
		"windows" => Platform::Windows,
		_ => Platform::Linux,
	}
}

/// Creates a text file without replacing an existing destination.
pub(crate) fn write_new(path: &Utf8Path, contents: &str) -> Result<()> {
	write_new_bytes(path, contents.as_bytes())
}

/// Creates a template file, including bundled binary assets.
fn write_new_bytes(path: &Utf8Path, contents: &[u8]) -> Result<()> {
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent).with_context(|| format!("could not create {parent}"))?;
	}

	let mut file = std::fs::OpenOptions::new()
		.write(true)
		.create_new(true)
		.open(path)
		.with_context(|| format!("could not create {path}"))?;
	file.write_all(contents)
		.with_context(|| format!("could not write {path}"))
}

/// Rejects names Cargo would refuse as a package name.
pub(crate) fn ensure_package_name(name: &str) -> Result<()> {
	let mut characters = name.chars();
	let valid = characters
		.next()
		.is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
		&& characters.all(|character| {
			character.is_ascii_alphanumeric() || character == '_' || character == '-'
		});
	anyhow::ensure!(
		valid,
		"`{name}` is not a valid cargo package name: use ASCII letters, digits, `_`, and `-`, \
		 starting with a letter or `_` (or pass --name)"
	);
	Ok(())
}

/// Resolves the SDK once; each dependency below also checks its crate manifest.
pub(crate) fn resolve_sdk(sdk: &str) -> Result<Utf8PathBuf> {
	let root = std::fs::canonicalize(sdk).context("resolve --sdk-path")?;
	let root = Utf8PathBuf::from_path_buf(root)
		.map_err(|root| anyhow::anyhow!("{} is not UTF-8", root.display()))?;
	anyhow::ensure!(
		root.join("crates/leptoskit/Cargo.toml").is_file(),
		"--sdk-path {root} has no crates/leptoskit"
	);
	Ok(root)
}

/// A pinned SDK dependency with an optional local path and feature list.
pub(crate) fn sdk_dependency(
	package: &str,
	sdk: Option<&Utf8Path>,
	features: &[&str],
) -> Result<String> {
	let version = env!("CARGO_PKG_VERSION");
	let mut dependency = format!("{{ version = \"={version}\", default-features = false");

	if let Some(sdk) = sdk {
		let path = sdk.join("crates").join(package);
		anyhow::ensure!(
			path.join("Cargo.toml").is_file(),
			"--sdk-path lacks {package}"
		);
		write!(
			dependency,
			", path = {}",
			toml::Value::from(manifest_path(path.as_str()))
		)?;
	}

	if !features.is_empty() {
		write!(dependency, ", features = {features:?}")?;
	}

	dependency.push_str(" }");
	Ok(dependency)
}

/// A path as a manifest spells it: with `/` between components, which Cargo
/// reads on every platform, so a manifest written on Windows doesn't mix
/// separators or carry backslashes.
pub(crate) fn manifest_path(path: &str) -> String {
	path.replace('\\', "/")
}

/// Assembles every template before creating any destination files.
fn template(config: &LeptoskitConfig, sdk: Option<&Utf8Path>) -> Result<Vec<(String, Vec<u8>)>> {
	let name = &config.app.name;
	let app = format!("{name}_app");
	let backend = format!("{name}_backend");
	let shared = format!("{name}_shared");
	let crate_name = name.replace('-', "_");
	let expand = |source: &str| {
		source
			.replace("__NAME__", name)
			.replace("__CRATE__", &crate_name)
	};
	let mut files = vec![
		(
			leptoskit_config::CONFIG_FILE.to_owned(),
			config.to_toml_string().context("serialize config")?,
		),
		(
			".gitignore".into(),
			"/target\n/dist\n/.leptoskit\n/android/.gradle\n/android/**/build\n/android/local.\
			 properties\n"
				.into(),
		),
		(
			"Cargo.toml".into(),
			server_manifest(name, sdk, config.db.as_ref(), config.auth.is_some())?,
		),
		(
			"src/main.rs".into(),
			expand(include_str!("new_project/templates/server.rs")),
		),
		(
			format!("{app}/Cargo.toml"),
			app_manifest(name, sdk, config.auth.is_some())?,
		),
		(
			format!("{app}/build.rs"),
			"fn main() -> Result<(), leptoskit_build::Error> \
			 {\n\tleptoskit_build::platforms()\n}\n"
				.into(),
		),
		(
			format!("{app}/src/lib.rs"),
			expand(include_str!("new_project/templates/app.rs")),
		),
		(
			format!("{app}/src/routes.rs"),
			"//! App routes with automatic platform overrides.\n\nleptoskit::platform_module!(pub \
			 mod home = \"routes/home\");\n"
				.into(),
		),
		(
			format!("{app}/src/routes/home.rs"),
			include_str!("new_project/templates/home.rs").into(),
		),
		(
			format!("{app}/src/components/mod.rs"),
			"//! Reusable application components shared by all platforms.\n".into(),
		),
		(
			format!("{app}/src/services/mod.rs"),
			include_str!("new_project/templates/services.rs").into(),
		),
		(
			format!("{backend}/Cargo.toml"),
			format!(
				"[package]\nname = \"{backend}\"\nversion = \"0.1.0\"\nedition = \
				 \"2024\"\npublish = false\n\n[dependencies]\n{shared} = {{ path = \
				 \"../{shared}\" }}\nleptoskit = {server_dependency}\n",
				server_dependency = sdk_dependency("leptoskit", sdk, &["server"])?,
			),
		),
		(
			format!("{backend}/src/lib.rs"),
			include_str!("new_project/templates/backend.rs").into(),
		),
		(
			format!("{backend}/src/rpc/mod.rs"),
			expand(include_str!("new_project/templates/rpc.rs")),
		),
		(
			format!("{shared}/Cargo.toml"),
			format!(
				"[package]\nname = \"{shared}\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish \
				 = false\n\n[dependencies]\nserde = {{ version = \"1\", features = [\"derive\"] \
				 }}\n"
			),
		),
		(
			format!("{shared}/src/lib.rs"),
			include_str!("new_project/templates/shared.rs").into(),
		),
		("web/index.html".into(), index_html(name)),
		(
			"assets/app.css".into(),
			include_str!("new_project/templates/app.css").into(),
		),
		("README.md".into(), readme(name)),
	];
	if let Some(db) = &config.db {
		database_files(&mut files, name, db, sdk, config.auth.is_some())?;

		if config.auth.is_some() {
			auth_files(&mut files, name);
		}
	}
	if config.extension.is_some() {
		extension_files(&mut files, name)?;
	}

	files.extend(platform_files(config, sdk)?);
	let mut files: Vec<_> = files
		.into_iter()
		.map(|(path, source)| (path, source.into_bytes()))
		.collect();
	files.push((
		"android/gradle/wrapper/gradle-wrapper.jar".into(),
		include_bytes!("native/templates/gradle/gradle-wrapper.jar").to_vec(),
	));
	Ok(files)
}

/// A new app's extension: a popup at the home screen, a background worker
/// that answers messages, and the Chromium and Firefox packages.
fn extension_config(name: &str) -> leptoskit_config::ExtensionConfig {
	use leptoskit_config::extension::ExtensionBackground;
	use leptoskit_config::extension::ExtensionPage;
	use leptoskit_config::extension::FirefoxSettings;

	leptoskit_config::ExtensionConfig {
		name: Some(name.to_owned()),
		popup: Some(ExtensionPage {
			route: "/".to_owned(),
			browsers: None,
		}),
		background: Some(ExtensionBackground {
			events: vec!["runtime.message".to_owned()],
		}),
		firefox: Some(FirefoxSettings {
			id: format!("{name}@example.com"),
			strict_min_version: None,
			data_collection: vec!["none".to_owned()],
		}),
		..leptoskit_config::ExtensionConfig::default()
	}
}

/// The app crate's extension: its feature, its entry points, and the start
/// function the website keeps to itself.
fn extension_files(files: &mut Vec<(String, String)>, name: &str) -> Result<()> {
	let app = format!("{name}_app");
	for (path, source) in files.iter_mut() {
		if *path == format!("{app}/Cargo.toml") {
			*source = source.replacen(
				"\n[dependencies]",
				"# The browser extension: `leptoskit build --platform extension`.\nextension = \
				 [\"csr\", \"leptoskit/extension\"]\n\n[dependencies]",
				1,
			);
		} else if *path == format!("{app}/src/lib.rs") {
			*source = source
				// Without the newline, so a Windows checkout's CRLF template
				// matches too.
				.replacen(
					"pub mod services;",
					"pub mod services;\n#[cfg(feature = \"extension\")]\nmod extension;",
					1,
				)
				.replacen(
					"#[cfg(all(target_arch = \"wasm32\", feature = \"web\", not(feature = \
					 \"hydrate\")))]",
					"#[cfg(all(\n\ttarget_arch = \"wasm32\",\n\tfeature = \"web\",\n\tnot(feature \
					 = \"hydrate\"),\n\tnot(feature = \"extension\")\n))]",
					1,
				);
			source.push_str(
				"\n// The browser extension's popup and background worker; see \
				 `extension.rs`.\n#[cfg(feature = \
				 \"extension\")]\nleptoskit::extension::entry!(main: extension::main, background: \
				 extension::background);\n",
			);
		} else if path == "README.md" {
			let _ = write!(
				source,
				"\n## The browser extension\n\n`{app}/src/extension.rs` opens the home screen in \
				 the extension's popup, and answers messages in its background worker. \
				 `[extension]` in `leptoskit.toml` lists its surfaces.\n\n```bash\nleptoskit \
				 build --platform extension   # .leptoskit/extension/chromium and \
				 firefox\nleptoskit dev --platform extension     # in Chrome for Testing, \
				 reloaded on every change\nleptoskit test extension               # every \
				 surface, in a real browser\n```\n"
			);
		}
	}
	anyhow::ensure!(
		files.iter().any(|(path, source)| {
			*path == format!("{app}/src/lib.rs") && source.contains("entry!")
		}),
		"the app template changed shape; update the extension scaffold"
	);
	files.push((
		format!("{app}/src/extension.rs"),
		include_str!("new_project/templates/extension.rs").into(),
	));
	Ok(())
}

/// The `leptoskit` feature that connects to a dialect's database.
fn database_feature(db: &DbConfig) -> &'static str {
	match db.dialect {
		DbDialect::Postgres => "db-postgres",
		DbDialect::Sqlite => "db-sqlite",
	}
}

/// Replaces the backend and server with versions that use the database, and
/// adds the crate that declares the tables and the empty migrations directory.
/// With `auth`, the server signs people in, the backend reads the database
/// the sign-in layer gives each request, and the tables include the users.
fn database_files(
	files: &mut Vec<(String, String)>,
	name: &str,
	db: &DbConfig,
	sdk: Option<&Utf8Path>,
	auth: bool,
) -> Result<()> {
	let crate_name = name.replace('-', "_");
	let backend_crate = format!("{name}_backend");
	let shared = format!("{name}_shared");
	let database = format!("{name}_db");
	let feature = database_feature(db);
	let url = match db.dialect {
		DbDialect::Postgres => {
			format!(
				"std::env::var(\"DATABASE_URL\")\n\t\t.map_err(|_| \"set DATABASE_URL to the \
				 Postgres database, such as postgres://localhost/{name}\")?"
			)
		}
		DbDialect::Sqlite => {
			format!(
				"std::env::var(\"DATABASE_URL\")\n\t\t.unwrap_or_else(|_| \
				 \"sqlite://{name}.sqlite3\".to_owned())"
			)
		}
	};
	let expand = |source: &str| {
		source
			.replace("__DATABASE_URL__", &url)
			.replace("__NAME__", name)
			.replace("__CRATE__", &crate_name)
	};
	let mut replace = |path: String, contents: String| {
		match files.iter_mut().find(|(existing, _)| *existing == path) {
			Some(entry) => entry.1 = contents,
			None => files.push((path, contents)),
		}
	};

	let (server, backend, tables) = if auth {
		(
			include_str!("new_project/templates/server_auth.rs"),
			include_str!("new_project/templates/backend_auth.rs"),
			include_str!("new_project/templates/db_auth.rs"),
		)
	} else {
		(
			include_str!("new_project/templates/server_db.rs"),
			include_str!("new_project/templates/backend_db.rs"),
			include_str!("new_project/templates/db.rs"),
		)
	};
	replace("src/main.rs".into(), expand(server));
	replace(
		format!("{backend_crate}/Cargo.toml"),
		format!(
			"[package]\nname = \"{backend_crate}\"\nversion = \"0.1.0\"\nedition = \
			 \"2024\"\npublish = false\n\n[dependencies]\n{database} = {{ path = \
			 \"../{database}\", features = [\"server\"] }}\n{shared} = {{ path = \"../{shared}\" \
			 }}\nleptoskit = {leptoskit}\nserde = {{ version = \"1\", features = [\"derive\"] }}\n",
			leptoskit = sdk_dependency("leptoskit", sdk, &["server", feature])?,
		),
	);
	replace(format!("{backend_crate}/src/lib.rs"), backend.into());
	replace(
		format!("{backend_crate}/src/rpc/mod.rs"),
		expand(include_str!("new_project/templates/rpc_db.rs")),
	);
	let dialect = db.dialect.as_str();
	let (auth_dependency, auth_feature) = if auth {
		(
			format!(
				"leptoskit_auth = {}\n",
				sdk_dependency("leptoskit_auth", sdk, &[])?
			),
			format!(", \"leptoskit_auth/{dialect}\""),
		)
	} else {
		(String::new(), String::new())
	};
	replace(
		format!("{database}/Cargo.toml"),
		format!(
			"[package]\nname = \"{database}\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = \
			 false\n\n[dependencies]\n{auth_dependency}leptoskit_db = \
			 {dependency}\n\n[features]\n# The server side of the tables: rows, queries, \
			 migrations, and the driver.\nserver = [\"leptoskit_db/{dialect}\"{auth_feature}]\n",
			dependency = sdk_dependency("leptoskit_db", sdk, &[])?,
		),
	);
	replace(format!("{database}/src/lib.rs"), tables.into());
	replace(
		format!("{}/registry.txt", db.migrations.display()),
		"# The migrations, in the order they apply. `leptoskit db migration create` adds to it.\n"
			.into(),
	);

	if db.dialect == DbDialect::Sqlite {
		let gitignore = files
			.iter_mut()
			.find(|(path, _)| path == ".gitignore")
			.expect("the scaffold writes .gitignore");
		writeln!(gitignore.1, "/{name}.sqlite3*")?;
	}

	let readme = files
		.iter_mut()
		.find(|(path, _)| path == "README.md")
		.expect("the scaffold writes README.md");
	readme.1.push_str(&database_readme(name, db));
	Ok(())
}

/// Puts the sign-in screens in front of the app, and gives the server its
/// sign-in module.
fn auth_files(files: &mut Vec<(String, String)>, name: &str) {
	let app = format!("{name}_app");
	let lib = format!("{app}/src/lib.rs");
	let source = include_str!("new_project/templates/app_auth.rs").replace("__NAME__", name);

	match files.iter_mut().find(|(path, _)| *path == lib) {
		Some(entry) => entry.1 = source,
		None => files.push((lib, source)),
	}

	for (path, contents) in crate::auth_pages::PAGES {
		files.push((format!("{app}/src/{path}"), contents.to_owned()));
	}

	files.push((
		"src/sign_in.rs".into(),
		crate::auth_pages::server_module("../leptoskit.toml"),
	));

	if let Some(readme) = files.iter_mut().find(|(path, _)| path == "README.md") {
		readme.1.push_str(&auth_readme(name));
	}
}

fn auth_readme(name: &str) -> String {
	format!(
		r#"
## Sign-in

The app starts on a sign-in screen: an email address, then a code emailed to it. The same two steps create an account for a new address, and each new account gets a row in `{name}_db`'s `User` table. Once signed in, the bar at the top shows where you're signed in and signs you out.

The server needs its keys before it starts. Keep them in your secret store:

```sh
export AUTH_ENCRYPTION_KEYS="$(date +%Y-%m):$(openssl rand 32 | basenc --base64url | tr -d =)"
```

Before you deploy, set `origin` in `[auth]` to your site's URL, such as `https://myapp.com`, and say how to send email in `[auth.mail]`. In a debug build, emails aren't sent: open `/_leptoskit/mail` to read them, or sign in to any address without a code by posting `{{ "email": "ada@example.com" }}` to `/auth/dev/sign-in`. Release builds have neither, and refuse to start without `[auth.mail]`.

The screens live in `{name}_app/src/auth/` and are yours to restyle. They work the same in the browser and in native apps. A native app finds the server at `SERVER` in `{name}_app/src/lib.rs`. It reaches it through the `http` plugin and keeps its refresh token with `secure-store`; both are granted in `[native]` in `leptoskit.toml`, and the HTTP plugin may reach the development server. Before you ship, set `SERVER` to your site's origin and put that origin in `[native.plugin-options.http] origins`. The Linux, Windows, and terminal runners can't host plugins yet, so there the screens answer that sign-in isn't set up.
"#
	)
}

fn database_readme(name: &str, db: &DbConfig) -> String {
	let url = match db.dialect {
		DbDialect::Postgres => {
			format!(
				"Set `DATABASE_URL` to a Postgres database, such as \
				 `postgres://localhost/{name}`, before starting the server."
			)
		}
		DbDialect::Sqlite => {
			format!(
				"The server keeps its data in `{name}.sqlite3`, in the directory it starts from, \
				 unless `DATABASE_URL` names another database."
			)
		}
	};

	format!(
		r"
## Database

`{name}_db` declares the tables as Rust structs. The server sees every column; clients only ever get the `Public…` types, which leave out `private` and `secret` columns. Create the first migration, then run the server, which applies pending migrations as it starts:

```sh
leptoskit db migration create init
leptoskit dev
```

After changing a table, run `leptoskit db migration create <name>` and commit what it writes under `migrations/`. `leptoskit db check` fails when the tables changed without a migration, so run it in CI. {url}

`{name}_backend` serves `GET /api/notes` and `POST /api/notes` from the example `Note` table.
"
	)
}

/// The root SSR package owns workspace composition and imports the shared app.
///
/// It depends on `leptoskit` alone for the server: `axum`, `tokio`,
/// `leptos`, and `leptos_axum` come re-exported through its `server`
/// feature, at the versions leptoskit was built against. With a database it
/// also depends on the crate that declares the tables, to embed their
/// migrations, and with sign-in it turns on `auth`.
fn server_manifest(
	name: &str,
	sdk: Option<&Utf8Path>,
	db: Option<&DbConfig>,
	auth: bool,
) -> Result<String> {
	let mut features = vec!["server"];

	if let Some(db) = db {
		features.push(database_feature(db));
	}

	if auth {
		features.push("auth");
	}

	let dependency = sdk_dependency("leptoskit", sdk, &features)?;
	let (database_member, database_dependency) = match db {
		Some(_) => {
			(
				format!(", \"{name}_db\""),
				format!("{name}_db = {{ path = \"{name}_db\", features = [\"server\"] }}\n"),
			)
		}
		None => (String::new(), String::new()),
	};
	Ok(format!(
		r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"
publish = false

[workspace]
resolver = "2"
members = ["{name}_app", "{name}_backend", "{name}_shared"{database_member}, "ios", "android", "macos", "windows", "linux", "tui"]
default-members = ["."]

[features]
default = ["ssr"]
ssr = ["{name}_app/web", "{name}_app/ssr", "leptoskit/ssr"]

[dependencies]
{name}_app = {{ path = "{name}_app", default-features = false }}
{name}_backend = {{ path = "{name}_backend" }}
{database_dependency}leptoskit = {dependency}

[profile.release]
lto = "thin"
codegen-units = 1

# Keep host proc-macros loadable on macOS 27 (rust-lang/rust#157750).
[profile.release.build-override]
strip = "none"
"#
	))
}

/// One application library supplies SSR, browser Wasm, and native Wasm exports.
///
/// With sign-in, the app uses the auth client, the query cache, and leptos's
/// signals, and a native build keeps its own executor for their tasks.
fn app_manifest(name: &str, sdk: Option<&Utf8Path>, auth: bool) -> Result<String> {
	let features: &[&str] = if auth {
		&["auth", "leptos", "query"]
	} else {
		&[]
	};
	let dependency = sdk_dependency("leptoskit", sdk, features)?;
	let (native, executor) = if auth {
		(
			"\"leptoskit/guest\", \"dep:any_spawner\", \"dep:futures\"",
			"# The native build's executor for sign-in requests.\nany_spawner = { version = \
			 \"0.3\", optional = true }\nfutures = { version = \"0.3\", default-features = false, \
			 features = [\"executor\"], optional = true }\n",
		)
	} else {
		("\"leptoskit/guest\"", "")
	};
	let build_dependency = sdk_dependency("leptoskit_build", sdk, &[])?;
	let wasm_bindgen = crate::wasm::embedded_wasm_bindgen();
	Ok(format!(
		r#"[package]
name = "{name}_app"
version = "0.1.0"
edition = "2024"
publish = false

[lib]
crate-type = ["rlib", "cdylib"]

[features]
default = []
web = ["leptoskit/web", "dep:leptos", "dep:wasm-bindgen"]
native = [{native}]
ios = ["native", "leptoskit/ios"]
android = ["native", "leptoskit/android"]
macos = ["native", "leptoskit/macos"]
windows = ["native", "leptoskit/windows"]
linux = ["native", "leptoskit/linux"]
tui = ["native", "leptoskit/tui"]
csr = ["web", "leptos/csr"]
hydrate = ["web", "leptos/hydrate"]
ssr = ["web", "leptos/ssr"]

[dependencies]
{name}_shared = {{ path = "../{name}_shared" }}
leptoskit = {dependency}
leptos = {{ version = "0.9.0-rc0", default-features = false, optional = true }}
# The embedded wasm-bindgen CLI accepts exactly its matching library version.
wasm-bindgen = {{ version = "={wasm_bindgen}", optional = true }}
{executor}
[build-dependencies]
leptoskit_build = {build_dependency}
"#
	))
}

/// Platform folders contain app-owned native integration and host manifests.
fn platform_files(
	config: &LeptoskitConfig,
	sdk: Option<&Utf8Path>,
) -> Result<Vec<(String, String)>> {
	let name = &config.app.name;
	let package = &config.native.package_id;
	let expand = |source: &str| {
		source
			.replace("__PACKAGE__", package)
			.replace("__NAME__", name)
			.replace("__VERSION__", "0.1.0")
	};
	let mut files = vec![
		(
			"ios/App.swift".into(),
			include_str!("native/templates/App.swift").into(),
		),
		(
			"ios/Info.plist".into(),
			expand(include_str!("native/templates/Info.plist")),
		),
		(
			"ios/AppUITests.swift".into(),
			include_str!("new_project/templates/AppUITests.swift").into(),
		),
		(
			"macos/App.swift".into(),
			include_str!("native/templates/MacApp.swift").into(),
		),
		(
			"macos/Info.plist".into(),
			expand(include_str!("native/templates/MacInfo.plist")),
		),
		(
			"android/gradlew".into(),
			include_str!("native/templates/gradle/gradlew").into(),
		),
		(
			"android/gradlew.bat".into(),
			include_str!("native/templates/gradle/gradlew.bat").into(),
		),
		(
			"android/gradle/wrapper/gradle-wrapper.properties".into(),
			include_str!("native/templates/gradle/gradle-wrapper.properties").into(),
		),
		(
			"android/gradle/LICENSE".into(),
			include_str!("native/templates/gradle/LICENSE").into(),
		),
		(
			"android/settings.gradle.kts".into(),
			"pluginManagement { repositories { google(); mavenCentral(); gradlePluginPortal() } \
			 }\ndependencyResolutionManagement { \
			 repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS); repositories { \
			 google(); mavenCentral() } }\nrootProject.name = \
			 \"LeptosKitApp\"\ninclude(\":app\")\n"
				.into(),
		),
		(
			"android/build.gradle.kts".into(),
			"plugins { id(\"com.android.application\") version \"9.0.1\" apply false }\n".into(),
		),
		(
			"android/gradle.properties".into(),
			"org.gradle.jvmargs=-Xmx2g\norg.gradle.workers.max=2\nandroid.useAndroidX=true\n"
				.into(),
		),
		(
			"android/app/build.gradle.kts".into(),
			expand(include_str!("native/templates/build.gradle.kts"))
				.replace("../../../.leptoskit", "../../.leptoskit")
				.replace(".leptoskit/mobile", ".leptoskit/android/mobile"),
		),
		(
			// App-owned; the rules the JNI library needs are regenerated in
			// .leptoskit/android so they track the bridge version.
			"android/app/proguard-rules.pro".into(),
			"# R8 rules for this app's own code. leptoskit writes the rules its native library \
			 needs\n# to .leptoskit/android/proguard-rules.pro on every build.\n"
				.into(),
		),
		(
			"android/app/src/main/AndroidManifest.xml".into(),
			expand(include_str!("native/templates/AndroidManifest.xml")),
		),
		(
			"android/app/src/main/res/values/styles.xml".into(),
			include_str!("native/templates/styles.xml").into(),
		),
		(
			"android/app/src/main/java/MainActivity.kt".into(),
			include_str!("native/templates/MainActivity.kt")
				.replace("dev.leptoskit.generated", package),
		),
		(
			"android/app/src/androidTest/java/NativeAppTest.kt".into(),
			include_str!("new_project/templates/NativeAppTest.kt")
				.replace("dev.leptoskit.generated", package),
		),
		(
			"windows/src/main.rs".into(),
			expand(include_str!("native/templates/WindowsMain.rs")),
		),
		(
			"linux/src/main.rs".into(),
			expand(include_str!("native/templates/LinuxMain.rs")),
		),
		(
			"windows/src/runtime.rs".into(),
			include_str!("native/templates/DesktopRuntime.rs").into(),
		),
		(
			"linux/src/runtime.rs".into(),
			include_str!("native/templates/DesktopRuntime.rs").into(),
		),
		(
			"tui/src/main.rs".into(),
			expand(include_str!("native/templates/TuiMain.rs")),
		),
		(
			"tui/src/runtime.rs".into(),
			include_str!("native/templates/DesktopRuntime.rs").into(),
		),
		(
			"windows/build.rs".into(),
			include_str!("native/templates/WindowsBuild.rs").into(),
		),
		(
			"windows/app.manifest".into(),
			include_str!("native/templates/WindowsManifest.xml").into(),
		),
	];

	for platform in NATIVE_PLATFORMS {
		files.push((
			format!("{platform}/Cargo.toml"),
			host_manifest(name, platform, sdk, config.auth.is_some())?,
		));
		// The terminal runs on every desktop; its gate is the architecture.
		let gate = if platform == "tui" {
			"#[cfg(not(target_arch = \"wasm32\"))]"
		} else {
			&format!("#[cfg(target_os = \"{platform}\")]")
		};
		files.push((
			format!("{platform}/src/lib.rs"),
			format!(
				"//! Platform host registration generated from this runner's \
				 dependencies.\n{gate}\ninclude!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \
				 \"/../.leptoskit/{platform}/registrant.rs\"));\n"
			),
		));
	}

	Ok(files)
}

/// OS dependencies are isolated in runner crates and never enter the Wasm app.
///
/// With sign-in, the hosts that run runtime plugins depend on the ones the
/// client needs, since only a host's direct dependencies can register them.
fn host_manifest(name: &str, platform: &str, sdk: Option<&Utf8Path>, auth: bool) -> Result<String> {
	let ffi = sdk_dependency("leptoskit_bridge_ffi", sdk, &["app"])?;
	// The terminal runner builds wherever cargo does, so its gate is the
	// architecture; the widget runners stay behind their operating systems.
	let gate = if platform == "tui" {
		"not(target_arch = \"wasm32\")".to_owned()
	} else {
		format!("target_os = \"{platform}\"")
	};
	let mut manifest = format!(
		"[package]\nname = \"{name}_{platform}\"\nversion = \"0.1.0\"\nedition = \
		 \"2024\"\npublish = false\n\n[lib]\nname = \"leptoskit_app\"\ncrate-type = \
		 [\"staticlib\"]\n\n[target.'cfg({gate})'.dependencies]\nleptoskit_bridge_ffi = {ffi}\n"
	);

	if auth && PLUGIN_PLATFORMS.contains(&platform) {
		for (_, package) in AUTH_PLUGINS {
			writeln!(
				manifest,
				"{package} = {}",
				sdk_dependency(package, sdk, &[])?
			)?;
		}
	}

	if matches!(platform, "windows" | "linux" | "tui") {
		let feature = match platform {
			"linux" => "gtk",
			"tui" => "tui",
			_ => "windows",
		};
		let features: &[&str] = if platform == "tui" {
			&["wasm", feature, "codepush-http"]
		} else {
			&["wasm", "rive", feature, "codepush-http"]
		};
		let dependency = sdk_dependency("leptoskit", sdk, features)?;
		writeln!(manifest, "leptoskit = {dependency}")?;
		writeln!(
			manifest,
			"\n[[bin]]\nname = \"{}-{platform}\"\npath = \"src/main.rs\"\n",
			name.replace('_', "-")
		)?;
	}

	Ok(manifest)
}

/// The CSR document also documents the browser bootstrap for custom web hosts.
fn index_html(name: &str) -> String {
	let lib = format!("{}_app", name.replace('-', "_"));
	format!(
		r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <link rel="icon" href="data:," />
    <title>{name}</title>
    <link rel="stylesheet" href="/assets/app.css" />
  </head>
  <body>
    <script type="module">
      import init from "/pkg/{lib}.js";
      await init({{ module_or_path: "/pkg/{lib}_bg.wasm" }});
    </script>
  </body>
</html>
"#
	)
}

/// Explains source ownership, platform overrides, and normal build commands.
fn readme(name: &str) -> String {
	format!(
		r#"# {name}

Write the application once in `{name}_app`. The browser, web SSR server, and native runners render the same screen and counter behavior. `{name}_backend` implements RPC endpoints; `{name}_shared` holds their serializable contracts. The root server imports the app and backend, while application builds never import backend implementation.

```sh
leptoskit build
leptoskit dev
leptoskit build --platform ios
leptoskit build --platform android
leptoskit build --platform macos
leptoskit build --platform windows
leptoskit build --platform linux
```

All six destinations are enabled in `leptoskit.toml`. Each build activates exactly one platform feature in `{name}_app`; add optional dependencies to that feature when they support the app's actual compilation target. Native app code compiles to Wasm, so UIKit, JNI, and other OS dependencies belong in the corresponding runner's Cargo manifest or a native plugin.

`src/main.rs` starts the SSR server, serves browser files, and registers the backend's `GET /api/health` endpoint. Shared assets live in `assets/`. `web/index.html` supplies the document for CSR mode (`app.render = "csr"`); SSR uses the shared app's `shell` function.

## Platform overrides

`{name}_app/src/routes.rs` declares the home module once:

```rust
leptoskit::platform_module!(pub mod home = "routes/home");
```

The module path is relative to the app crate's `src/` directory. Add `routes/home.ios.rs`, `routes/home.android.rs`, `routes/home.web.rs`, `routes/home.macos.rs`, `routes/home.windows.rs`, or `routes/home.linux.rs` to replace `routes/home.rs` on that platform. Imports stay `crate::routes::home`. The shared file is the fallback; replacements must expose the same interface. Use this declaration for any module that needs platform overrides. The build helper watches additions and removals of override files.

`ios/`, `android/`, `macos/`, `windows/`, and `linux/` are app-owned platform runners. Install the matching SDK when building a destination. All generated registrants, SDK integration files, and build intermediates stay under `.leptoskit/<platform>/`, which is ignored by Git. Prepare a native target before opening its generated Xcode project or the app-owned `android/` Gradle project.

The starter requests no native plugin capabilities. Add native plugins to the runner's direct Cargo dependencies and grant them explicitly in `native.plugins`.
"#
	)
}

/// Stable root template files, independent of the chosen application name.
pub fn template_files() -> &'static [&'static str] {
	&[
		leptoskit_config::CONFIG_FILE,
		".gitignore",
		"Cargo.toml",
		"src/main.rs",
		"web/index.html",
		"assets/app.css",
		"README.md",
	]
}

/// Ensures the workspace contains its shared crates and every platform runner.
///
/// # Errors
///
/// Returns an error naming the first missing source file.
pub fn verify_scaffold(dir: &Path) -> Result<()> {
	for file in template_files() {
		anyhow::ensure!(dir.join(file).is_file(), "scaffold is missing {file}");
	}

	let config = LeptoskitConfig::load_from(dir)?;
	let name = config.app.name;

	for file in [
		format!("{name}_app/src/lib.rs"),
		format!("{name}_app/src/routes/home.rs"),
		format!("{name}_backend/src/rpc/mod.rs"),
		format!("{name}_shared/src/lib.rs"),
	] {
		anyhow::ensure!(dir.join(&file).is_file(), "scaffold is missing {file}");
	}

	for platform in NATIVE_PLATFORMS {
		let file = format!("{platform}/Cargo.toml");
		anyhow::ensure!(dir.join(&file).is_file(), "scaffold is missing {file}");
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::host_manifest;
	use super::manifest_path;

	#[test]
	fn the_terminal_host_is_gated_on_the_architecture_alone() {
		let manifest = host_manifest("demo", "tui", None, true).unwrap();
		assert_eq!(manifest.matches("[target.").count(), 1, "{manifest}");
		assert!(manifest.contains("[target.'cfg(not(target_arch = \"wasm32\"))'.dependencies]"));
		assert!(!manifest.contains("target_os = \"tui\""));
		assert!(manifest.contains("name = \"demo-tui\""));
		assert!(manifest.contains("features = [\"wasm\", \"tui\", \"codepush-http\"]"));

		let linux = host_manifest("demo", "linux", None, true).unwrap();
		assert!(linux.contains("[target.'cfg(target_os = \"linux\")'.dependencies]"));
		assert!(linux.contains("features = [\"wasm\", \"rive\", \"gtk\", \"codepush-http\"]"));

		let windows = host_manifest("demo", "windows", None, true).unwrap();
		assert!(
			windows.contains("features = [\"wasm\", \"rive\", \"windows\", \"codepush-http\"]")
		);
		// Desktop runners can't host plugins, so they don't link sign-in's.
		assert!(!windows.contains("leptoskit_plugin_http"), "{windows}");
	}

	#[test]
	fn manifest_paths_use_forward_slashes() {
		assert_eq!(
			manifest_path(r"C:\sdk\crates\leptoskit_plugin_http"),
			"C:/sdk/crates/leptoskit_plugin_http"
		);
		assert_eq!(
			manifest_path("/sdk/crates/leptoskit_plugin_http"),
			"/sdk/crates/leptoskit_plugin_http"
		);
	}

	#[test]
	fn plugin_hosts_depend_on_the_sign_in_plugins_only_with_sign_in() {
		for platform in ["ios", "android", "macos"] {
			let with = host_manifest("demo", platform, None, true).unwrap();
			assert!(
				with.contains("leptoskit_plugin_http = { version = "),
				"{with}"
			);
			assert!(
				with.contains("leptoskit_plugin_secure_store = { version = "),
				"{with}"
			);

			let without = host_manifest("demo", platform, None, false).unwrap();
			assert!(!without.contains("leptoskit_plugin_"), "{without}");
		}
	}
}
