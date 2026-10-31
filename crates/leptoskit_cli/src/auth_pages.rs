//! `leptoskit generate auth`: writes sign-in screens into an existing app,
//! and wires them up as far as can be done without rewriting the app's own
//! code. `leptoskit new --auth` writes the same files.
//!
//! Every write is idempotent. A file that already matches is left alone; a
//! file that differs is the app's to keep unless `--force` replaces it; and
//! edits to existing files only add what's missing.

use std::fmt;
use std::fmt::Write as _;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::LeptoskitConfig;

/// The screens, relative to the client crate's `src/`.
pub(crate) const PAGES: [(&str, &str); 4] = [
	("auth/mod.rs", include_str!("auth_pages/templates/mod.rs")),
	(
		"auth/sign_in.rs",
		include_str!("auth_pages/templates/sign_in.rs"),
	),
	(
		"auth/devices.rs",
		include_str!("auth_pages/templates/devices.rs"),
	),
	(
		"auth/security.rs",
		include_str!("auth_pages/templates/security.rs"),
	),
];

/// The route files for an app that uses the file router, relative to its
/// routes directory.
const ROUTES: [(&str, &str); 3] = [
	(
		"sign-in.rs",
		include_str!("auth_pages/templates/route_sign_in.rs"),
	),
	(
		"devices.rs",
		include_str!("auth_pages/templates/route_devices.rs"),
	),
	(
		"security.rs",
		include_str!("auth_pages/templates/route_security.rs"),
	),
];

/// The `leptoskit` features the screens use.
const CLIENT_FEATURES: [&str; 3] = ["auth", "leptos", "query"];

/// The `[auth]` table added to a `leptoskit.toml` that has none.
const AUTH_TABLE: &str = "
# Sign-in. Secrets stay in the environment: AUTH_ENCRYPTION_KEYS holds the
# server's keys, as `id:key` pairs.
[auth]
keys_env = \"AUTH_ENCRYPTION_KEYS\"
";

/// The server's sign-in module, reading the configuration at `config`,
/// relative to the module.
pub(crate) fn server_module(config: &str) -> String {
	include_str!("auth_pages/templates/server.rs").replace("__CONFIG_PATH__", config)
}

/// Runs `leptoskit generate auth` in the project around the current
/// directory, and prints what it did and what's left to do.
///
/// # Errors
///
/// Returns an error when there's no project, its client crate can't be
/// found, or a file can't be read or written.
pub fn run(force: bool) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	let report = generate(&root, &config, force)?;
	print!("{report}");
	Ok(())
}

/// What one run did, in the order it did it, and what's left for the
/// developer.
#[derive(Debug, Default)]
pub struct Report {
	/// One line per file created, replaced, or edited.
	pub changed: Vec<String>,
	/// One line per file left as it was because it differs.
	pub kept: Vec<String>,
	/// The steps the generator can't take itself.
	pub next: Vec<String>,
}

impl fmt::Display for Report {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		for line in &self.changed {
			writeln!(f, "{line}")?;
		}

		for line in &self.kept {
			writeln!(f, "{line}")?;
		}

		if self.changed.is_empty() {
			writeln!(
				f,
				"nothing changed: the sign-in screens are already in place"
			)?;
		}

		if !self.next.is_empty() {
			writeln!(f, "\nnext:")?;

			for (number, step) in self.next.iter().enumerate() {
				writeln!(f, "  {}. {step}", number + 1)?;
			}
		}

		Ok(())
	}
}

/// Writes the screens into the project at `root`, and reports what changed.
///
/// # Errors
///
/// As [`run`].
pub fn generate(root: &Path, config: &LeptoskitConfig, force: bool) -> Result<Report> {
	let mut report = Report::default();
	let client = client_dir(root, config)?;
	let source = client.join("src");
	let entry = ["lib.rs", "main.rs"]
		.into_iter()
		.map(|file| source.join(file))
		.find(|path| path.is_file())
		.with_context(|| {
			format!(
				"{} has no src/lib.rs or src/main.rs",
				relative(root, &client)
			)
		})?;
	anyhow::ensure!(
		!source.join("auth.rs").exists(),
		"{} already exists; the screens go in src/auth/, so rename it first",
		relative(root, &source.join("auth.rs"))
	);

	for (path, contents) in PAGES {
		write_owned(root, &source.join(path), contents, force, &mut report)?;
	}

	declare_module(root, &entry, "auth", &mut report)?;
	add_features(
		root,
		&client.join("Cargo.toml"),
		&CLIENT_FEATURES,
		&mut report,
	)?;

	if let Some(routes) = routes_dir(root, config) {
		add_routes(root, config, &routes, force, &mut report)?;
	}

	if config.auth.is_none() {
		add_auth_table(root, &mut report)?;
	}

	let granted = native_plugins(root, &mut report)?;
	let wired = std::fs::read_to_string(&entry)?.contains("auth::provide(");

	if !wired {
		report.next.push(format!(
			"In {}, call `leptoskit::query::QueryClient::new().provide()` and then \
			 `auth::provide()` where the app starts, in its reactive owner, and show \
			 `auth::screens().gate(|| home)` in front of the screens that need someone signed in.",
			relative(root, &entry)
		));
		report.next.push(if granted {
			"A native app also calls `leptoskit::auth::client::configure(ClientConfig::new().\
			 origin(\"https://your.site\"))` when it starts, with an origin listed in \
			 `[native.plugin-options.http] origins`; the sign-in guide shows how."
				.to_owned()
		} else {
			"A native app also calls `leptoskit::auth::client::configure(ClientConfig::new().\
			 origin(\"https://your.site\"))` when it starts, and needs the `http` and \
			 `secure-store` plugins; the sign-in guide shows how."
				.to_owned()
		});
	}

	server(root, config, &client, force, &mut report)?;
	Ok(report)
}

/// The server's side: its sign-in module and the `auth` feature, when the
/// project has a server and a database.
fn server(
	root: &Path,
	config: &LeptoskitConfig,
	client: &Path,
	force: bool,
	report: &mut Report,
) -> Result<()> {
	let manifest = root.join("Cargo.toml");
	let main = root.join("src/main.rs");

	if !main.is_file() || !has_feature(&manifest, "leptoskit", "server")? {
		return Ok(());
	}

	if config.db.is_none() {
		report.next.push(
			"Sign-in keeps its accounts in your database, so the server's side wasn't added. Add \
			 a `[db]` table to leptoskit.toml, then run `leptoskit generate auth` again."
				.to_owned(),
		);
		return Ok(());
	}

	// A server that is also the client crate already has `src/auth/`, so the
	// server's module takes another name.
	let module = root.join("src/sign_in.rs");
	write_owned(
		root,
		&module,
		&server_module("../leptoskit.toml"),
		force,
		report,
	)?;
	declare_module(root, &main, "sign_in", report)?;

	if client != root {
		add_features(root, &manifest, &["auth"], report)?;
	}

	if !std::fs::read_to_string(&main)?.contains("sign_in::service") {
		report.next.push(
			"In src/main.rs, build the service once the database is migrated, with your user \
			 table: `let auth = sign_in::service::<User>(&db)?;`. Then add \
			 `.merge(auth.router())` to the router, and `.layer(auth.layer())` last, so it wraps \
			 every route."
				.to_owned(),
		);
		report.next.push(
			"In the crate that declares your tables, add \
			 `leptoskit::db::packs!(leptoskit_auth);`, key your user table by \
			 `Id<auth::Account>`, implement `auth::OnSignUp` for it, and run `leptoskit db \
			 migration create auth`."
				.to_owned(),
		);
		report.next.push(
			"Set AUTH_ENCRYPTION_KEYS before starting the server: `export \
			 AUTH_ENCRYPTION_KEYS=\"$(date +%Y-%m):$(openssl rand 32 | basenc --base64url | tr -d \
			 =)\"`."
				.to_owned(),
		);
	}

	Ok(())
}

/// The directory of the crate the app's interface lives in:
/// `build.client_package`, or the root package.
fn client_dir(root: &Path, config: &LeptoskitConfig) -> Result<PathBuf> {
	let root_package = package_name(&root.join("Cargo.toml"))?;
	let Some(package) = config.build.client_package.clone().or(root_package.clone()) else {
		anyhow::bail!(
			"there's no package at {}; set `build.client_package` in {}",
			root.display(),
			leptoskit_config::CONFIG_FILE
		);
	};

	if root_package.as_deref() == Some(package.as_str()) {
		return Ok(root.to_path_buf());
	}

	for member in workspace_members(&root.join("Cargo.toml"))?
		.into_iter()
		.chain([package.clone()])
	{
		let dir = root.join(member);

		if package_name(&dir.join("Cargo.toml"))?.as_deref() == Some(package.as_str()) {
			return Ok(dir);
		}
	}

	anyhow::bail!(
		"couldn't find the client package `{package}` in the workspace at {}",
		root.display()
	)
}

/// The routes directory, when the app uses the file router.
fn routes_dir(root: &Path, config: &LeptoskitConfig) -> Option<PathBuf> {
	let spec = crate::build::router_spec(config)?;
	let dir = spec
		.options
		.get("dir")
		.and_then(toml::Value::as_str)
		.map_or_else(
			|| leptoskit_config::RouterConfig::default().dir,
			PathBuf::from,
		);
	Some(root.join(dir))
}

/// Adds `/sign-in`, `/devices`, and `/security` to the file router, unless the
/// app already has routes by those names, then brings the generated module up
/// to date.
fn add_routes(
	root: &Path,
	config: &LeptoskitConfig,
	routes: &Path,
	force: bool,
	report: &mut Report,
) -> Result<()> {
	let mut wrote = false;

	for (file, contents) in ROUTES {
		let target = routes.join(file);
		let existing = walkdir::WalkDir::new(routes)
			.into_iter()
			.filter_map(Result::ok)
			.find(|entry| entry.file_name() == file && entry.path() != target);

		if let Some(existing) = existing {
			report.kept.push(format!(
				"kept {}: the app already has that route",
				relative(root, existing.path())
			));
			continue;
		}

		wrote |= write_owned(root, &target, contents, force, report)?;
	}

	if wrote && let Err(error) = crate::routes::sync(root, config) {
		report.next.push(format!(
			"Regenerating the routes module failed ({error:#}); fix that, then run `leptoskit \
			 plugin:router routes`."
		));
	}

	Ok(())
}

/// Gives the native apps the plugins the sign-in client needs: each host
/// that registers plugins (iOS, Android, and macOS) depends on them, and
/// `leptoskit.toml` grants them. Returns whether the app has such a host.
fn native_plugins(root: &Path, report: &mut Report) -> Result<bool> {
	let hosts: Vec<PathBuf> = crate::new_project::PLUGIN_PLATFORMS
		.into_iter()
		.map(|platform| root.join(platform).join("Cargo.toml"))
		.filter(|manifest| manifest.is_file())
		.collect();

	if hosts.is_empty() {
		return Ok(false);
	}

	for manifest in &hosts {
		add_plugin_dependencies(root, manifest, report)?;
	}

	grant_plugins(root, report)?;
	Ok(true)
}

/// Adds the sign-in plugins next to a host's `leptoskit_bridge_ffi`, at the
/// same version, and from the same checkout when it uses a path.
fn add_plugin_dependencies(root: &Path, manifest: &Path, report: &mut Report) -> Result<()> {
	let shown = relative(root, manifest);
	let contents =
		std::fs::read_to_string(manifest).with_context(|| format!("could not read {shown}"))?;
	let mut document = contents
		.parse::<toml_edit::DocumentMut>()
		.with_context(|| format!("could not parse {shown}"))?;
	let Some(dependencies) = bridge_dependencies(&mut document) else {
		report.next.push(format!(
			"{shown} doesn't depend on leptoskit_bridge_ffi; add leptoskit_plugin_http and \
			 leptoskit_plugin_secure_store to the dependencies of the crate that does."
		));
		return Ok(());
	};
	let bridge = dependencies
		.get("leptoskit_bridge_ffi")
		.and_then(toml_edit::Item::as_table_like);
	let version = bridge
		.and_then(|bridge| bridge.get("version"))
		.and_then(toml_edit::Item::as_str)
		.or_else(|| {
			dependencies
				.get("leptoskit_bridge_ffi")
				.and_then(toml_edit::Item::as_str)
		})
		.map(str::to_owned);
	let checkout = bridge
		.and_then(|bridge| bridge.get("path"))
		.and_then(toml_edit::Item::as_str)
		.map(str::to_owned);
	let mut added = Vec::new();

	for (_, package) in crate::new_project::AUTH_PLUGINS {
		if dependencies.contains_key(package) {
			continue;
		}

		let mut dependency = toml_edit::InlineTable::new();

		if let Some(version) = &version {
			dependency.insert("version", version.as_str().into());
		}

		dependency.insert("default-features", false.into());

		if let Some(checkout) = &checkout {
			dependency.insert("path", sibling(checkout, package).into());
		}

		dependencies.insert(package, toml_edit::value(dependency));
		added.push(package);
	}

	if added.is_empty() {
		return Ok(());
	}

	std::fs::write(manifest, document.to_string())
		.with_context(|| format!("could not write {shown}"))?;
	report
		.changed
		.push(format!("updated {shown}: added {}", added.join(" and ")));
	Ok(())
}

/// The path of a crate next to `leptoskit_bridge_ffi`, keeping the
/// separator style the manifest already uses, so a Windows checkout
/// doesn't get `/sdk/crates\leptoskit_plugin_http`.
fn sibling(path: &str, package: &str) -> String {
	match path.rfind(['/', '\\']) {
		Some(end) => format!("{}{package}", &path[..=end]),
		None => package.to_owned(),
	}
}

/// The dependency table that holds `leptoskit_bridge_ffi`: `[dependencies]`,
/// or a `[target.'cfg(…)'.dependencies]`.
fn bridge_dependencies(
	document: &mut toml_edit::DocumentMut,
) -> Option<&mut dyn toml_edit::TableLike> {
	let has_bridge = |item: &toml_edit::Item| {
		item.as_table_like()
			.is_some_and(|table| table.contains_key("leptoskit_bridge_ffi"))
	};

	if document.get("dependencies").is_some_and(has_bridge) {
		return document.get_mut("dependencies")?.as_table_like_mut();
	}

	let targets = document.get_mut("target")?.as_table_like_mut()?;
	let key = targets
		.iter()
		.find(|(_, target)| target.get("dependencies").is_some_and(has_bridge))
		.map(|(key, _)| key.to_owned())?;
	targets
		.get_mut(&key)?
		.get_mut("dependencies")?
		.as_table_like_mut()
}

/// Grants the sign-in plugins in `leptoskit.toml`, and lets the HTTP plugin
/// reach the development server unless the app already chose its origins.
fn grant_plugins(root: &Path, report: &mut Report) -> Result<()> {
	let path = root.join(leptoskit_config::CONFIG_FILE);
	let contents = std::fs::read_to_string(&path)
		.with_context(|| format!("could not read {}", path.display()))?;
	let mut document = contents
		.parse::<toml_edit::DocumentMut>()
		.with_context(|| format!("could not parse {}", path.display()))?;
	let native = document
		.entry("native")
		.or_insert_with(toml_edit::table)
		.as_table_like_mut()
		.context("[native] in leptoskit.toml isn't a table")?;
	let plugins = native
		.entry("plugins")
		.or_insert_with(|| toml_edit::value(toml_edit::Array::new()))
		.as_array_mut()
		.context("`plugins` in [native] isn't a list")?;
	let mut changes = Vec::new();

	for (name, _) in crate::new_project::AUTH_PLUGINS {
		if !plugins.iter().any(|plugin| plugin.as_str() == Some(name)) {
			plugins.push(name);
			changes.push(format!("granted {name}"));
		}
	}

	let options = native.entry("plugin-options").or_insert_with(|| {
		let mut table = toml_edit::Table::new();
		table.set_implicit(true);
		toml_edit::Item::Table(table)
	});
	let options = options
		.as_table_like_mut()
		.context("`plugin-options` in [native] isn't a table")?;

	if !options.contains_key("http") {
		let mut http = toml_edit::Table::new();
		http.insert(
			"origins",
			toml_edit::value(toml_edit::Array::from_iter(
				crate::new_project::AUTH_DEV_ORIGINS,
			)),
		);
		options.insert("http", toml_edit::Item::Table(http));
		changes.push("let the HTTP plugin reach the development server".to_owned());
	}

	if changes.is_empty() {
		return Ok(());
	}

	let contents = document.to_string();
	LeptoskitConfig::from_toml(&contents, root).with_context(|| {
		format!(
			"granting the sign-in plugins would break {}",
			path.display()
		)
	})?;
	std::fs::write(&path, contents)
		.with_context(|| format!("could not write {}", path.display()))?;
	report.changed.push(format!(
		"updated {}: {}",
		leptoskit_config::CONFIG_FILE,
		changes.join(", ")
	));
	Ok(())
}

/// Appends the `[auth]` table to `leptoskit.toml`, checking that the file
/// still loads.
fn add_auth_table(root: &Path, report: &mut Report) -> Result<()> {
	let path = root.join(leptoskit_config::CONFIG_FILE);
	let mut contents = std::fs::read_to_string(&path)
		.with_context(|| format!("could not read {}", path.display()))?;

	if !contents.ends_with('\n') {
		contents.push('\n');
	}

	contents.push_str(AUTH_TABLE);
	LeptoskitConfig::from_toml(&contents, root)
		.with_context(|| format!("adding [auth] would break {}", path.display()))?;
	std::fs::write(&path, contents)
		.with_context(|| format!("could not write {}", path.display()))?;
	report.changed.push(format!(
		"updated {}: added [auth]",
		leptoskit_config::CONFIG_FILE
	));
	Ok(())
}

/// Writes a file the app owns once it exists: creates it when missing, and
/// leaves it alone when it matches or, without `force`, when it differs.
/// Returns whether it wrote.
fn write_owned(
	root: &Path,
	path: &Path,
	contents: &str,
	force: bool,
	report: &mut Report,
) -> Result<bool> {
	let shown = relative(root, path);

	match std::fs::symlink_metadata(path) {
		Ok(metadata) => {
			anyhow::ensure!(metadata.is_file(), "{shown} isn't an ordinary file");

			if std::fs::read_to_string(path)? == contents {
				return Ok(false);
			}

			if !force {
				report.kept.push(format!(
					"kept {shown}: it differs from the template; --force replaces it"
				));
				return Ok(false);
			}

			std::fs::write(path, contents).with_context(|| format!("could not write {shown}"))?;
			report.changed.push(format!("replaced {shown}"));
		}
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
			if let Some(parent) = path.parent() {
				std::fs::create_dir_all(parent)
					.with_context(|| format!("could not create {}", relative(root, parent)))?;
			}

			std::fs::write(path, contents).with_context(|| format!("could not write {shown}"))?;
			report.changed.push(format!("created {shown}"));
		}
		Err(error) => return Err(error).with_context(|| format!("could not inspect {shown}")),
	}

	Ok(true)
}

/// Appends `pub mod <name>;` to a crate root that doesn't declare the module.
fn declare_module(root: &Path, entry: &Path, name: &str, report: &mut Report) -> Result<()> {
	let mut contents = std::fs::read_to_string(entry)?;
	let declared = contents.lines().any(|line| {
		let line = line.trim();
		[
			format!("mod {name};"),
			format!("pub mod {name};"),
			format!("pub(crate) mod {name};"),
		]
		.contains(&line.to_owned())
	});

	if declared {
		return Ok(());
	}

	if !contents.ends_with('\n') {
		contents.push('\n');
	}

	let visibility = if name == "auth" { "pub " } else { "" };
	writeln!(
		contents,
		"\n/// Written by `leptoskit generate auth`.\n{visibility}mod {name};"
	)?;
	std::fs::write(entry, contents)?;
	report.changed.push(format!(
		"updated {}: declared `{visibility}mod {name};`",
		relative(root, entry)
	));
	Ok(())
}

/// Adds `features` to the `dependency` in a manifest, keeping its formatting.
fn add_features(
	root: &Path,
	manifest: &Path,
	features: &[&str],
	report: &mut Report,
) -> Result<()> {
	let shown = relative(root, manifest);
	let contents =
		std::fs::read_to_string(manifest).with_context(|| format!("could not read {shown}"))?;
	let mut document = contents
		.parse::<toml_edit::DocumentMut>()
		.with_context(|| format!("could not parse {shown}"))?;
	let added = add_dependency_features(&mut document, "leptoskit", features)
		.with_context(|| format!("{shown} doesn't depend on leptoskit"))?;

	if added.is_empty() {
		return Ok(());
	}

	std::fs::write(manifest, document.to_string())
		.with_context(|| format!("could not write {shown}"))?;
	report.changed.push(format!(
		"updated {shown}: added the leptoskit feature{} {}",
		if added.len() == 1 { "" } else { "s" },
		added.join(", ")
	));
	Ok(())
}

/// Adds the missing `features` to `name` under `[dependencies]`, turning a
/// bare version into a table. Returns the features it added, or `None`
/// without the dependency.
fn add_dependency_features(
	document: &mut toml_edit::DocumentMut,
	name: &str,
	features: &[&str],
) -> Option<Vec<String>> {
	let dependency = document.get_mut("dependencies")?.get_mut(name)?;

	if let Some(version) = dependency.as_str().map(str::to_owned) {
		let mut table = toml_edit::InlineTable::new();
		table.insert("version", version.into());
		*dependency = toml_edit::value(table);
	}

	let table = dependency.as_table_like_mut()?;
	let inserted = !table.contains_key("features");
	let list = table
		.entry("features")
		.or_insert_with(|| toml_edit::value(toml_edit::Array::new()))
		.as_array_mut()?;
	let mut added = Vec::new();

	for feature in features {
		if !list
			.iter()
			.any(|existing| existing.as_str() == Some(feature))
		{
			list.push(*feature);
			added.push((*feature).to_owned());
		}
	}

	// A key added to `{ … }` keeps the space that closed the table before
	// it, so lay the table out again.
	if inserted && let Some(inline) = dependency.as_inline_table_mut() {
		inline.fmt();
	}

	Some(added)
}

/// Whether `dependency` in a manifest turns on `feature`.
fn has_feature(manifest: &Path, dependency: &str, feature: &str) -> Result<bool> {
	let Ok(contents) = std::fs::read_to_string(manifest) else {
		return Ok(false);
	};
	let document: toml::Table = toml::from_str(&contents)
		.with_context(|| format!("could not parse {}", manifest.display()))?;
	Ok(document
		.get("dependencies")
		.and_then(|dependencies| dependencies.get(dependency))
		.and_then(|dependency| dependency.get("features"))
		.and_then(toml::Value::as_array)
		.is_some_and(|features| features.iter().any(|value| value.as_str() == Some(feature))))
}

/// The `[package]` name in a manifest, if it has one.
fn package_name(manifest: &Path) -> Result<Option<String>> {
	let Ok(contents) = std::fs::read_to_string(manifest) else {
		return Ok(None);
	};
	let document: toml::Table = toml::from_str(&contents)
		.with_context(|| format!("could not parse {}", manifest.display()))?;
	Ok(document
		.get("package")
		.and_then(|package| package.get("name"))
		.and_then(toml::Value::as_str)
		.map(str::to_owned))
}

/// The workspace's member paths, as written.
fn workspace_members(manifest: &Path) -> Result<Vec<String>> {
	let Ok(contents) = std::fs::read_to_string(manifest) else {
		return Ok(Vec::new());
	};
	let document: toml::Table = toml::from_str(&contents)
		.with_context(|| format!("could not parse {}", manifest.display()))?;
	Ok(document
		.get("workspace")
		.and_then(|workspace| workspace.get("members"))
		.and_then(toml::Value::as_array)
		.map(|members| {
			members
				.iter()
				.filter_map(toml::Value::as_str)
				.map(str::to_owned)
				.collect()
		})
		.unwrap_or_default())
}

/// `path` relative to the project root, with `/` separators.
fn relative(root: &Path, path: &Path) -> String {
	path.strip_prefix(root)
		.unwrap_or(path)
		.to_string_lossy()
		.replace(std::path::MAIN_SEPARATOR, "/")
}

#[cfg(test)]
mod tests;
